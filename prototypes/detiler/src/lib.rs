//! Prototype detiler for the Raspberry Pi HEVC decoder output format
//! `V4L2_PIX_FMT_NV12_COL128` — "NV12 with 128-byte-wide column tiling".
//!
//! Why this exists: the Pi 5's `v4l2slh265dec` decodes HEVC in hardware but
//! emits this tiled DMA-BUF, which current Wayland sinks can't import — so
//! allcast falls back to software decode (the single-core load we've been
//! fighting). A fast detile to linear NV12 unlocks hardware decode. The inner
//! loop is a column→row transpose of 128-byte runs, which is exactly the kind
//! of memory-bound copy where hand-written NEON earns its keep.
//!
//! # Layout (per plane)
//!
//! The plane is cut into vertical COLUMNS 128 bytes wide. Each column stores
//! its rows contiguously (column row 0, then row 1, …); columns run left→right:
//!
//! ```text
//!   tiled_offset(x, y) = col_base(x / 128) + y * 128 + (x % 128)
//!   col_base(c)        = c * (128 * aligned_height)
//! ```
//!
//! Luma and chroma share the scheme. Chroma is interleaved UV, so its byte
//! width equals the luma width and its height is half.
//!
//! # Correctness caveat
//!
//! `aligned_height` (the column-stride basis) for *real* hardware buffers comes
//! from the V4L2 buffer geometry and may be padded (e.g. to 64/128). The
//! round-trip tests here prove the detiler exactly inverts the tiler and that
//! the scalar / NEON / asm kernels agree — they do NOT prove the layout matches
//! a specific driver build. Confirm `aligned_height` against a real buffer dump
//! (`v4l2-ctl`/GStreamer caps) before trusting on-device.

pub const TILE_W: usize = 128;

/// Geometry of one tiled plane.
#[derive(Clone, Copy, Debug)]
pub struct PlaneGeom {
    /// Visible bytes per row (== pixels for 8-bit luma; == width for NV12 UV).
    pub width: usize,
    /// Visible rows to emit.
    pub height: usize,
    /// Column height used for the 128-byte column stride (>= height, may be
    /// padded by the hardware).
    pub aligned_height: usize,
}

impl PlaneGeom {
    #[inline]
    pub fn cols(&self) -> usize {
        self.width.div_ceil(TILE_W)
    }
    /// Bytes between the start of column c and column c+1 in the tiled buffer.
    #[inline]
    pub fn col_stride(&self) -> usize {
        TILE_W * self.aligned_height
    }
    /// Total tiled size of this plane.
    #[inline]
    pub fn tiled_len(&self) -> usize {
        self.cols() * self.col_stride()
    }
    /// Size of the linear (detiled) plane, tightly packed.
    #[inline]
    pub fn linear_len(&self) -> usize {
        self.width * self.height
    }
}

/// (luma, chroma) geometry for an NV12 frame.
pub fn nv12_planes(width: usize, height: usize, aligned_height: usize) -> (PlaneGeom, PlaneGeom) {
    let luma = PlaneGeom { width, height, aligned_height };
    let chroma = PlaneGeom {
        width,
        height: height / 2,
        aligned_height: aligned_height / 2,
    };
    (luma, chroma)
}

// ---------------------------------------------------------------------------
// Scalar reference
// ---------------------------------------------------------------------------

/// Detile one plane into a linear buffer with row stride `dst_stride`.
/// Portable reference implementation.
pub fn detile_plane_scalar(src: &[u8], dst: &mut [u8], g: PlaneGeom, dst_stride: usize) {
    assert!(src.len() >= g.tiled_len(), "src too small");
    assert!(dst.len() >= dst_stride * g.height, "dst too small");
    let col_stride = g.col_stride();
    let full_cols = g.width / TILE_W;
    let rem = g.width % TILE_W;
    for c in 0..g.cols() {
        let copy_w = if c < full_cols { TILE_W } else { rem };
        if copy_w == 0 {
            break;
        }
        let col_base = c * col_stride;
        let x0 = c * TILE_W;
        for y in 0..g.height {
            let s = col_base + y * TILE_W;
            let d = y * dst_stride + x0;
            dst[d..d + copy_w].copy_from_slice(&src[s..s + copy_w]);
        }
    }
}

/// Detile a full NV12 frame (scalar). `dst` is tightly packed linear NV12:
/// luma `width*height` then chroma `width*(height/2)`.
pub fn detile_nv12_scalar(src: &[u8], dst: &mut [u8], width: usize, height: usize, aligned_height: usize) {
    let (luma, chroma) = nv12_planes(width, height, aligned_height);
    let (l_src, c_src) = src.split_at(luma.tiled_len());
    let (l_dst, c_dst) = dst.split_at_mut(luma.linear_len());
    detile_plane_scalar(l_src, l_dst, luma, width);
    detile_plane_scalar(c_src, c_dst, chroma, width);
}

// ---------------------------------------------------------------------------
// NEON (intrinsics)
// ---------------------------------------------------------------------------

/// Detile one plane using NEON. Full 128-wide columns go through a vectorised
/// 128-byte copy (8×16B); a partial trailing column falls back to scalar.
#[cfg(target_arch = "aarch64")]
pub fn detile_plane_neon(src: &[u8], dst: &mut [u8], g: PlaneGeom, dst_stride: usize) {
    use core::arch::aarch64::{vld1q_u8_x4, vst1q_u8_x4};
    assert!(src.len() >= g.tiled_len(), "src too small");
    assert!(dst.len() >= dst_stride * g.height, "dst too small");
    let col_stride = g.col_stride();
    let full_cols = g.width / TILE_W;
    unsafe {
        let src0 = src.as_ptr();
        let dst0 = dst.as_mut_ptr();
        for c in 0..full_cols {
            let mut sp = src0.add(c * col_stride);
            let mut dp = dst0.add(c * TILE_W);
            for _ in 0..g.height {
                // Sequential source reads; prefetch the next column row.
                core::arch::asm!("prfm pldl1keep, [{0}, #256]", in(reg) sp, options(nostack, readonly, preserves_flags));
                let lo = vld1q_u8_x4(sp);
                let hi = vld1q_u8_x4(sp.add(64));
                vst1q_u8_x4(dp, lo);
                vst1q_u8_x4(dp.add(64), hi);
                sp = sp.add(TILE_W);
                dp = dp.add(dst_stride);
            }
        }
    }
    // Partial trailing column.
    let rem = g.width % TILE_W;
    if rem > 0 {
        let c = full_cols;
        let col_base = c * col_stride;
        let x0 = c * TILE_W;
        for y in 0..g.height {
            let s = col_base + y * TILE_W;
            let d = y * dst_stride + x0;
            dst[d..d + rem].copy_from_slice(&src[s..s + rem]);
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn detile_nv12_neon(src: &[u8], dst: &mut [u8], width: usize, height: usize, aligned_height: usize) {
    let (luma, chroma) = nv12_planes(width, height, aligned_height);
    let (l_src, c_src) = src.split_at(luma.tiled_len());
    let (l_dst, c_dst) = dst.split_at_mut(luma.linear_len());
    detile_plane_neon(l_src, l_dst, luma, width);
    detile_plane_neon(c_src, c_dst, chroma, width);
}

// ---------------------------------------------------------------------------
// Pure inline assembly (the literal "rewrite in assembly" ask)
// ---------------------------------------------------------------------------

/// Detile one plane with the per-column copy loop written entirely in AArch64
/// assembly: post-indexed `ld1` streams the column (sequential), `st1` scatters
/// to linear rows by `dst_stride`. Partial trailing column falls back to scalar.
#[cfg(target_arch = "aarch64")]
pub fn detile_plane_asm(src: &[u8], dst: &mut [u8], g: PlaneGeom, dst_stride: usize) {
    assert!(src.len() >= g.tiled_len(), "src too small");
    assert!(dst.len() >= dst_stride * g.height, "dst too small");
    let col_stride = g.col_stride();
    let full_cols = g.width / TILE_W;
    if g.height > 0 {
        unsafe {
            let src0 = src.as_ptr();
            let dst0 = dst.as_mut_ptr();
            for c in 0..full_cols {
                let sp = src0.add(c * col_stride);
                let dp = dst0.add(c * TILE_W);
                core::arch::asm!(
                    "2:",
                    // load 128 bytes of one column row, post-incrementing src
                    "ld1 {{v0.16b, v1.16b, v2.16b, v3.16b}}, [{s}], #64",
                    "ld1 {{v4.16b, v5.16b, v6.16b, v7.16b}}, [{s}], #64",
                    // store to the linear destination row
                    "st1 {{v0.16b, v1.16b, v2.16b, v3.16b}}, [{d}]",
                    "add {dt}, {d}, #64",
                    "st1 {{v4.16b, v5.16b, v6.16b, v7.16b}}, [{dt}]",
                    // advance dst by one linear row, decrement row counter
                    "add {d}, {d}, {stride}",
                    "subs {n}, {n}, #1",
                    "b.ne 2b",
                    s = inout(reg) sp => _,
                    d = inout(reg) dp => _,
                    dt = out(reg) _,
                    stride = in(reg) dst_stride,
                    n = inout(reg) g.height => _,
                    out("v0") _, out("v1") _, out("v2") _, out("v3") _,
                    out("v4") _, out("v5") _, out("v6") _, out("v7") _,
                    options(nostack),
                );
            }
        }
    }
    let rem = g.width % TILE_W;
    if rem > 0 {
        let c = full_cols;
        let col_base = c * col_stride;
        let x0 = c * TILE_W;
        for y in 0..g.height {
            let s = col_base + y * TILE_W;
            let d = y * dst_stride + x0;
            dst[d..d + rem].copy_from_slice(&src[s..s + rem]);
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn detile_nv12_asm(src: &[u8], dst: &mut [u8], width: usize, height: usize, aligned_height: usize) {
    let (luma, chroma) = nv12_planes(width, height, aligned_height);
    let (l_src, c_src) = src.split_at(luma.tiled_len());
    let (l_dst, c_dst) = dst.split_at_mut(luma.linear_len());
    detile_plane_asm(l_src, l_dst, luma, width);
    detile_plane_asm(c_src, c_dst, chroma, width);
}

// ---------------------------------------------------------------------------
// Reference tiler (inverse) — used to synthesize tiled buffers for tests/bench
// without needing the actual hardware decoder.
// ---------------------------------------------------------------------------

fn tile_plane(linear: &[u8], tiled: &mut [u8], g: PlaneGeom, src_stride: usize) {
    let col_stride = g.col_stride();
    let full_cols = g.width / TILE_W;
    let rem = g.width % TILE_W;
    for c in 0..g.cols() {
        let copy_w = if c < full_cols { TILE_W } else { rem };
        if copy_w == 0 {
            break;
        }
        let col_base = c * col_stride;
        let x0 = c * TILE_W;
        for y in 0..g.height {
            let s = y * src_stride + x0;
            let d = col_base + y * TILE_W;
            tiled[d..d + copy_w].copy_from_slice(&linear[s..s + copy_w]);
        }
    }
}

/// Produce a tiled NV12_COL128 buffer from linear NV12. Inverse of the detilers.
pub fn tile_nv12_reference(linear: &[u8], width: usize, height: usize, aligned_height: usize) -> Vec<u8> {
    let (luma, chroma) = nv12_planes(width, height, aligned_height);
    let mut out = vec![0u8; luma.tiled_len() + chroma.tiled_len()];
    let (l_dst, c_dst) = out.split_at_mut(luma.tiled_len());
    let (l_src, c_src) = linear.split_at(luma.linear_len());
    tile_plane(l_src, l_dst, luma, width);
    tile_plane(c_src, c_dst, chroma, width);
    out
}

/// A deterministic linear NV12 test frame (content sensitive to (x,y,plane)).
pub fn make_linear_nv12(width: usize, height: usize) -> Vec<u8> {
    let mut v = vec![0u8; width * height + width * (height / 2)];
    for y in 0..height {
        for x in 0..width {
            v[y * width + x] = (x.wrapping_mul(3) ^ y.wrapping_mul(7)) as u8;
        }
    }
    let base = width * height;
    for y in 0..height / 2 {
        for x in 0..width {
            v[base + y * width + x] = (x.wrapping_add(y).wrapping_mul(5) ^ 0xA5) as u8;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 1920;
    const H: usize = 1080;
    const AH: usize = 1088; // luma column height padded to a multiple of 64

    #[test]
    fn scalar_round_trips() {
        let linear = make_linear_nv12(W, H);
        let tiled = tile_nv12_reference(&linear, W, H, AH);
        let mut out = vec![0u8; linear.len()];
        detile_nv12_scalar(&tiled, &mut out, W, H, AH);
        assert_eq!(out, linear, "scalar detile must invert the tiler");
    }

    #[test]
    fn round_trips_with_partial_column() {
        // 1900 is not a multiple of 128 → exercise the remainder column path.
        let (w, h, ah) = (1900usize, 256usize, 256usize);
        let linear = make_linear_nv12(w, h);
        let tiled = tile_nv12_reference(&linear, w, h, ah);
        let mut out = vec![0u8; linear.len()];
        detile_nv12_scalar(&tiled, &mut out, w, h, ah);
        assert_eq!(out, linear);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn neon_matches_scalar() {
        let linear = make_linear_nv12(W, H);
        let tiled = tile_nv12_reference(&linear, W, H, AH);
        let mut a = vec![0u8; linear.len()];
        let mut b = vec![0u8; linear.len()];
        detile_nv12_scalar(&tiled, &mut a, W, H, AH);
        detile_nv12_neon(&tiled, &mut b, W, H, AH);
        assert_eq!(a, b, "NEON must match scalar");
        assert_eq!(b, linear, "NEON must reproduce the original");
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn asm_matches_scalar() {
        let linear = make_linear_nv12(W, H);
        let tiled = tile_nv12_reference(&linear, W, H, AH);
        let mut a = vec![0u8; linear.len()];
        let mut b = vec![0u8; linear.len()];
        detile_nv12_scalar(&tiled, &mut a, W, H, AH);
        detile_nv12_asm(&tiled, &mut b, W, H, AH);
        assert_eq!(a, b, "asm must match scalar");
        assert_eq!(b, linear, "asm must reproduce the original");
    }
}
