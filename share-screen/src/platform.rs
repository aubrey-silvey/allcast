//! Per-OS pipeline element choices. Each function returns a candidate list
//! in preference order; the caller probes the GStreamer registry to pick
//! the first available one at runtime.

use crate::config::Codec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

pub fn current_os() -> Os {
    if cfg!(target_os = "linux") {
        Os::Linux
    } else if cfg!(target_os = "macos") {
        Os::Macos
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        // best-effort fallback
        Os::Linux
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Encoder {
    pub gst_element: &'static str,
    pub hardware: bool,
}

pub fn encoder_candidates(os: Os, codec: Codec) -> &'static [(&'static str, bool)] {
    match (os, codec) {
        (Os::Linux, Codec::H265) => &[
            ("nvh265enc", true),
            ("nvcudah265enc", true),
            ("vah265enc", true),
            ("vaapih265enc", true),
            ("qsvh265enc", true),
            ("amfh265enc", true),
            ("x265enc", false),
        ],
        (Os::Linux, Codec::H264) => &[
            ("nvh264enc", true),
            ("nvcudah264enc", true),
            ("vah264enc", true),
            ("vaapih264enc", true),
            ("qsvh264enc", true),
            ("amfh264enc", true),
            ("vulkanh264enc", true),
            ("x264enc", false),
        ],
        (Os::Macos, Codec::H265) => &[
            ("vtenc_h265_hw", true),
            ("vtenc_h265", true),
            ("x265enc", false),
        ],
        (Os::Macos, Codec::H264) => &[
            ("vtenc_h264_hw", true),
            ("vtenc_h264", true),
            ("x264enc", false),
        ],
        (Os::Windows, Codec::H265) => &[
            ("nvh265enc", true),
            ("nvcudah265enc", true),
            ("qsvh265enc", true),
            ("mfh265enc", true),
            ("amfh265enc", true),
            ("x265enc", false),
        ],
        (Os::Windows, Codec::H264) => &[
            ("nvh264enc", true),
            ("nvcudah264enc", true),
            ("qsvh264enc", true),
            ("mfh264enc", true),
            ("amfh264enc", true),
            ("x264enc", false),
        ],
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Decoder {
    pub gst_element: &'static str,
    pub hardware: bool,
}

pub fn decoder_candidates(os: Os, codec: Codec) -> &'static [(&'static str, bool)] {
    match (os, codec) {
        // Linux: SW first because Pi 5's HW path (v4l2slh265dec) emits tiled
        // NV12 that compositors can't import; on x86 HW (vah265dec) is fine
        // and would win — but we keep SW first for predictability across
        // Linux variants. Tune per machine if you care.
        (Os::Linux, Codec::H265) => &[
            ("avdec_h265", false),
            ("vah265dec", true),
            ("vaapih265dec", true),
            ("nvh265dec", true),
            ("nvcudah265dec", true),
            ("qsvh265dec", true),
            ("vulkanh265dec", true),
            ("v4l2slh265dec", true),
        ],
        (Os::Linux, Codec::H264) => &[
            ("avdec_h264", false),
            ("vah264dec", true),
            ("vaapih264dec", true),
            ("nvh264dec", true),
            ("nvcudah264dec", true),
            ("qsvh264dec", true),
            ("vulkanh264dec", true),
            ("v4l2h264dec", true),
        ],
        (Os::Macos, Codec::H265) => &[
            ("vtdec_hw", true),
            ("vtdec", true),
            ("avdec_h265", false),
        ],
        (Os::Macos, Codec::H264) => &[
            ("vtdec_hw", true),
            ("vtdec", true),
            ("avdec_h264", false),
        ],
        (Os::Windows, Codec::H265) => &[
            ("d3d12h265dec", true),
            ("d3d11h265dec", true),
            ("nvh265dec", true),
            ("nvcudah265dec", true),
            ("qsvh265dec", true),
            ("mfh265dec", true),
            ("avdec_h265", false),
        ],
        (Os::Windows, Codec::H264) => &[
            ("d3d12h264dec", true),
            ("d3d11h264dec", true),
            ("nvh264dec", true),
            ("nvcudah264dec", true),
            ("qsvh264dec", true),
            ("mfh264dec", true),
            ("avdec_h264", false),
        ],
    }
}

/// First-match GStreamer element from a candidate list, hardware-preferred.
pub fn pick_element(candidates: &[(&'static str, bool)]) -> Option<(&'static str, bool)> {
    use gstreamer::prelude::*;
    let registry = gstreamer::Registry::get();
    candidates.iter().find(|(name, _)| {
        registry
            .find_feature(name, gstreamer::ElementFactory::static_type())
            .is_some()
    }).copied()
}

/// Default sink for the local OS. `auto` lets the user defer.
pub fn default_sink(os: Os) -> &'static str {
    match os {
        Os::Linux => "waylandsink fullscreen=true sync=false",
        Os::Macos => "osxvideosink sync=false",
        Os::Windows => "d3d11videosink sync=false",
    }
}

/// Default capture source for the local OS, in GStreamer syntax. May need a
/// portal step on Linux to fill in `fd=N path=ID`.
pub fn default_source(os: Os) -> &'static str {
    match os {
        // For Linux, the portal helper provides fd + node id, see portal.rs.
        Os::Linux => "pipewiresrc fd=@PW_FD@ path=@NODE_ID@",
        Os::Macos => "avfvideosrc capture-screen=true",
        Os::Windows => "d3d11screencapturesrc",
    }
}
