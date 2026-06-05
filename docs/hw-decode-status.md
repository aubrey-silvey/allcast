# Hardware HEVC decode on the Pi 5 — status & findings

_Last investigated: 2026-06-05, on the production receiver (`sis@10.0.17.84`,
Pi 5, GStreamer 1.26.2)._

## TL;DR

Hardware HEVC decode on the Pi 5 is **still not usable**, but the reason has
**moved** since the README was written, and the fix is no longer ours to make:

- The old blocker — "the decoder emits a tiled buffer no Wayland sink can
  import, and there's no detiler" — is **obsolete on GStreamer 1.26.2**. Stock
  `videoconvert` now detiles the Pi's tiled format directly.
- The **current** blocker is one step earlier: `v4l2slh265dec` fails in
  `negotiate()` with *"Unsupported pixel format"* before it ever produces a
  buffer. That's a GStreamer/kernel-driver bug, not an allcast problem.

So: **keep software decode** (`avdec_h265`, current default). The path to HW
decode is `v4l2slh265dec ! videoconvert ! waylandsink` with **no custom code** —
gated entirely on the upstream negotiate bug being fixed.

## What the hardware actually offers

`/dev/video19` is the stateless HEVC decoder:

```
OUTPUT (coded):  'S265'  HEVC Parsed Slice Data
CAPTURE (decoded):
  [0] 'NC12'  Y/CbCr 4:2:0 (128b cols)        # = V4L2_PIX_FMT_NV12_COL128
  [1] 'NC30'  10-bit Y/CbCr 4:2:0 (128b cols)
```

`NC12` is GStreamer's `NV12_128C8`: NV12 where each plane is cut into vertical
**128-byte-wide columns**, each column storing its rows contiguously
(`tiled_offset(x,y) = (x/128)*(128*aligned_height) + y*128 + (x%128)`; chroma
same scheme, half height). This matches the model in `prototypes/detiler/`.

## Tests run (reproducible on the Pi)

| # | Pipeline | Result |
|---|---|---|
| A | `… x265enc ! h265parse ! v4l2slh265dec ! videoconvert ! NV12 ! fakesink` | ❌ decoder `negotiate()`: *Unsupported pixel format* |
| B | `… ! v4l2slh265dec ! fakesink` (native, downstream accepts anything) | ❌ same negotiate error |
| D | `videotestsrc ! NV12_128C8 ! videoconvert ! NV12 ! fakesink` (no HW decoder) | ✅ **EOS, clean — stock videoconvert detiles `NV12_128C8`** |
| E | `… ! v4l2slh265dec ! video/x-raw,format=NV12_128C8 ! fakesink` (format pinned) | ❌ same negotiate error |

Stream in A/B/E was confirmed 8-bit **Main** profile, 4:2:0 — so the negotiate
failure is **not** a bit-depth/profile mismatch.

Error origin:
```
v4l2slh265dec: Unsupported pixel format
../sys/v4l2codecs/gstv4l2codech265dec.c(434): gst_v4l2_codec_h265_dec_negotiate ()
```

## Why the custom NEON detiler was NOT integrated

`prototypes/detiler/` contains a correct, tested NV12_128C8 detiler (scalar +
NEON + pure asm, round-trip validated on aarch64). It was built to solve the
"no detiler" problem — but:

1. **Stock `videoconvert` already does it** (Test D), so custom code would be
   redundant *and* slower.
2. The detile is **memory-bandwidth-bound**: on the Pi the NEON and hand-asm
   versions both lost to scalar `copy_from_slice` (`memcpy`) — ~0.97 ms/frame
   (≈3% of one core at 30 fps). Hand assembly doesn't beat a tuned `memcpy` on a
   pure copy.

The prototype stays as a documented experiment/benchmark; it is **not** wired
into the receiver and should not be.

## When/if the negotiate bug is fixed

The unlock is trivial and stock — change the receiver decode path to prefer
hardware and let `videoconvert` detile:

```
udpsrc … ! rtpjitterbuffer … ! rtph265depay ! h265parse
        ! v4l2slh265dec ! videoconvert ! waylandsink
```

`receiver/src/main.rs` already lists `v4l2slh265dec` in `H265_CANDIDATES` (last,
so SW wins today). Promoting it + ensuring `videoconvert` sits before the sink
is the whole change. Re-test with Test A above first.

## To chase the negotiate bug later (open-ended, upstream)

- Diff GStreamer's `gstv4l2codech265dec.c` format-selection against the
  `NC12`/`NC30` the driver enumerates (`v4l2-ctl -d /dev/video19 --list-formats`).
- Try a newer `gstreamer1.0-plugins-bad` (the v4l2codecs live there) and/or a
  newer rpi kernel; this is plausibly already fixed upstream.
- Search GStreamer/rpi issue trackers for `v4l2slh265dec` + "Unsupported pixel
  format" / `NV12_128C8` negotiate.

Until then, software decode + the adaptive gRPC bitrate throttle remain the
practical CPU-pressure levers.
