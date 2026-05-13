//! egui first-run window: role, peer list, quality preset, codec, monitor.

use crate::config::{self, Codec, Config, Peer, Quality, Role};
use crate::monitor;
use std::sync::{Arc, Mutex};

pub fn run(initial: Option<Config>) -> Option<Config> {
    let state = Arc::new(Mutex::new(AppState::new(initial.unwrap_or_default())));
    let result: Arc<Mutex<Option<Config>>> = Arc::new(Mutex::new(None));
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([560.0, 720.0])
            .with_resizable(true)
            .with_title("share-screen — configure"),
        ..Default::default()
    };
    let state_c = state.clone();
    let result_c = result.clone();
    let r = eframe::run_simple_native("share-screen config", native, move |ctx, _| {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            let mut s = state_c.lock().unwrap();
            s.draw(ui);
            if let Some(cfg) = s.taken.take() {
                *result_c.lock().unwrap() = Some(cfg);
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            }
        });
    });
    if let Err(e) = r { eprintln!("egui run error: {e:?}"); }
    let out = result.lock().unwrap().clone();
    out
}

struct AppState {
    cfg: Config,
    monitors: monitor::Enumeration,
    new_peer_label: String,
    new_peer_addr: String,
    taken: Option<Config>,
    error: Option<String>,
}

impl AppState {
    fn new(cfg: Config) -> Self {
        let monitors = match cfg.role {
            Role::Sender => monitor::enumerate_for_sender(),
            Role::Receiver => monitor::enumerate_for_receiver(),
        }
        .unwrap_or(monitor::Enumeration { source: monitor::Source::None, names: vec![], user_selectable: true });
        Self {
            cfg,
            monitors,
            new_peer_label: String::new(),
            new_peer_addr: String::new(),
            taken: None,
            error: None,
        }
    }

    fn refresh_monitors(&mut self) {
        self.monitors = match self.cfg.role {
            Role::Sender => monitor::enumerate_for_sender(),
            Role::Receiver => monitor::enumerate_for_receiver(),
        }
        .unwrap_or(monitor::Enumeration {
            source: monitor::Source::None,
            names: vec![],
            user_selectable: true,
        });
    }

    fn draw(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui::{Color32, ComboBox, RichText};

        ui.heading("share-screen — configuration");
        ui.separator();
        ui.add_space(6.0);

        // ROLE -----------------------------------------------------------
        ui.label(RichText::new("Role").strong());
        let prev_role = self.cfg.role;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.cfg.role, Role::Sender, "Sender");
            ui.selectable_value(&mut self.cfg.role, Role::Receiver, "Receiver");
        });
        if prev_role != self.cfg.role {
            self.refresh_monitors();
        }
        ui.add_space(10.0);

        // QUALITY --------------------------------------------------------
        ui.label(RichText::new("Quality").strong());
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.cfg.quality, Quality::Text, "📝 Text / code")
                .on_hover_text("30 fps, ~12 Mbps, low latency. Optimised for static screens and typing.");
            ui.selectable_value(&mut self.cfg.quality, Quality::Video, "🎬 Video / motion")
                .on_hover_text("60 fps, ~20 Mbps, smooth playback. Higher latency.");
        });
        ui.label(RichText::new(format!(
            "→ {} fps, {} kbps",
            self.cfg.quality.framerate(),
            self.cfg.quality.bitrate_kbps()
        )).weak());
        ui.add_space(10.0);

        // CODEC ----------------------------------------------------------
        ui.horizontal(|ui| {
            ui.label(RichText::new("Codec").strong());
            ComboBox::from_id_salt("codec")
                .selected_text(format!("{:?}", self.cfg.codec))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cfg.codec, Codec::H265, "H.265 (HEVC)");
                    ui.selectable_value(&mut self.cfg.codec, Codec::H264, "H.264 (AVC)");
                });
        });
        ui.add_space(10.0);

        // PEERS (sender) or LISTEN PORT (receiver) -----------------------
        if self.cfg.role == Role::Sender {
            ui.label(RichText::new("Peers").strong());
            ui.label(RichText::new(
                "Saved destinations. The selected one is where this sender pushes."
            ).weak());
            ui.add_space(4.0);

            let mut remove: Option<usize> = None;
            for (i, peer) in self.cfg.peers.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut selected = self.cfg.active_peer == i;
                    if ui.radio(selected, "").clicked() {
                        selected = true;
                    }
                    if selected {
                        self.cfg.active_peer = i;
                    }
                    let label = if peer.label.is_empty() { &peer.address } else { &peer.label };
                    ui.label(label);
                    if !peer.label.is_empty() {
                        ui.label(RichText::new(format!("  ({})", peer.address)).weak());
                    }
                    if ui.small_button("✕").on_hover_text("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.cfg.peers.remove(i);
                if self.cfg.active_peer >= self.cfg.peers.len() && !self.cfg.peers.is_empty() {
                    self.cfg.active_peer = self.cfg.peers.len() - 1;
                }
            }
            if self.cfg.peers.is_empty() {
                ui.label(RichText::new("(no peers yet — add one below)").weak().color(Color32::DARK_GRAY));
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Add a peer:").weak());
            ui.horizontal(|ui| {
                ui.add(eframe::egui::TextEdit::singleline(&mut self.new_peer_label)
                    .hint_text("label (optional)")
                    .desired_width(140.0));
                ui.add(eframe::egui::TextEdit::singleline(&mut self.new_peer_addr)
                    .hint_text("host:port")
                    .desired_width(200.0));
                if ui.button("➕ Add").clicked() {
                    let addr = self.new_peer_addr.trim().to_string();
                    if addr.is_empty() || !addr.contains(':') {
                        self.error = Some("Peer must be host:port".into());
                    } else {
                        self.cfg.peers.push(Peer {
                            label: self.new_peer_label.trim().to_string(),
                            address: addr,
                        });
                        if self.cfg.peers.len() == 1 {
                            self.cfg.active_peer = 0;
                        }
                        self.new_peer_label.clear();
                        self.new_peer_addr.clear();
                        self.error = None;
                    }
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Listen port").strong());
                let mut p = self.cfg.listen_port as i64;
                if ui.add(eframe::egui::DragValue::new(&mut p).range(1..=65535)).changed() {
                    self.cfg.listen_port = p as u16;
                }
            });
        }
        ui.add_space(10.0);

        // MONITOR --------------------------------------------------------
        ui.label(RichText::new("Monitor").strong());
        match self.monitors.source {
            monitor::Source::None => {
                if cfg!(target_os = "linux") && self.cfg.role == Role::Sender {
                    ui.label(RichText::new(
                        "Linux sender: the xdg-desktop-portal pops a chooser \
                         each time it shares — you pick the monitor (and \
                         persist mode remembers it). This field has no effect."
                    ).weak());
                } else if cfg!(target_os = "linux") && self.cfg.role == Role::Receiver {
                    ui.label(RichText::new(
                        "Could not enumerate monitors (no kscreen-doctor or \
                         wlr-randr available). Compositor will place the window."
                    ).weak());
                } else {
                    ui.label(RichText::new(
                        "Monitor enumeration not yet implemented for this OS."
                    ).weak());
                }
                self.cfg.monitor = "auto".into();
            }
            _ => {
                if self.monitors.names.is_empty() {
                    ui.label(RichText::new("(none detected)").weak());
                    self.cfg.monitor = "auto".into();
                } else if self.monitors.user_selectable {
                    ComboBox::from_id_salt("monitor")
                        .selected_text(&self.cfg.monitor)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.cfg.monitor, "auto".into(), "auto");
                            for m in &self.monitors.names {
                                ui.selectable_value(&mut self.cfg.monitor, m.clone(), m);
                            }
                        });
                } else {
                    ui.label(format!("Detected: {}", self.monitors.names.join(", ")));
                    ui.label(RichText::new(
                        "On Linux + role=sender the portal chooses at share time; \
                         on receiver the compositor places the window. Listed for \
                         awareness only."
                    ).weak());
                    self.cfg.monitor = "auto".into();
                }
            }
        }
        ui.add_space(20.0);

        // ERRORS + BUTTONS ----------------------------------------------
        if let Some(err) = &self.error {
            ui.colored_label(Color32::RED, err);
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            if ui.button("Save & Start").clicked() {
                self.try_save_and_take();
            }
            if ui.button("Save & Quit").clicked() {
                if let Err(e) = self.validate() {
                    self.error = Some(e);
                } else if let Err(e) = config::save(&self.cfg) {
                    self.error = Some(format!("save failed: {e}"));
                } else {
                    std::process::exit(0);
                }
            }
            if ui.button("Cancel").clicked() {
                std::process::exit(0);
            }
        });
    }

    fn try_save_and_take(&mut self) {
        if let Err(e) = self.validate() {
            self.error = Some(e);
            return;
        }
        if let Err(e) = config::save(&self.cfg) {
            self.error = Some(format!("save failed: {e}"));
            return;
        }
        self.taken = Some(self.cfg.clone());
    }

    fn validate(&self) -> Result<(), String> {
        if self.cfg.role == Role::Sender {
            if self.cfg.peers.is_empty() {
                return Err("Sender needs at least one peer (add one above).".into());
            }
            if self.cfg.active_peer >= self.cfg.peers.len() {
                return Err("Active peer index out of range.".into());
            }
            for p in &self.cfg.peers {
                let s = p.address.trim();
                let (_h, port) = s.rsplit_once(':').ok_or_else(||
                    format!("Peer `{}` must be host:port", p.address)
                )?;
                if port.parse::<u16>().is_err() {
                    return Err(format!("Peer `{}` has invalid port", p.address));
                }
            }
        }
        Ok(())
    }
}
