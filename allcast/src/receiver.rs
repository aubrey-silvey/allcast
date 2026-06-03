//! Receiver role: idle UDP-probe daemon that activates a decode+display
//! pipeline when RTP traffic arrives, and tears it down after silence.

use anyhow::{Result, anyhow};
use gstreamer as gst;
use gstreamer::prelude::*;
use std::io::ErrorKind;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

use crate::config::{Codec, Config};
use crate::platform::{self, Os};

pub fn run(cfg: &Config, stop: Arc<AtomicBool>) -> Result<()> {
    gst::init()?;

    let os = platform::current_os();
    let candidates = platform::decoder_candidates(os, cfg.codec);
    let (decoder_name, hardware) = platform::pick_element(candidates)
        .ok_or_else(|| anyhow!("no decoder for {:?} on {:?}", cfg.codec, os))?;
    info!(
        codec = ?cfg.codec,
        decoder = decoder_name,
        hardware,
        listen = cfg.listen_port,
        os = ?os,
        "selected decoder"
    );

    let sink = platform::default_sink(os);
    while !stop.load(Ordering::Relaxed) {
        if !wait_for_traffic(cfg.listen_port, &stop)? {
            break;
        }
        if let Err(e) = run_active(cfg, decoder_name, sink, &stop) {
            error!(error = %e, "active session ended with error — returning to idle");
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    Ok(())
}

fn wait_for_traffic(port: u16, stop: &AtomicBool) -> Result<bool> {
    info!(port, "idle — waiting for first RTP packet");
    let sock = UdpSocket::bind(("0.0.0.0", port))
        .map_err(|e| anyhow!("could not bind probe socket on port {port}: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut buf = [0u8; 2048];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                info!(bytes = n, %src, "first packet — activating");
                drop(sock);
                return Ok(true);
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) => return Err(anyhow!("probe recv: {e}")),
        }
    }
}

fn run_active(cfg: &Config, decoder_name: &str, sink: &str, stop: &AtomicBool) -> Result<()> {
    let (pipeline, last_packet_ns) = build_pipeline(cfg, decoder_name, sink)?;
    let bus = pipeline.bus().ok_or_else(|| anyhow!("no bus"))?;
    pipeline.set_state(gst::State::Playing)
        .map_err(|e| anyhow!("set_state(Playing) failed: {e}"))?;
    info!("receiver pipeline active — fullscreen on configured monitor");

    let idle_timeout = Duration::from_secs(cfg.idle_timeout_s as u64);
    let mut last_heartbeat = SystemTime::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            info!("shutdown signal");
            break;
        }
        if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
            match msg.view() {
                gst::MessageView::Eos(_) => { info!("eos"); break; }
                gst::MessageView::Error(e) => {
                    error!(error = %e.error(), debug = ?e.debug(), "pipeline error");
                    break;
                }
                gst::MessageView::Warning(w) => warn!(warning = %w.error(), "pipeline warning"),
                _ => {}
            }
        }

        let last_ns = last_packet_ns.load(Ordering::Relaxed);
        if last_ns > 0 {
            let last = SystemTime::UNIX_EPOCH + Duration::from_nanos(last_ns as u64);
            if let Ok(age) = SystemTime::now().duration_since(last) {
                if age > idle_timeout {
                    info!(?idle_timeout, "no RTP packets — deactivating");
                    break;
                }
                if SystemTime::now()
                    .duration_since(last_heartbeat)
                    .map(|d| d > Duration::from_secs(30))
                    .unwrap_or(false)
                {
                    last_heartbeat = SystemTime::now();
                    info!(silence_ms = age.as_millis() as u64, "alive");
                }
            }
        }
    }

    pipeline.set_state(gst::State::Null)
        .map_err(|e| anyhow!("set_state(Null) failed: {e}"))?;
    info!("receiver pipeline deactivated — monitor released");
    Ok(())
}

fn build_pipeline(
    cfg: &Config,
    decoder_name: &str,
    sink: &str,
) -> Result<(gst::Pipeline, Arc<AtomicI64>)> {
    let (encoding_name, depay, parse) = match cfg.codec {
        Codec::H264 => ("H264", "rtph264depay", "h264parse"),
        Codec::H265 => ("H265", "rtph265depay", "h265parse"),
    };
    let dec_args = match decoder_name {
        "avdec_h265" | "avdec_h264" => "max-threads=1 thread-type=1",
        _ => "",
    };
    let desc = format!(
        "udpsrc name=netin port={port} buffer-size={rcvbuf} \
         caps=application/x-rtp,media=video,clock-rate=90000,encoding-name={enc},payload={pt} \
         ! rtpjitterbuffer latency={jitter} mode=1 \
         ! {depay} ! {parse} ! {decoder_name} {dec_args} ! videoconvert ! {sink}",
        port = cfg.listen_port,
        rcvbuf = cfg.rcvbuf,
        enc = encoding_name,
        pt = cfg.payload_type,
        jitter = cfg.jitter_ms,
    );
    info!(%desc, "building receiver pipeline");
    let element = gst::parse::launch(&desc)
        .map_err(|e| anyhow!("pipeline parse failed: {e}\n  desc=`{desc}`"))?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("not a gst::Pipeline"))?;

    let last_packet_ns = Arc::new(AtomicI64::new(0));
    let netin = pipeline
        .by_name("netin")
        .ok_or_else(|| anyhow!("udpsrc element named `netin` missing"))?;
    let pad = netin
        .static_pad("src")
        .ok_or_else(|| anyhow!("udpsrc has no `src` pad"))?;
    let last_clone = last_packet_ns.clone();
    pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        last_clone.store(now_ns, Ordering::Relaxed);
        gst::PadProbeReturn::Ok
    });
    Ok((pipeline, last_packet_ns))
}

#[allow(dead_code)]
fn _ref_os(_: Os) {}
