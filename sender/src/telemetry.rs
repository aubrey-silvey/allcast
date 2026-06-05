//! gRPC telemetry client + adaptive throttle. Subscribes to the receiver's
//! health stream and adjusts the encoder's target bitrate live so the receiver
//! stays out of decode-saturation / packet-loss territory. Best-effort: any
//! failure here is logged and retried; the media path is never blocked.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use allcast_telemetry::telemetry_client::TelemetryClient;
use allcast_telemetry::SubscribeRequest;
use tracing::{info, warn};

/// Tunables for the adaptive controller.
pub struct ThrottleConfig {
    pub receiver_host: String,
    pub telemetry_port: u16,
    pub interval_ms: u32,
    pub max_bitrate_kbps: u32,
    pub min_bitrate_kbps: u32,
}

/// Shared handle to the live encoder element. The capture pipeline is rebuilt
/// on auto-recovery, so the controller always adjusts whatever encoder is
/// current (or nothing, between rebuilds).
pub struct EncoderHandle {
    inner: Mutex<Option<gst::Element>>,
}

impl EncoderHandle {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(None) })
    }

    /// Point the controller at the current encoder (or `None` while rebuilding).
    pub fn set(&self, enc: Option<gst::Element>) {
        if let Ok(mut g) = self.inner.lock() {
            *g = enc;
        }
    }

    /// Apply a target bitrate (kbps) if the current encoder exposes `bitrate`.
    fn apply_bitrate(&self, kbps: u32) {
        if let Ok(g) = self.inner.lock() {
            if let Some(enc) = g.as_ref() {
                if enc.find_property("bitrate").is_some() {
                    enc.set_property("bitrate", kbps);
                }
            }
        }
    }
}

/// Spawn the telemetry client + controller on its own runtime thread.
pub fn spawn(cfg: ThrottleConfig, enc: Arc<EncoderHandle>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                warn!(error = %e, "throttle: failed to build runtime");
                return;
            }
        };
        rt.block_on(run(cfg, enc, stop));
    });
}

async fn run(cfg: ThrottleConfig, enc: Arc<EncoderHandle>, stop: Arc<AtomicBool>) {
    let mut target = cfg.max_bitrate_kbps;
    while !stop.load(Ordering::Relaxed) {
        if let Err(e) = stream_once(&cfg, &enc, &stop, &mut target).await {
            warn!(error = %e, "throttle: telemetry stream ended — reconnecting");
        }
        // Don't hammer reconnects; media is unaffected meanwhile.
        for _ in 0..20 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

async fn stream_once(
    cfg: &ThrottleConfig,
    enc: &Arc<EncoderHandle>,
    stop: &Arc<AtomicBool>,
    target: &mut u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = format!("http://{}:{}", cfg.receiver_host, cfg.telemetry_port);
    let mut client = TelemetryClient::connect(endpoint.clone()).await?;
    info!(%endpoint, "throttle: subscribed to receiver telemetry");

    let req = SubscribeRequest {
        sender_id: "allcast-sender".into(),
        interval_ms: cfg.interval_ms,
    };
    let mut stream = client.subscribe(req).await?.into_inner();

    let mut last_drops = 0u64;
    let mut have_drops = false;

    while let Some(sample) = stream.message().await? {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // Only adapt while the receiver is actually decoding.
        if !sample.active {
            continue;
        }

        let drop_delta = if have_drops {
            sample.rcvbuf_drops.saturating_sub(last_drops)
        } else {
            0
        };
        last_drops = sample.rcvbuf_drops;
        have_drops = true;

        let prev = *target;
        // Decode is single-core-bound, so decode_cpu_pct ~100 = one core pegged.
        // Back off hard on saturation or fresh packet loss; recover gently when
        // there's clear headroom.
        if sample.decode_cpu_pct > 90.0 || drop_delta > 0 {
            *target = (*target * 85 / 100).max(cfg.min_bitrate_kbps);
        } else if sample.decode_cpu_pct < 70.0 {
            *target = (*target + *target / 10).min(cfg.max_bitrate_kbps);
        }

        if *target != prev {
            enc.apply_bitrate(*target);
            info!(
                bitrate_kbps = *target,
                prev_kbps = prev,
                decode_cpu_pct = format_args!("{:.0}", sample.decode_cpu_pct),
                recv_q = sample.recv_queue_bytes,
                drop_delta,
                "throttle: adjusted bitrate"
            );
        }
    }
    Ok(())
}
