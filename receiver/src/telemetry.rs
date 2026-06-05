//! gRPC telemetry server. Streams receiver-side health (decode CPU, packet
//! rate, UDP backlog, drops) to subscribing senders so they can throttle from
//! real state. Runs on its own tokio runtime thread, independent of the
//! GStreamer pipeline; stats are shared via atomics.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use allcast_telemetry::telemetry_server::{Telemetry, TelemetryServer};
use allcast_telemetry::{SubscribeRequest, TelemetrySample};
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

/// Live counters shared between the GStreamer pipeline and the gRPC server.
pub struct SharedStats {
    /// Decode pipeline active (vs idle daemon).
    pub active: AtomicBool,
    /// Cumulative RTP packets seen at the udpsrc pad.
    pub packets: AtomicU64,
    listen_port: u16,
    ncpu: u32,
}

impl SharedStats {
    pub fn new(listen_port: u16) -> Arc<Self> {
        let ncpu = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1);
        Arc::new(Self {
            active: AtomicBool::new(false),
            packets: AtomicU64::new(0),
            listen_port,
            ncpu,
        })
    }
}

struct TelemetrySvc {
    stats: Arc<SharedStats>,
}

#[tonic::async_trait]
impl Telemetry for TelemetrySvc {
    type SubscribeStream = Pin<Box<dyn Stream<Item = Result<TelemetrySample, Status>> + Send>>;

    async fn subscribe(
        &self,
        req: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let r = req.into_inner();
        let interval_ms = if r.interval_ms == 0 { 250 } else { r.interval_ms.clamp(50, 2000) } as u64;
        let stats = self.stats.clone();
        info!(sender = %r.sender_id, interval_ms, "telemetry subscriber connected");

        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
            let mut last_packets = stats.packets.load(Ordering::Relaxed);
            let mut last_cpu = read_self_cpu_ticks();
            let mut last_at = Instant::now();
            loop {
                ticker.tick().await;
                let now = Instant::now();
                let dt = now.duration_since(last_at).as_secs_f64().max(1e-3);
                last_at = now;

                let packets = stats.packets.load(Ordering::Relaxed);
                let packet_rate = packets.saturating_sub(last_packets) as f64 / dt;
                last_packets = packets;

                let cpu = read_self_cpu_ticks();
                // SC_CLK_TCK is 100 on Linux: ticks/100 = cpu-seconds.
                let decode_cpu_pct = (cpu.saturating_sub(last_cpu) as f64 / 100.0) / dt * 100.0;
                last_cpu = cpu;

                let sample = TelemetrySample {
                    timestamp_ms: now_ms(),
                    active: stats.active.load(Ordering::Relaxed),
                    decode_cpu_pct,
                    recv_queue_bytes: read_recv_queue(stats.listen_port),
                    rcvbuf_drops: read_rcvbuf_drops(),
                    packet_rate,
                    ncpu: stats.ncpu,
                };
                if tx.send(Ok(sample)).await.is_err() {
                    break; // subscriber went away
                }
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

/// Spawn the telemetry gRPC server on its own runtime thread. Non-fatal: a
/// failure here just means senders can't subscribe; media is unaffected.
pub fn serve(stats: Arc<SharedStats>, port: u16) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                warn!(error = %e, "telemetry: failed to build runtime");
                return;
            }
        };
        rt.block_on(async move {
            let addr = match format!("0.0.0.0:{port}").parse() {
                Ok(a) => a,
                Err(e) => {
                    warn!(error = %e, "telemetry: bad listen address");
                    return;
                }
            };
            info!(%addr, "telemetry gRPC server listening");
            if let Err(e) = tonic::transport::Server::builder()
                .add_service(TelemetryServer::new(TelemetrySvc { stats }))
                .serve(addr)
                .await
            {
                warn!(error = %e, "telemetry server exited");
            }
        });
    });
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// utime + stime (clock ticks) for this process from /proc/self/stat.
fn read_self_cpu_ticks() -> u64 {
    let Ok(s) = std::fs::read_to_string("/proc/self/stat") else {
        return 0;
    };
    // The comm field (2) is parenthesised and may contain spaces; everything
    // after the final ')' is space-separated, starting at field 3 (state).
    let Some(idx) = s.rfind(')') else { return 0 };
    let rest: Vec<&str> = s[idx + 1..].split_whitespace().collect();
    // utime = field 14, stime = field 15 → offsets 11 and 12 after state(3).
    if rest.len() > 12 {
        let utime = rest[11].parse::<u64>().unwrap_or(0);
        let stime = rest[12].parse::<u64>().unwrap_or(0);
        return utime + stime;
    }
    0
}

/// Bytes queued on the UDP receive socket for `port`, from /proc/net/udp[6].
fn read_recv_queue(port: u16) -> u32 {
    let want = format!("{port:04X}");
    for path in ["/proc/net/udp", "/proc/net/udp6"] {
        let Ok(s) = std::fs::read_to_string(path) else { continue };
        for line in s.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() <= 4 {
                continue;
            }
            // f[1] = local_address "IP:PORT" (hex); f[4] = "tx_queue:rx_queue".
            let Some((_, p)) = f[1].split_once(':') else { continue };
            if p.eq_ignore_ascii_case(&want) {
                if let Some((_, rx)) = f[4].split_once(':') {
                    if let Ok(v) = u32::from_str_radix(rx, 16) {
                        return v;
                    }
                }
            }
        }
    }
    0
}

/// Cumulative UDP RcvbufErrors (SO_RCVBUF overflow drops) from /proc/net/snmp.
fn read_rcvbuf_drops() -> u64 {
    let Ok(s) = std::fs::read_to_string("/proc/net/snmp") else {
        return 0;
    };
    let mut udp = s.lines().filter(|l| l.starts_with("Udp:"));
    let _header = udp.next();
    if let Some(data) = udp.next() {
        let v: Vec<&str> = data.split_whitespace().collect();
        // Udp: In NoPorts InErrors Out RcvbufErrors ...  → index 5
        if v.len() > 5 {
            return v[5].parse().unwrap_or(0);
        }
    }
    0
}
