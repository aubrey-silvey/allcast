# share-screen

End-to-end realtime screen sharing in Rust. Two crates in one workspace:

- **`sender/`** — captures a display, encodes on the GPU, exposes a WHEP
  endpoint. Runs on the source machine.
- **`receiver/`** — pulls a WHEP stream, decodes (HW where available), and
  takes over the display via bare KMS. Built for a dedicated Raspberry Pi 5
  kiosk.

The two halves negotiate codec at session time. Both halves implement
WHEP (RFC 9725) over plain HTTP.

## Codec negotiation

The SDP offer carries both H.264 and H.265. The peer's answer selects one
and the pipeline is built around that choice.

| Codec | Encode HW vendors | Decode HW on Pi 5 |
|---|---|---|
| H.264 | NVIDIA NVENC, Intel VAAPI/QSV, AMD VAAPI/AMF/Vulkan | ❌ (CPU only on Pi 5) |
| H.265 | NVIDIA NVENC, Intel VAAPI/QSV, AMD VAAPI/AMF | ✅ (dedicated HEVC block) |

VP9 is not in the codec set: AMD has never shipped VP9 *encode* in any
GPU generation, so it cannot be a "GPU on all three vendors" path.

webrtc-rs does not ship an H.265 RTP payloader, so `sender/src/h265.rs`
implements RFC 7798 (single NAL packets + Fragmentation Units). The
H.265 session uses `TrackLocalStaticRTP` + `rtp::Packetizer` with that
payloader; H.264 still uses the built-in `TrackLocalStaticSample` path.

## sender/

Captures, GPU-encodes, serves a WHEP endpoint.

```sh
cargo run -p share-screen-sender --release -- --bind 0.0.0.0:8080
```

| Flag | Env | Default | Meaning |
|---|---|---|---|
| `--bind` | `BIND` | `0.0.0.0:8080` | HTTP bind address |
| `--source` | `SOURCE` | `ximagesrc use-damage=false` | GStreamer source element + props |
| `--framerate` | `FRAMERATE` | `30` | Output framerate (fps) |
| `--width` | `WIDTH` | `1920` | Output width |
| `--height` | `HEIGHT` | `1080` | Output height |

`ximagesrc` works on X11. Under Wayland use
`--source 'pipewiresrc path=<node-id>'` after picking a node via the
xdg-desktop-portal screencast flow.

H.265 payloader has unit tests:

```sh
cargo test -p share-screen-sender
```

## receiver/

Pulls a WHEP stream and pushes decoded frames straight to the display
via `kmssink`. No X, no Wayland, no compositor. Designed for a Pi 5
that's dedicated to this purpose.

```sh
cargo run -p share-screen-receiver --release -- \
  --whep-url http://sender.local:8080/whep
```

| Flag | Env | Default | Meaning |
|---|---|---|---|
| `--whep-url` | `WHEP_URL` | *(required)* | Sender's `/whep` endpoint URL |
| `--kms-device` | `KMS_DEVICE` | `/dev/dri/card1` | DRM device (Pi 5: usually card1 = vc4) |
| `--kms-connector` | `KMS_CONNECTOR` | *unset* | Specific HDMI connector ID; auto-picks if omitted |

### Pi 5 specifics

- H.265 uses the `v4l2slh265dec` element — the dedicated HEVC decoder
  block exposed by the Linux V4L2 stateless decoder driver. H.264 falls
  back to `avdec_h264` (CPU; the A76 quad is fast enough for 1080p30,
  but H.265 is the preferred path because it stays out of the CPU).
- The unit must run **without** a desktop. Boot to `multi-user.target`,
  not `graphical.target`, so nothing else holds DRM master.
- The receiver process needs to be in the `video` and `render` groups
  to open `/dev/dri/*`.

### Cross-build

The receiver is intended to be built on the Pi itself (cross-compiling
GStreamer's C deps is fiddly). On a fresh Pi 5 with Raspberry Pi OS
Bookworm:

```sh
sudo apt install -y \
  rustc cargo \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
  gstreamer1.0-libav gstreamer1.0-v4l2
git clone <repo> && cd share-screen
cargo build --release -p share-screen-receiver
sudo install -m 755 target/release/share-screen-receiver /usr/local/bin/
```

### systemd

`receiver/contrib/share-screen-receiver.service` is a unit file that
runs the receiver as a system service. Drop it into
`/etc/systemd/system/`, write `/etc/share-screen-receiver.env` with
`WHEP_URL=http://...`, then:

```sh
sudo systemctl enable --now share-screen-receiver
```

To make the Pi boot straight into the receiver:

```sh
sudo systemctl set-default multi-user.target
sudo systemctl disable lightdm    # if installed
```
