//! Sender role: GStreamer pipeline that captures the screen, encodes with the
//! best available HW codec, and pushes RTP/UDP to the active peer.

use anyhow::{Result, anyhow};
use gstreamer as gst;
use gstreamer::prelude::*;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info, warn};

use crate::config::{Codec, Config};
use crate::platform::{self, Os};

pub fn run(cfg: &Config, stop: Arc<AtomicBool>) -> Result<()> {
    gst::init()?;

    let peer = cfg
        .active_peer_address()
        .ok_or_else(|| anyhow!("no peers configured; run `share-screen config` to add one"))?;
    let (host, port) = resolve_dest(peer)?;

    let os = platform::current_os();
    let candidates = platform::encoder_candidates(os, cfg.codec);
    let (encoder_name, hardware) = platform::pick_element(candidates)
        .ok_or_else(|| anyhow!("no encoder for {:?} on {:?}", cfg.codec, os))?;
    info!(
        codec = ?cfg.codec,
        encoder = encoder_name,
        hardware,
        peer,
        os = ?os,
        quality = ?cfg.quality,
        "selected encoder"
    );

    let source = platform::default_source(os);
    let pipeline = build_pipeline(cfg, encoder_name, source, &host, port)?;
    run_pipeline(&pipeline, &stop)
}

fn resolve_dest(s: &str) -> Result<(String, u16)> {
    let (host, port) = s
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("peer must be host:port (got `{s}`)"))?;
    let port: u16 = port.parse().map_err(|_| anyhow!("invalid port in `{s}`"))?;
    let _ = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow!("could not resolve `{host}`: {e}"))?;
    Ok((host.to_string(), port))
}

fn encoder_args(
    element: &str,
    key_int_max: u32,
    bitrate_kbps: u32,
    target_usage: u32,
    rate_control: &str,
) -> String {
    let va_rc = match rate_control {
        "cqp" => "rate-control=cqp qpi=22 qpp=22".to_string(),
        _ => format!("rate-control=cbr bitrate={br}", br = bitrate_kbps),
    };
    match element {
        "vah265enc" | "vah264enc" | "vaapih265enc" | "vaapih264enc" => format!(
            "key-int-max={k} b-frames=0 ref-frames=1 target-usage={tu} {va_rc}",
            k = key_int_max,
            tu = target_usage
        ),
        "nvh265enc" | "nvh264enc" | "nvcudah265enc" | "nvcudah264enc" => format!(
            "preset=low-latency-hq zerolatency=true rc-mode=cbr-ld-hq \
             gop-size={k} bframes=0 bitrate={br}",
            k = key_int_max,
            br = bitrate_kbps
        ),
        "vtenc_h265" | "vtenc_h265_hw" | "vtenc_h264" | "vtenc_h264_hw" => format!(
            "realtime=true allow-frame-reordering=false bitrate={br}",
            br = bitrate_kbps
        ),
        "mfh265enc" | "mfh264enc" => format!(
            "low-latency=true bitrate={br}",
            br = bitrate_kbps
        ),
        "qsvh265enc" | "qsvh264enc" => format!(
            "low-latency=true gop-size={k} ref-frames=1 b-frames=0 bitrate={br}",
            k = key_int_max,
            br = bitrate_kbps
        ),
        "amfh265enc" | "amfh264enc" => format!(
            "usage=ultra-low-latency gop-size={k} bitrate={br}",
            k = key_int_max,
            br = bitrate_kbps
        ),
        "x265enc" | "x264enc" => format!(
            "tune=zerolatency speed-preset=veryfast key-int-max={k} bframes=0 bitrate={br}",
            k = key_int_max,
            br = bitrate_kbps
        ),
        _ => format!("key-int-max={k}", k = key_int_max),
    }
}

fn build_pipeline(
    cfg: &Config,
    encoder_name: &str,
    source: &str,
    host: &str,
    port: u16,
) -> Result<gst::Pipeline> {
    let (rtp_pay, parse_caps, parse) = match cfg.codec {
        Codec::H264 => (
            "rtph264pay aggregate-mode=zero-latency",
            "video/x-h264,stream-format=byte-stream,alignment=au",
            "h264parse config-interval=-1",
        ),
        Codec::H265 => (
            "rtph265pay aggregate-mode=zero-latency",
            "video/x-h265,stream-format=byte-stream,alignment=au",
            "h265parse config-interval=-1",
        ),
    };
    let framerate = cfg.quality.framerate();
    let bitrate = cfg.quality.bitrate_kbps();
    let target_usage = cfg.quality.target_usage();
    let rate_control = cfg.quality.rate_control();
    let key_int_max = framerate;
    let enc_args = encoder_args(encoder_name, key_int_max, bitrate, target_usage, rate_control);
    let desc = format!(
        "{source} ! videorate ! videoconvert ! videoscale \
         ! video/x-raw,format=NV12,width={w},height={h},framerate={fr}/1 \
         ! {enc} {enc_args} \
         ! {parse} ! {parse_caps} \
         ! {rtp_pay} pt={pt} mtu=1200 config-interval=-1 \
         ! udpsink host={host} port={port} sync=false async=false",
        enc = encoder_name,
        w = cfg.width,
        h = cfg.height,
        fr = framerate,
        pt = cfg.payload_type,
    );
    info!(%desc, "building sender pipeline");
    let element = gst::parse::launch(&desc)
        .map_err(|e| anyhow!("pipeline parse failed: {e}\n  desc=`{desc}`"))?;
    element
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("not a gst::Pipeline"))
}

fn run_pipeline(pipeline: &gst::Pipeline, stop: &AtomicBool) -> Result<()> {
    let bus = pipeline.bus().ok_or_else(|| anyhow!("no bus"))?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| anyhow!("set_state(Playing) failed: {e}"))?;
    info!("sender pipeline running");
    loop {
        if stop.load(Ordering::Relaxed) {
            info!("shutdown signal");
            break;
        }
        match bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
            None => continue,
            Some(msg) => match msg.view() {
                gst::MessageView::Eos(_) => {
                    info!("eos");
                    break;
                }
                gst::MessageView::Error(e) => {
                    error!(error = %e.error(), debug = ?e.debug(), "pipeline error");
                    return Err(anyhow!("{}", e.error()));
                }
                gst::MessageView::Warning(w) => warn!(warning = %w.error(), "pipeline warning"),
                _ => {}
            },
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    Ok(())
}

#[allow(dead_code)]
fn _ref_os(_: Os) {}
