# allcast — known issues & what needs fixing

Findings from the 2026-06-16 side-by-side test against Moonlight/Sunshine on the
production Pi 5 (`ghostvox@192.168.0.208`, Trixie arm64), software decode on the
receiver. See also `moonlight/` for the working alternative, and
`hw-decode-status.md` (on `feat/adaptive-bitrate-and-fast-start`) for the
deeper HW-decode investigation.

> **Heads-up:** much of the *smoothness/robustness* work below may already be
> addressed on the **`feat/adaptive-bitrate-and-fast-start`** branch (commit
> "low-latency fast-start, resilient capture, adaptive bitrate via gRPC
> telemetry"), which was **not** merged when these notes were written. **Verify
> against that branch before acting.**

## 1. Sender screen-capture is unimplemented on Linux (blocker) — likely STILL OPEN

`allcast/src/sender.rs::run()` uses `platform::default_source()` which returns
the literal `pipewiresrc fd=@PW_FD@ path=@NODE_ID@`. Nothing performs the
xdg-desktop-portal ScreenCast negotiation or substitutes the real PipeWire
fd/node-id, so `allcast send` dies at pipeline parse:

```
could not set property "fd" in element "pipewiresrc" to "@PW_FD@"
```

→ The unified `allcast` binary cannot capture a Wayland desktop at all. The
`feat` branch reworks the **standalone** `sender/src/main.rs` (defaults to
`ximagesrc`, X11-only) but does **not** touch `allcast/src/sender.rs`, so this
gap likely remains for the unified binary on KDE Wayland.

**Fix:** implement the portal handshake (CreateSession → SelectSources →
Start → OpenPipeWireRemote) to get the fd + node id, then substitute into the
pipeline. (`ashpd` is the usual Rust crate.)

## 2. Receiver smoothness at 1080p60 — VERIFY against feat branch

With a steady real HEVC stream, the receiver (`avdec_h265 max-threads=1` →
`videoconvert` → `kmssink sync=false`, bare KMS) showed:

- **Tearing** under motion — `kmssink sync=false` does no page-flip sync.
- **Periodic hitches / brief white frames** even with flapping disabled —
  single-threaded software decode underruns at 1080p60 (matches the original
  "not clean 60fps" note).

**Fix ideas:** allow multi-threaded decode (the `max-threads=1` low-latency
choice trades throughput it can't spare on the Pi); offer a vsync sink option;
the feat branch's adaptive-bitrate may relieve the underruns.

## 3. Idle-detection causes false teardowns (white flashes) — VERIFY against feat branch

The daemon infers "stream idle" from the **decoder's** udpsrc pad going quiet,
but that also happens on any *downstream* stall — so a decode hiccup looks like
"stream ended", tears down the pipeline (KMS modeset = white flash), and
rebuilds. With the default `idle_timeout_s=3` this produced a ~15 s flap cycle.
`--idle-timeout-s 60` masks it but doesn't fix the root cause.

**Fix:** base idle detection on the actual UDP socket (packets still arrive
during a decode stall), not the decoder pad. The feat branch's "fast-start /
resilient capture" may already rework this.

## 4. Hardware HEVC decode — see hw-decode-status.md (NOT an allcast bug)

The Pi 5 HW decoder works but `v4l2slh265dec` fails GStreamer `negotiate()`
("Unsupported pixel format") — an upstream GStreamer/kernel bug. Note: stock
`videoconvert` on GStreamer 1.26.2 **already detiles** the Pi's `NV12_128C8`
(SAND), so no custom detiler is needed; the `prototypes/detiler/` on the feat
branch is a benchmark, intentionally not integrated. A 2026-06-16 retest found
`v4l2slh265dec ! kmssink` (no videoconvert) *did* reach PLAYING but rendered
garbage and **wedged the DRM driver on teardown** (required a Pi reboot) — do
not ship direct kmssink SAND scan-out. Keep software decode until the upstream
negotiate bug is fixed.

## Priority

1. **Sender portal capture** (#1) — without it allcast can't send on Wayland.
2. **Receiver smoothness** (#2) + **idle-detection** (#3) — confirm/extend the
   `feat/adaptive-bitrate-and-fast-start` work rather than redo it.
3. HW decode (#4) — blocked upstream; nothing to do but track it.
