//! Correctness gate + microbenchmark for the NV12_COL128 detilers.
//!
//!   cargo run --release --bin detile-bench
//!
//! On x86 only the scalar path runs; build/run on the Pi (aarch64) for NEON/asm.

use std::hint::black_box;
use std::time::Instant;

use detiler::*;

fn bench(name: &str, frames: usize, linear_bytes: usize, mut f: impl FnMut()) {
    for _ in 0..10 {
        f(); // warm caches
    }
    let t = Instant::now();
    for _ in 0..frames {
        f();
    }
    let el = t.elapsed();
    let per = el.as_secs_f64() / frames as f64;
    // Memory traffic is ~read + write of the frame.
    let gbps = 2.0 * linear_bytes as f64 * frames as f64 / el.as_secs_f64() / 1e9;
    println!(
        "  {name:<8} {:>7.3} ms/frame   {gbps:>6.2} GB/s   ~{:>5.0} detiles/s on one core",
        per * 1e3,
        1.0 / per,
    );
}

fn main() {
    let (w, h, ah) = (1920usize, 1080usize, 1088usize);
    let linear = make_linear_nv12(w, h);
    let tiled = tile_nv12_reference(&linear, w, h, ah);
    let mut out = vec![0u8; linear.len()];

    detile_nv12_scalar(&tiled, &mut out, w, h, ah);
    assert_eq!(out, linear, "scalar detile incorrect");

    println!(
        "NV12_COL128 detile — {w}x{h}, {} KiB linear/frame, aligned_height={ah}",
        linear.len() / 1024
    );
    let frames = 2000;
    bench("scalar", frames, linear.len(), || {
        detile_nv12_scalar(black_box(&tiled), black_box(&mut out), w, h, ah)
    });

    #[cfg(target_arch = "aarch64")]
    {
        detile_nv12_neon(&tiled, &mut out, w, h, ah);
        assert_eq!(out, linear, "neon detile incorrect");
        bench("neon", frames, linear.len(), || {
            detile_nv12_neon(black_box(&tiled), black_box(&mut out), w, h, ah)
        });

        detile_nv12_asm(&tiled, &mut out, w, h, ah);
        assert_eq!(out, linear, "asm detile incorrect");
        bench("asm", frames, linear.len(), || {
            detile_nv12_asm(black_box(&tiled), black_box(&mut out), w, h, ah)
        });
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("  (neon/asm build & run on aarch64 — run this on the Pi for the real numbers)");
}
