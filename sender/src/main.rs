use anyhow::{Result, anyhow};
use clap::{Parser, ValueEnum};
use gstreamer as gst;
use gstreamer::prelude::*;
use std::net::ToSocketAddrs;
use tracing::{error, info, warn};

#[derive(Parser, Debug, Clone)]
#[command(version, about = "GPU-encoded screen pusher: RTP/H.26x over UDP to a known receiver.")]
struct Cli {
    /// destination host:port (the receiver). Address must already be reachable;
    /// no negotiation, no discovery, no HTTP — just blast packets.
    #[arg(long, env = "DEST")]
    dest: String,

    #[arg(long, env = "SOURCE", default_value = "ximagesrc use-damage=false")]
    source: String,

    #[arg(long, env = "CODEC", default_value = "h265")]
    codec: Codec,

    /// RTP payload type. Both ends must agree (no SDP to negotiate it for us).
    #[arg(long, env = "PT", default_value_t = 96)]
    pt: u8,

    #[arg(long, env = "FRAMERATE", default_value_t = 30)]
    framerate: u32,

    #[arg(long, env = "WIDTH", default_value_t = 1920)]
    width: u32,

    #[arg(long, env = "HEIGHT", default_value_t = 1080)]
    height: u32,

    /// MTU for RTP packets. 1200 is the WebRTC convention; Ethernet handles 1500.
    #[arg(long, env = "MTU", default_value_t = 1200)]
    mtu: u32,

    /// Target encoder bitrate in kbps. Desktop with text needs more than typical
    /// video: 10000 (10 Mbps) is comfortable for 1080p30; 15000+ for 1080p60.
    #[arg(long, env = "BITRATE_KBPS", default_value_t = 10000)]
    bitrate_kbps: u32,

    /// VA-API target-usage (1..7). Lower = better quality, higher = faster encode.
    /// 1 = best quality, 4 = balanced (default), 7 = fastest. Ignored on non-VA encoders.
    #[arg(long, env = "TARGET_USAGE", default_value_t = 4)]
    target_usage: u32,

    /// Rate control mode: `cbr` (constant bitrate, even for static content) or
    /// `cqp` (constant quality; small frames for static, big for motion).
    /// CQP is usually better for desktop content but `--bitrate-kbps` is then
    /// ignored — use `--qp` instead. Only applies to VA-API encoders here.
    #[arg(long, env = "RATE_CONTROL", default_value = "cbr")]
    rate_control: String,

    /// QP value when --rate-control=cqp. 22 ≈ visually transparent, 18 ≈ archival.
    #[arg(long, env = "QP", default_value_t = 22)]
    qp: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Codec {
    H264,
    H265,
}

#[derive(Debug, Clone)]
struct Encoder {
    gst_element: &'static str,
    hardware: bool,
}

const H264_CANDIDATES: &[(&str, bool)] = &[
    ("nvh264enc", true),
    ("nvcudah264enc", true),
    ("vah264enc", true),
    ("vaapih264enc", true),
    ("qsvh264enc", true),
    ("amfh264enc", true),
    ("vulkanh264enc", true),
    ("x264enc", false),
];

const H265_CANDIDATES: &[(&str, bool)] = &[
    ("nvh265enc", true),
    ("nvcudah265enc", true),
    ("vah265enc", true),
    ("vaapih265enc", true),
    ("qsvh265enc", true),
    ("amfh265enc", true),
    ("x265enc", false),
];

/// Per-encoder properties that disable lookahead and B-frames while keeping
/// quality usable for desktop content. Without these, encoders dwell on small
/// inputs (like a keystroke) for 100s of ms before emitting and pick file-
/// quality presets that crush text/sharp edges.
/// `target-usage=4` on VA-API is the balanced (default) preset — we keep that
/// for quality and rely on `b-frames=0` + `ref-frames=1` for the latency win.
fn low_latency_args(
    element: &str,
    key_int_max: u32,
    bitrate_kbps: u32,
    target_usage: u32,
    rate_control: &str,
    qp: u32,
) -> String {
    let va_rc = match rate_control {
        "cqp" => format!("rate-control=cqp qpi={q} qpp={q}", q = qp),
        _    => format!("rate-control=cbr bitrate={br}", br = bitrate_kbps),
    };
    match element {
        "vah265enc" | "vah264enc" | "vaapih265enc" | "vaapih264enc" => format!(
            "key-int-max={k} b-frames=0 ref-frames=1 target-usage={tu} {va_rc}",
            k = key_int_max, tu = target_usage
        ),
        "nvh265enc" | "nvh264enc" | "nvcudah265enc" | "nvcudah264enc" => format!(
            "preset=low-latency-hq zerolatency=true rc-mode=cbr-ld-hq \
             gop-size={k} bframes=0 bitrate={br}",
            k = key_int_max, br = bitrate_kbps
        ),
        "x265enc" => format!(
            "tune=zerolatency speed-preset=veryfast key-int-max={k} bframes=0 bitrate={br}",
            k = key_int_max, br = bitrate_kbps
        ),
        "x264enc" => format!(
            "tune=zerolatency speed-preset=veryfast key-int-max={k} bframes=0 bitrate={br}",
            k = key_int_max, br = bitrate_kbps
        ),
        "qsvh265enc" | "qsvh264enc" => format!(
            "low-latency=true gop-size={k} ref-frames=1 b-frames=0 bitrate={br}",
            k = key_int_max, br = bitrate_kbps
        ),
        "amfh265enc" | "amfh264enc" => format!(
            "usage=ultra-low-latency gop-size={k} bitrate={br}",
            k = key_int_max, br = bitrate_kbps
        ),
        _ => format!("key-int-max={k}", k = key_int_max),
    }
}

fn pick_encoder(codec: Codec) -> Option<Encoder> {
    let registry = gst::Registry::get();
    let table = match codec {
        Codec::H264 => H264_CANDIDATES,
        Codec::H265 => H265_CANDIDATES,
    };
    table
        .iter()
        .find_map(|(name, hw)| {
            registry
                .find_feature(name, gst::ElementFactory::static_type())
                .map(|_| Encoder {
                    gst_element: name,
                    hardware: *hw,
                })
        })
}

fn resolve_dest(s: &str) -> Result<(String, u16)> {
    let (host, port) = s
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("--dest must be host:port (got `{s}`)"))?;
    let port: u16 = port.parse().map_err(|_| anyhow!("invalid port in `{s}`"))?;
    // Validate that the host part resolves; pass the literal host to udpsink.
    let _ = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow!("could not resolve `{host}`: {e}"))?
        .next()
        .ok_or_else(|| anyhow!("no addresses for `{host}`"))?;
    Ok((host.to_string(), port))
}

fn build_pipeline(cli: &Cli, encoder: &Encoder) -> Result<gst::Pipeline> {
    let (rtp_pay, parse_caps, encoding) = match cli.codec {
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
    let (host, port) = resolve_dest(&cli.dest)?;
    // 1 keyframe per second so a fresh receiver renders within ~1s.
    let keyframe_interval = cli.framerate;
    let encoder_args = low_latency_args(
        encoder.gst_element,
        keyframe_interval,
        cli.bitrate_kbps,
        cli.target_usage,
        &cli.rate_control,
        cli.qp,
    );
    // `videorate` is needed because KWin/pipewire emits a variable framerate
    // (it pushes when surfaces change) but the encoder demands a fixed rate.
    // It buffers up to one frame internally; that's ≤16 ms at 60 fps and the
    // price of having the pipeline negotiate at all.
    let desc = format!(
        "{source} ! videorate ! videoconvert ! videoscale \
         ! video/x-raw,format=NV12,width={w},height={h},framerate={fr}/1 \
         ! {enc} {encoder_args} \
         ! {encoding} ! {parse_caps} \
         ! {rtp_pay} pt={pt} mtu={mtu} config-interval=-1 \
         ! udpsink host={host} port={port} sync=false async=false",
        source = cli.source,
        enc = encoder.gst_element,
        w = cli.width,
        h = cli.height,
        fr = cli.framerate,
        pt = cli.pt,
        mtu = cli.mtu,
    );
    info!(%desc, "building pipeline");
    let element = gst::parse::launch(&desc)
        .map_err(|e| anyhow!("pipeline parse failed: {e}\n  desc=`{desc}`"))?;
    element
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("parsed element is not a gst::Pipeline"))
}

fn run_until_eos_or_error(pipeline: &gst::Pipeline) -> Result<()> {
    let bus = pipeline
        .bus()
        .ok_or_else(|| anyhow!("pipeline has no bus"))?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| anyhow!("set_state(Playing) failed: {e}"))?;
    info!("pipeline running; Ctrl-C to stop");

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .ok();

    loop {
        if rx.try_recv().is_ok() {
            info!("ctrl-c received");
            break;
        }
        match bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
            None => continue,
            Some(msg) => match msg.view() {
                gst::MessageView::Eos(_) => {
                    info!("end of stream");
                    break;
                }
                gst::MessageView::Error(e) => {
                    error!(
                        src = ?e.src().map(|s| s.path_string()),
                        error = %e.error(),
                        debug = ?e.debug(),
                        "pipeline error"
                    );
                    return Err(anyhow!("{}", e.error()));
                }
                gst::MessageView::Warning(w) => {
                    warn!(
                        src = ?w.src().map(|s| s.path_string()),
                        warning = %w.error(),
                        "pipeline warning"
                    );
                }
                _ => {}
            },
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
    let encoder = pick_encoder(cli.codec)
        .ok_or_else(|| anyhow!("no encoder for codec {:?} found in the GStreamer registry", cli.codec))?;
    info!(
        codec = ?cli.codec,
        encoder = encoder.gst_element,
        hardware = encoder.hardware,
        dest = %cli.dest,
        "selected encoder"
    );

    let pipeline = build_pipeline(&cli, &encoder)?;
    run_until_eos_or_error(&pipeline)
}
