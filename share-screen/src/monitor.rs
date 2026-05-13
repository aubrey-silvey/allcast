//! Cross-platform monitor enumeration. Best-effort: tries OS-native tools and
//! falls back to an empty list when not available. Different from the active
//! monitor-share *negotiation* that happens via the xdg-desktop-portal on
//! Linux — the portal popup is the authoritative picker there.

use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    KdeKScreen,
    WlrRandr,
    Cocoa,
    Win32,
    None,
}

pub struct Enumeration {
    pub source: Source,
    pub names: Vec<String>,
    /// True if monitor selection is meaningful for *this* role + OS combo.
    /// On Linux + role=sender, the portal handles it, so the value here is
    /// informational only and we tell the user.
    pub user_selectable: bool,
}

#[cfg(target_os = "linux")]
pub fn enumerate_for_sender() -> Result<Enumeration> {
    // Linux sender always goes through the xdg-portal screencast prompt,
    // which is the authoritative monitor picker. We can list what's there
    // for user awareness, but the value isn't programmatically honoured.
    let (source, names) = enumerate_linux();
    Ok(Enumeration { source, names, user_selectable: false })
}

#[cfg(target_os = "linux")]
pub fn enumerate_for_receiver() -> Result<Enumeration> {
    let (source, names) = enumerate_linux();
    // On wlroots compositors (labwc / sway / Hyprland), there's no standard
    // protocol to tell the compositor which output a client surface should
    // appear on. The value is informational; placement is the compositor's
    // choice. KMS-direct sinks (kmssink connector-id=N) can actually pick.
    Ok(Enumeration { source, names, user_selectable: false })
}

#[cfg(target_os = "linux")]
fn enumerate_linux() -> (Source, Vec<String>) {
    // Try KDE's kscreen-doctor first (KWin Wayland and X).
    if let Some(names) = run_kscreen_doctor() {
        if !names.is_empty() {
            return (Source::KdeKScreen, names);
        }
    }
    // Try wlr-randr next (wlroots compositors).
    if let Some(names) = run_wlr_randr() {
        if !names.is_empty() {
            return (Source::WlrRandr, names);
        }
    }
    (Source::None, Vec::new())
}

#[cfg(target_os = "linux")]
fn run_kscreen_doctor() -> Option<Vec<String>> {
    use std::process::Command;
    let out = Command::new("kscreen-doctor").arg("-o").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut names = Vec::new();
    // kscreen-doctor -o lines look like (with ANSI escapes mixed in):
    //   "[01;32mOutput: [0;0m1 eDP-1 b549..."
    // Strip ANSI, then split: ["Output:", "1", "eDP-1", "<uuid>", ...]
    for line in text.lines() {
        let plain = strip_ansi(line);
        let trimmed = plain.trim();
        if let Some(rest) = trimmed.strip_prefix("Output:") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            // Output: <num> <connector_name> [<uuid>] ...
            // We want index 1 (the connector name like eDP-1, HDMI-A-1).
            if parts.len() >= 2 {
                let candidate = parts[1];
                if is_connector_name(candidate) {
                    names.push(candidate.to_string());
                }
            }
        }
    }
    Some(names)
}

#[cfg(target_os = "linux")]
fn strip_ansi(s: &str) -> String {
    // Minimal CSI / SGR stripper. Removes ESC [ ... letter sequences.
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() && !(bytes[i] as char).is_ascii_alphabetic() {
                i += 1;
            }
            i += 1; // consume the terminating letter
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn is_connector_name(s: &str) -> bool {
    // Names look like eDP-1, HDMI-A-1, DP-2, etc. Reject UUID-looking strings.
    s.contains('-')
        && s.len() <= 16
        && !s.chars().filter(|c| *c == '-').count() > 4
        && s.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(target_os = "linux")]
fn run_wlr_randr() -> Option<Vec<String>> {
    use std::process::Command;
    let out = Command::new("wlr-randr").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let names = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            // wlr-randr prints output names left-aligned and details indented.
            if !l.starts_with(char::is_whitespace) {
                l.split_whitespace().next().map(String::from)
            } else {
                None
            }
        })
        .collect();
    Some(names)
}

#[cfg(target_os = "macos")]
pub fn enumerate_for_sender() -> Result<Enumeration> {
    Ok(Enumeration { source: Source::Cocoa, names: cocoa_screens(), user_selectable: true })
}
#[cfg(target_os = "macos")]
pub fn enumerate_for_receiver() -> Result<Enumeration> {
    Ok(Enumeration { source: Source::Cocoa, names: cocoa_screens(), user_selectable: true })
}
#[cfg(target_os = "macos")]
fn cocoa_screens() -> Vec<String> {
    // TODO: implement via objc2 / core-graphics. For now list indices only.
    Vec::new()
}

#[cfg(target_os = "windows")]
pub fn enumerate_for_sender() -> Result<Enumeration> {
    Ok(Enumeration { source: Source::Win32, names: win_screens(), user_selectable: true })
}
#[cfg(target_os = "windows")]
pub fn enumerate_for_receiver() -> Result<Enumeration> {
    Ok(Enumeration { source: Source::Win32, names: win_screens(), user_selectable: true })
}
#[cfg(target_os = "windows")]
fn win_screens() -> Vec<String> {
    // TODO: implement via windows-rs / EnumDisplayMonitors. Empty for now.
    Vec::new()
}
