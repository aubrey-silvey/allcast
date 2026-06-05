mod telemetry;

use anyhow::{Result, anyhow};
use clap::{Parser, ValueEnum};
use gstreamer as gst;
use gstreamer::prelude::*;
use socket2::{Domain, Socket, Type};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use telemetry::SharedStats;
use tracing::{error, info, warn};

#[derive(Parser, Debug, Clone)]
#[command(version, about = "allcast receiver \u{2014} idle daemon that takes over a configured monitor when an RTP stream arrives.")]
struct Cli {
    /// UDP port to listen on for RTP packets.
    #[arg(long, env = "LISTEN", default_value_t = 5004)]
    listen: u16,

    #[arg(long, env = "CODEC", default_value = "h265")]
    codec: Codec,

    /// Must match the sender's PT (no SDP to negotiate).
    #[arg(long, env = "PT", default_value_t = 96)]
    pt: u8,

    /// Jitter buffer depth in ms. LAN-appropriate value is ~10–20.
    #[arg(long, env = "JITTER_MS", default_value_t = 10)]
    jitter_ms: u32,

    /// GStreamer sink element + properties. `sync=false` is important for
    /// low-latency live streams — without it the sink waits for the pipeline
    /// clock before rendering, costing up to one frame per stage.
    #[arg(long, env = "SINK", default_value = "waylandsink fullscreen=true sync=false")]
    sink: String,

    /// Wayland output name to render on (e.g. `HDMI-A-1`). Currently
    /// informational — the compositor decides placement, but the value is
    /// validated at startup against the live output list when possible.
    #[arg(long, env = "MONITOR")]
    monitor: Option<String>,

    /// Socket SO_RCVBUF in bytes. Capped by the host’s net.core.rmem_max
    /// (raise that on the Pi to allow large values).
    #[arg(long, env = "RCVBUF", default_value_t = 8 * 1024 * 1024)]
    rcvbuf: u32,

    /// Software-decoder worker threads (avdec only). 0 = auto (one per core).
    /// Decoding uses *slice* threading, which spreads a single frame across
    /// cores WITHOUT the frame-buffering latency that frame-threading adds —
    /// so it cuts per-core CPU when the sender emits a multi-slice bitstream
    /// (see the sender's `--slices`) and is a harmless no-op otherwise.
    #[arg(long, env = "DECODE_THREADS", default_value_t = 0)]
    decode_threads: u32,

    /// Seconds of RTP silence before deactivating and releasing the monitor.
    #[arg(long, env = "IDLE_TIMEOUT_S", default_value_t = 3)]
    idle_timeout_s: u32,

    /// One-shot mode: build pipeline immediately, run until EOS / error / Ctrl-C,
    /// then exit. Default is the daemon loop that idles between streams.
    #[arg(long, env = "ONCE", default_value_t = false)]
    once: bool,

    /// TCP port for the gRPC telemetry server (receiver→sender health stream).
    /// 0 disables it.
    #[arg(long, env = "TELEMETRY_PORT", default_value_t = allcast_telemetry::DEFAULT_PORT)]
    telemetry_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Codec {
    H264,
    H265,
}

#[derive(Debug, Clone)]
struct Decoder {
    gst_element: &'static str,
    hardware: bool,
}

// NB: Pi 5's `v4l2slh265dec` emits a tiled NV12_128C8 DMABuf that current
// waylandsink/glimagesink can't import. Software `avdec_h265` is listed
// first so it wins on the Pi until a Pi-specific detiler ships.
const H264_CANDIDATES: &[(&str, bool)] = &[
    ("avdec_h264", false),
    ("vah264dec", true),
    ("vaapih264dec", true),
    ("nvh264dec", true),
    ("nvcudah264dec", true),
    ("qsvh264dec", true),
    ("vulkanh264dec", true),
    ("v4l2h264dec", true),
];

const H265_CANDIDATES: &[(&str, bool)] = &[
    ("avdec_h265", false),
    ("vah265dec", true),
    ("vaapih265dec", true),
    ("nvh265dec", true),
    ("nvcudah265dec", true),
    ("qsvh265dec", true),
    ("vulkanh265dec", true),
    ("v4l2slh265dec", true),
];

fn pick_decoder(codec: Codec) -> Option<Decoder> {
    let registry = gst::Registry::get();
    let table = match codec {
        Codec::H264 => H264_CANDIDATES,
        Codec::H265 => H265_CANDIDATES,
    };
    table.iter().find_map(|(name, hw)| {
        registry
            .find_feature(name, gst::ElementFactory::static_type())
            .map(|_| Decoder {
                gst_element: name,
                hardware: *hw,
            })
    })
}

/// Build the decode/render pipeline. When `socket` is `Some`, the daemon has
/// already bound the port and detected (peeked, not consumed) the first packet;
/// we hand that exact socket to `udpsrc` so the opening IDR keyframe sitting in
/// the kernel receive buffer is decoded immediately — no rebind, no lost
/// packets, no waiting up to a full GOP for the next keyframe. When `None`
/// (one-shot mode) `udpsrc` binds the port itself.
fn build_pipeline(
    cli: &Cli,
    decoder: &Decoder,
    socket: Option<gio::Socket>,
    stats: &Arc<SharedStats>,
) -> Result<(gst::Pipeline, Arc<AtomicI64>)> {
    let (encoding_name, depay, parse) = match cli.codec {
        Codec::H264 => ("H264", "rtph264depay", "h264parse"),
        Codec::H265 => ("H265", "rtph265depay", "h265parse"),
    };
    // Low-latency tweaks layered on top of the basic pipeline:
    //   * `rtpjitterbuffer mode=1` = "buffer" mode, no clock-slaving overhead.
    //   * `avdec_h26x thread-type=slice` parallelises decode of a single frame
    //     across cores — unlike frame threading (the old `thread-type=1`) it
    //     does NOT buffer 2-3 frames, so latency is unchanged. It only cuts
    //     per-core CPU when the bitstream has multiple slices/tiles; otherwise
    //     it runs single-threaded with no penalty.
    let dec_args = match decoder.gst_element {
        "avdec_h265" | "avdec_h264" => {
            format!("max-threads={n} thread-type=slice", n = cli.decode_threads)
        }
        _ => String::new(),
    };
    // With a handed-in socket, `udpsrc` neither binds the port nor touches
    // SO_RCVBUF (the daemon already set both) — so leave those off the launch
    // line and attach the socket below. Without one, it binds itself.
    let udpsrc_props = if socket.is_some() {
        String::new()
    } else {
        format!("port={port} buffer-size={rcvbuf}", port = cli.listen, rcvbuf = cli.rcvbuf)
    };
    let desc = format!(
        "udpsrc name=netin {udpsrc_props} \
         caps=application/x-rtp,media=video,clock-rate=90000,encoding-name={enc_name},payload={pt} \
         ! rtpjitterbuffer latency={jitter} mode=1 \
         ! {depay} ! {parse} ! {decoder} {dec_args} ! videoconvert ! {sink}",
        enc_name = encoding_name,
        pt = cli.pt,
        jitter = cli.jitter_ms,
        depay = depay,
        parse = parse,
        decoder = decoder.gst_element,
        sink = cli.sink,
    );
    info!(%desc, "building pipeline");
    let element = gst::parse::launch(&desc)
        .map_err(|e| anyhow!("pipeline parse failed: {e}\n  desc=`{desc}`"))?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("parsed element is not a gst::Pipeline"))?;

    if let Some(sock) = socket {
        let netin = pipeline
            .by_name("netin")
            .ok_or_else(|| anyhow!("udpsrc element named `netin` missing"))?;
        netin.set_property("socket", &sock);
        // Let udpsrc close the fd when the pipeline tears down to idle, so the
        // next activation binds a fresh socket cleanly.
        netin.set_property("close-socket", true);
    }

    // Pad probe on the udpsrc src pad — every buffer (= one RTP packet) updates
    // the last-packet timestamp. The daemon loop polls this to decide when to
    // deactivate due to silence.
    let last_packet_ns = Arc::new(AtomicI64::new(0));
    let netin = pipeline
        .by_name("netin")
        .ok_or_else(|| anyhow!("udpsrc element named `netin` missing"))?;
    let pad = netin
        .static_pad("src")
        .ok_or_else(|| anyhow!("udpsrc has no `src` pad"))?;
    let last_clone = last_packet_ns.clone();
    let stats_clone = stats.clone();
    pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        last_clone.store(now_ns, Ordering::Relaxed);
        stats_clone.packets.fetch_add(1, Ordering::Relaxed);
        gst::PadProbeReturn::Ok
    });

    Ok((pipeline, last_packet_ns))
}

/// Bind UDP `port` and block until the first packet arrives or `stop` is set.
/// Returns the bound socket (wrapped for GStreamer) rather than dropping it:
/// the first packet is detected with `peek` (MSG_PEEK), so it stays in the
/// kernel buffer for `udpsrc` to read. Handing this same socket to the pipeline
/// — instead of dropping it and binding a fresh one — means the opening IDR
/// keyframe isn't lost in the gap, so the picture appears within a frame
/// instead of after the next keyframe (up to a full GOP later). Returns `None`
/// only when `stop` fires first. `rcvbuf` matches the active pipeline so the
/// packets that accumulate before udpsrc starts aren't dropped.
fn wait_for_traffic(port: u16, rcvbuf: u32, stop: &AtomicBool) -> Result<Option<gio::Socket>> {
    info!(port, "idle — waiting for first RTP packet");
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, None)
        .map_err(|e| anyhow!("create probe socket: {e}"))?;
    sock.set_reuse_address(true).ok();
    if let Err(e) = sock.set_recv_buffer_size(rcvbuf as usize) {
        warn!(error = %e, rcvbuf, "could not set SO_RCVBUF on probe socket");
    }
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    sock.bind(&addr.into())
        .map_err(|e| anyhow!("bind probe socket on port {port}: {e}"))?;

    // std UdpSocket for peek_from (MSG_PEEK) — detect without consuming.
    let probe: UdpSocket = sock.into();
    probe.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut buf = [0u8; 2048];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match probe.peek_from(&mut buf) {
            Ok((n, src)) => {
                info!(bytes = n, %src, "first packet — handing socket to pipeline");
                // Clear the probe-loop timeout/flags before udpsrc takes over.
                probe.set_read_timeout(None)?;
                probe.set_nonblocking(false)?;
                let owned: OwnedFd = probe.into();
                let gsock = unsafe { gio::Socket::from_fd(owned) }
                    .map_err(|e| anyhow!("wrap probe socket for gstreamer: {e}"))?;
                return Ok(Some(gsock));
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) => return Err(anyhow!("probe peek: {e}")),
        }
    }
}

/// Build + run the pipeline until either the GStreamer bus reports an error/EOS,
/// `idle_timeout_s` of RTP silence have elapsed, or `stop` is set.
fn run_active(
    cli: &Cli,
    decoder: &Decoder,
    socket: gio::Socket,
    stats: &Arc<SharedStats>,
    stop: &AtomicBool,
) -> Result<()> {
    let (pipeline, last_packet_ns) = build_pipeline(cli, decoder, Some(socket), stats)?;
    let bus = pipeline
        .bus()
        .ok_or_else(|| anyhow!("pipeline has no bus"))?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| anyhow!("set_state(Playing) failed: {e}"))?;
    info!("pipeline active — fullscreen on configured monitor");
    stats.active.store(true, Ordering::Relaxed);

    let idle_timeout = Duration::from_secs(cli.idle_timeout_s as u64);
    let mut last_log_silence = SystemTime::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            info!("shutdown signal");
            break;
        }
        if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
            match msg.view() {
                gst::MessageView::Eos(_) => {
                    info!("end of stream");
                    break;
                }
                gst::MessageView::Error(e) => {
                    error!(
                        src = ?e.src().map(|s| s.path_string()),
                        error = %e.error(),
                        debug = ?e.debug(),
                        "pipeline error — deactivating"
                    );
                    break;
                }
                gst::MessageView::Warning(w) => {
                    warn!(
                        src = ?w.src().map(|s| s.path_string()),
                        warning = %w.error(),
                        "pipeline warning"
                    );
                }
                _ => {}
            }
        }

        // Silence detection. Probe captures every udpsrc buffer; if the most
        // recent one is older than idle_timeout, transition back to idle.
        let last_ns = last_packet_ns.load(Ordering::Relaxed);
        if last_ns > 0 {
            let last = SystemTime::UNIX_EPOCH + Duration::from_nanos(last_ns as u64);
            if let Ok(age) = SystemTime::now().duration_since(last) {
                if age > idle_timeout {
                    info!(?idle_timeout, "no RTP packets — deactivating");
                    break;
                }
                // Heartbeat log every 30 s while active.
                if SystemTime::now()
                    .duration_since(last_log_silence)
                    .map(|d| d > Duration::from_secs(30))
                    .unwrap_or(false)
                {
                    last_log_silence = SystemTime::now();
                    info!(silence_ms = age.as_millis() as u64, "alive");
                }
            }
        }
    }

    stats.active.store(false, Ordering::Relaxed);
    pipeline
        .set_state(gst::State::Null)
        .map_err(|e| anyhow!("set_state(Null) failed: {e}"))?;
    info!("pipeline deactivated — monitor released");
    Ok(())
}

fn run_daemon(
    cli: &Cli,
    decoder: &Decoder,
    stats: &Arc<SharedStats>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    while !stop.load(Ordering::Relaxed) {
        let socket = match wait_for_traffic(cli.listen, cli.rcvbuf, &stop)? {
            Some(s) => s,
            None => break,
        };
        if let Err(e) = run_active(cli, decoder, socket, stats, &stop) {
            error!(error = %e, "active session ended with error — returning to idle");
            // Brief backoff so a hard-failing pipeline doesn't spin.
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    Ok(())
}

fn run_once(
    cli: &Cli,
    decoder: &Decoder,
    stats: &Arc<SharedStats>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    stats.active.store(true, Ordering::Relaxed);
    let (pipeline, _last) = build_pipeline(cli, decoder, None, stats)?;
    let bus = pipeline
        .bus()
        .ok_or_else(|| anyhow!("pipeline has no bus"))?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| anyhow!("set_state(Playing) failed: {e}"))?;
    info!("one-shot mode — Ctrl-C to stop");
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
            match msg.view() {
                gst::MessageView::Eos(_) => break,
                gst::MessageView::Error(e) => {
                    error!(error = %e.error(), "pipeline error");
                    return Err(anyhow!("{}", e.error()));
                }
                _ => {}
            }
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    gst::init()?;

    let cli = Cli::parse();
    let decoder = pick_decoder(cli.codec)
        .ok_or_else(|| anyhow!("no decoder for codec {:?} in the GStreamer registry", cli.codec))?;
    info!(
        codec = ?cli.codec,
        decoder = decoder.gst_element,
        hardware = decoder.hardware,
        listen = cli.listen,
        sink = %cli.sink,
        monitor = ?cli.monitor,
        idle_timeout_s = cli.idle_timeout_s,
        mode = if cli.once { "once" } else { "daemon" },
        "starting"
    );

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    ctrlc::set_handler(move || stop_clone.store(true, Ordering::Relaxed)).ok();

    let stats = SharedStats::new(cli.listen);
    if cli.telemetry_port != 0 {
        telemetry::serve(stats.clone(), cli.telemetry_port);
    }

    if cli.once {
        run_once(&cli, &decoder, &stats, stop)
    } else {
        run_daemon(&cli, &decoder, &stats, stop)
    }
}
