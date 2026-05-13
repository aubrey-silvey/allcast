# share-screen

Realtime screen replication between two computers, in Rust.

One host captures a monitor, GPU-encodes it to H.265 (or H.264), and pushes
RTP packets to another host's UDP port. The other host listens, decodes,
and renders fullscreen. No HTTP server, no SDP negotiation, no central
relay — sender pushes bytes, receiver listens for bytes, the network
either delivers them or it doesn't.

Measured end-to-end latency on a quiet Gigabit LAN, AMD Radeon 680M
sender → Raspberry Pi 5 receiver:

| Mode | Framerate | Bitrate | Glass-to-glass latency |
|---|---|---:|---:|
| **Text / code** preset | 30 fps | 12 Mbps | **~100 ms** |
| **Video / motion** preset | 60 fps | 20 Mbps | ~125 ms |

(See `docs/latency-method.md` if added later — methodology is OCR of a
ms-precision wall clock burned into the sender stream.)

## Status

**Working today (Linux x86_64 sender ↔ Linux ARM64 receiver):**

- Pure RTP/UDP push (`rtph265pay` → `udpsink` / `udpsrc` → `rtph265depay`)
- VAAPI hardware encode on AMD; per-vendor candidate tables for NVENC, QSV,
  AMF, Vulkan, software fallback
- Software decode on Raspberry Pi 5 (the HW HEVC block exists but its tiled
  output is unusable with current Wayland sinks; A76 quad has plenty of
  headroom for 1080p60 SW decode)
- Idle-then-active daemon on the receiver: binds a UDP probe socket, pops
  fullscreen on first packet, releases the monitor after 3 s of silence
- KDE xdg-desktop-portal screencast on the Linux sender (via a small
  Python helper that exposes the pipewire fd to the child process)
- Embedded `egui` config window: role, codec, peer list (with `+` button
  and saved selection), Text-vs-Video quality preset, monitor info
- Monitor enumeration on Linux via `kscreen-doctor` (KWin) or `wlr-randr`
  (wlroots)

**In progress / queued:**

- macOS Apple Silicon build (avfvideosrc / vtenc / osxvideosink) — code
  paths exist in `share-screen/src/platform.rs`, never built
- Windows x86_64 build (d3d11screencapturesrc / mfh265enc / d3d11videosink)
  — same status
- Native Rust replacement for the Python portal helper (zbus)
- GitHub Actions release builds for all three platforms
- macOS / Windows monitor enumeration (`objc2` + `windows-rs`)

## Workspace layout

Three crates in one Cargo workspace:

| Crate | Purpose | Status |
|---|---|---|
| `share-screen/` | **New unified binary.** Single `share-screen` executable with `send`, `recv`, and `config` subcommands. egui first-run window. Intended to subsume the other two crates once Mac/Windows ports land. | active development |
| `sender/` | Original standalone sender used for benchmarking. CLI-only, lots of knobs (`--target-usage`, `--rate-control`, `--qp`, …). Linux only. | will be deprecated after `share-screen` ports |
| `receiver/` | Original standalone receiver — idle daemon with monitor takeover. Deployed on the Pi 5 via a systemd user unit. | currently the production receiver |

## Quick start

### Pi 5 receiver (existing)

```sh
# On the Pi (sis@10.0.17.84)
git clone ssh://git@ipkeeper.silvey.io:2222/share-screen.git
cd share-screen
./receiver/install.sh                # builds + installs + enables systemd user unit
journalctl --user -u share-screen-receiver -f
```

Receiver binds UDP `5004` and idles until traffic arrives. Settings live in
`~/.config/share-screen-receiver.env`.

### Linux sender (current, via Python portal helper)

```sh
cd ~/source/share-screen
cargo build --release -p share-screen-sender

# /tmp/screencast-portal.py is a small Python script that drives the
# xdg-desktop-portal screencast flow and execs the sender as a child
# with the pipewire fd inherited.
python3 /tmp/screencast-portal.py \
    target/release/share-screen-sender \
    --dest 10.0.17.84:5004 \
    --codec h265 --framerate 30 --bitrate-kbps 12000 --target-usage 2 \
    --source 'pipewiresrc fd=@PW_FD@ path=@NODE_ID@'
```

### Unified `share-screen` binary (new)

```sh
cargo run -p share-screen --release
# First launch pops the egui config window. Save → starts in the
# configured role. Re-open the GUI anytime with:
share-screen config

# Subcommands work too:
share-screen send       # use saved config
share-screen recv       # use saved config
share-screen where      # print path of the config file
share-screen monitors   # debug: list detected monitors
```

Config lives in `~/.config/share-screen/config.toml` (XDG paths on Mac and
Windows respectively).

## Architecture decisions worth knowing

- **No HTTP / no negotiation.** Earlier the project ran WHEP (HTTP-based
  WebRTC egress) — that was removed in favour of bare RTP/UDP. The sender
  knows its peers from config; the receiver binds a port and decodes
  whatever shows up. Trade-off: peer addresses are static, no NAT
  traversal, no auth. Fine for a known-LAN deployment.
- **H.265 RTP packetization is in-house when needed.** The old WHEP code
  needed an RTP H.265 payloader since `webrtc-rs` doesn't ship one — it
  lived at `sender/src/h265.rs`. The current pure-RTP path uses
  `rtph265pay` from `gst-plugins-good` and that file is gone.
- **Pi 5 software decode is intentional.** `v4l2slh265dec` emits a
  `NV12_128C8` tiled DMA-BUF that current Wayland sinks can't import;
  there is no detiler in stock Trixie (the PiSP back-end driver `pispbe`
  has the hardware, but no GStreamer element wires to it). Until upstream
  ships that glue, `avdec_h265` is the right choice — A76 cores have
  enough budget for 1080p60.
- **Latency floor is ~100 ms.** Compositor capture, encoder, network,
  jitter buffer, decoder, sink, scanout. Encoder/decoder hot-rodding
  (target-usage, CQP vs CBR, etc.) moves the needle by ~10 ms; the only
  big lever we found is **dropping from 60 fps to 30 fps**, which halves
  the Pi's decoder pressure.

## Latency tuning

Two preset profiles in the GUI cover the two interesting cases:

- **Text / code**: 30 fps, 12 Mbps, optimised for keystroke responsiveness.
  Snappier; motion looks choppier.
- **Video / motion**: 60 fps, 20 Mbps, smoother. Higher latency.

The CLI form (legacy `sender` crate) exposes all the knobs separately —
useful for the benchmarks recorded above.

## Known limits

- **Receiver-side monitor placement on Wayland is compositor-controlled.**
  No standard protocol lets a client request which output its surface
  appears on. `wlr-output-management-v1` is read-only from clients.
  Practical workaround: configure your Wayland compositor (e.g. labwc
  `~/.config/labwc/rc.xml`) to place share-screen's surface on the desired
  output by class name.
- **Python portal helper on Linux** — the sender currently needs
  `/tmp/screencast-portal.py` to start a screencast session and hand the
  pipewire fd to the binary. Replacing this with native Rust (zbus) is
  queued.
- **No reconnect grace.** When traffic stops for `idle_timeout_s`, the
  receiver tears the pipeline down. The next stream's first ~150 ms of
  packets land before the pipeline is back up; the next keyframe (1 s
  default) restores the picture.
- **One peer per active sender.** Multiple destinations need
  `multiudpsink` — code change is small but not done yet.

## Repository

- Remote: `ssh://git@ipkeeper.silvey.io:2222/share-screen.git` (ipkeeper
  self-hosted Git). `master` is the trunk; pushes to `dev` would trigger
  a container build, which is not relevant for this desktop-binary
  project — don't push there.
- Pi 5 receiver target: `monitor2` / `10.0.17.84`. Keyless SSH from this
  dev box, passwordless sudo on the Pi for the `sis` account; details in
  the local memory.
