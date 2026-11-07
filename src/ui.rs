//! The window: My servers, Catalog, Shared folders, Clients, Secrets, Gateway.

use crate::catalog::{self, Entry};
use crate::clients::{self, Status, CLIENTS};
use crate::gateway;
use crate::oauth;
use crate::state::{self, Runner, Server, State, ToolInfo};
use eframe::egui::{
    self, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense, Stroke, StrokeKind,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// Palette (our own neutral dark theme).
const BG: Color32 = Color32::from_rgb(0x15, 0x18, 0x1d);
const SIDEBAR: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x26);
const CARD: Color32 = Color32::from_rgb(0x22, 0x27, 0x30);
const CARD_HI: Color32 = Color32::from_rgb(0x2a, 0x31, 0x3c);
const LINE: Color32 = Color32::from_rgb(0x33, 0x3a, 0x46);
const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe9, 0xee);
const MUTED: Color32 = Color32::from_rgb(0x95, 0xa0, 0xb0);
const ACCENT: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
const GREEN: Color32 = Color32::from_rgb(0x2f, 0xbf, 0x71);
const AMBER: Color32 = Color32::from_rgb(0xf5, 0xa5, 0x24);
const RED: Color32 = Color32::from_rgb(0xef, 0x5a, 0x5a);
const TEAL: Color32 = Color32::from_rgb(0x22, 0xb8, 0xc8);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Servers,
    Catalog,
    Folders,
    Clients,
    Secrets,
    Gateway,
}

type Slot = Arc<Mutex<Option<Result<Vec<ToolInfo>, String>>>>;

#[derive(Default)]
struct CustomForm {
    open: bool,
    name: String,
    command: String,
    args: String,
    folders: bool,
}

pub struct App {
    state: State,
    catalog: Vec<Entry>,
    page: Page,
    selected: Option<String>,
    search: String,
    category: String,
    toast: Option<(String, Color32, f64)>,
    probes: HashMap<String, (Slot, Arc<Mutex<oauth::Progress>>)>,
    secret_inputs: HashMap<String, String>,
    env_key: String,
    env_val: String,
    custom: CustomForm,
    clients: Vec<clients::ClientState>,
    clients_at: f64,
    log: Vec<String>,
    log_at: f64,
    runtimes: Vec<(&'static str, Option<String>)>,
    screenshot: Option<PathBuf>,
    frame: u32,
    shot_asked: bool,
    persist: bool,
    /// When state.json last changed by us; a different time means the menu-bar icon
    /// changed it, so the window reloads.
    state_mtime: Option<std::time::SystemTime>,
    sync_at: f64,
}

fn state_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(State::path()).and_then(|m| m.modified()).ok()
}

fn page_named(name: &str) -> Page {
    match name {
        "catalog" => Page::Catalog,
        "folders" => Page::Folders,
        "clients" => Page::Clients,
        "secrets" => Page::Secrets,
        "gateway" => Page::Gateway,
        _ => Page::Servers,
    }
}

fn avatar_color(name: &str) -> Color32 {
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    const P: [Color32; 6] = [
        Color32::from_rgb(0x3b, 0x82, 0xf6),
        Color32::from_rgb(0x8b, 0x5c, 0xf6),
        Color32::from_rgb(0x22, 0xb8, 0xc8),
        Color32::from_rgb(0x2f, 0xbf, 0x71),
        Color32::from_rgb(0xf5, 0x9e, 0x0b),
        Color32::from_rgb(0xec, 0x48, 0x99),
    ];
    P[(h % 6) as usize]
}

fn avatar(ui: &mut egui::Ui, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let c = avatar_color(name);
    ui.painter().rect_filled(rect, CornerRadius::same((size * 0.28) as u8), c.gamma_multiply(0.25));
    ui.painter().rect_stroke(rect, CornerRadius::same((size * 0.28) as u8), Stroke::new(1.0_f32, c.gamma_multiply(0.6)), StrokeKind::Inside);
    let letter = name.chars().next().unwrap_or('?').to_uppercase().to_string();
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, letter, FontId::proportional(size * 0.48), c);
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = vec2(34.0, 18.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = if *on { ACCENT } else { LINE };
    ui.painter().rect_filled(rect, CornerRadius::same(9), bg);
    let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 7.0, Color32::WHITE);
    resp
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(ACCENT).corner_radius(6))
}

fn secondary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(TEXT)).fill(CARD_HI).stroke(Stroke::new(1.0_f32, LINE)).corner_radius(6))
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).size(13.0).strong().color(TEXT));
    ui.add_space(2.0);
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> App {
        let ctx = &cc.egui_ctx;
        let mut v = egui::Visuals::dark();
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.extreme_bg_color = Color32::from_rgb(0x11, 0x14, 0x18);
        v.faint_bg_color = CARD;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.5);
        v.widgets.inactive.weak_bg_fill = CARD_HI;
        v.widgets.inactive.bg_fill = CARD_HI;
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x33, 0x3c, 0x4a);
        v.widgets.active.weak_bg_fill = ACCENT;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = vec2(8.0, 7.0);
            s.spacing.button_padding = vec2(12.0, 5.0);
            s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(13.5));
            s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(13.5));
            s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(12.5));
        });

        let persist = screenshot.is_none();
        let catalog = catalog::load();
        let mut app = App {
            state: State::load(),
            catalog,
            page: Page::Servers,
            selected: None,
            search: String::new(),
            category: String::new(),
            toast: None,
            probes: HashMap::new(),
            secret_inputs: HashMap::new(),
            env_key: String::new(),
            env_val: String::new(),
            custom: CustomForm { folders: true, ..Default::default() },
            clients: vec![],
            clients_at: -100.0,
            log: vec![],
            log_at: -100.0,
            runtimes: vec![],
            screenshot,
            frame: 0,
            shot_asked: false,
            persist,
            state_mtime: state_mtime(),
            sync_at: 0.0,
        };
        app.runtimes = ["npx", "uvx", "node"].iter().map(|n| (*n, gateway::find_program(n))).collect();
        if !app.state.seeded {
            if app.state.server_index("filesystem").is_none() {
                if let Some(e) = app.catalog.iter().find(|e| e.name == "filesystem") {
                    let s = e.to_server();
                    app.state.servers.insert(0, s);
                }
            }
            app.state.seeded = true;
            app.save();
        }
        app.selected = std::env::var("MCP_DESK_SELECT")
            .ok()
            .filter(|n| app.state.server_index(n).is_some())
            .or_else(|| app.state.servers.first().map(|s| s.name.clone()));
        app.page = page_named(&std::env::var("MCP_DESK_PAGE").unwrap_or_default());
        if app.persist {
            // Keep a menu-bar icon running, so closing the window doesn't stop anything.
            crate::tray::write_pid("window.pid");
            crate::tray::ensure_running();
        }
        app
    }

    fn save(&mut self) {
        if !self.persist {
            return;
        }
        if let Err(e) = self.state.save() {
            self.notify(&format!("Couldn't save: {}", e), RED);
        }
        self.state_mtime = state_mtime();
    }

    /// Once a second: pick up changes made from the menu-bar icon, and come to the front
    /// when the menu asks for the window.
    fn sync_with_menu(&mut self, ctx: &egui::Context, now: f64) {
        if !self.persist || now - self.sync_at < 1.0 {
            return;
        }
        self.sync_at = now;
        let m = state_mtime();
        if m != self.state_mtime {
            self.state = State::load();
            self.state_mtime = m;
            if self.selected.as_ref().and_then(|n| self.state.server_index(n)).is_none() {
                self.selected = self.state.servers.first().map(|s| s.name.clone());
            }
        }
        let req = state::data_dir().join("show-request");
        if let Ok(page) = std::fs::read_to_string(&req) {
            let _ = std::fs::remove_file(&req);
            self.page = page_named(page.trim());
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn notify(&mut self, text: &str, color: Color32) {
        self.toast = Some((text.to_string(), color, 0.0));
    }

    fn total_tools(&self) -> usize {
        self.state
            .servers
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.known_tools.iter().filter(|t| !s.disabled_tools.contains(&t.name)).count())
            .sum()
    }

    fn start_probe(&mut self, name: &str) {
        let Some(i) = self.state.server_index(name) else { return };
        let srv = self.state.servers[i].clone();
        let st = self.state.clone();
        let slot: Slot = Arc::new(Mutex::new(None));
        let progress = Arc::new(Mutex::new(oauth::Progress::default()));
        let (s2, p2) = (slot.clone(), progress.clone());
        std::thread::spawn(move || {
            let r = gateway::probe(&srv, &st, p2);
            *s2.lock().unwrap() = Some(r);
        });
        self.probes.insert(name.to_string(), (slot, progress));
    }

    fn poll_probes(&mut self) {
        let mut done = vec![];
        for (name, (slot, _)) in &self.probes {
            if let Some(r) = slot.lock().unwrap().take() {
                done.push((name.clone(), r));
            }
        }
        for (name, r) in done {
            self.probes.remove(&name);
            match r {
                Ok(tools) => {
                    let n = tools.len();
                    if let Some(i) = self.state.server_index(&name) {
                        self.state.servers[i].known_tools = tools;
                    }
                    self.save();
                    self.notify(&format!("{}: {} tools found", name, n), GREEN);
                }
                Err(e) => self.notify(&format!("{}: {}", name, e), RED),
            }
        }
    }

    fn add_folders(&mut self, paths: Vec<PathBuf>) {
        let mut n = 0;
        for p in paths {
            let p = if p.is_dir() { p } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) };
            if self.state.add_folder(&p.display().to_string()) {
                n += 1;
            }
        }
        if n > 0 {
            self.save();
            self.notify(&format!("Added {} folder{} — every file server can use it now", n, if n == 1 { "" } else { "s" }), GREEN);
        }
    }

    // ------------------------------------------------------------------ chrome

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .exact_height(54.0)
            .frame(Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(16, 0)).stroke(Stroke::new(1.0_f32, LINE)))
            .show(ctx, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect, CornerRadius::same(7), ACCENT);
                    for (i, dx) in [-7.0f32, 0.0, 7.0].iter().enumerate() {
                        let c = rect.center() + vec2(*dx, if i == 1 { -4.0 } else { 4.0 });
                        p.circle_filled(c, 3.2, Color32::WHITE);
                    }
                    p.line_segment([rect.center() + vec2(-7.0, 4.0), rect.center() + vec2(0.0, -4.0)], Stroke::new(1.6_f32, Color32::WHITE));
                    p.line_segment([rect.center() + vec2(0.0, -4.0), rect.center() + vec2(7.0, 4.0)], Stroke::new(1.6_f32, Color32::WHITE));
                    ui.add_space(4.0);
                    ui.label(RichText::new("MCP Gateway Desk").size(16.0).strong());
                    ui.label(RichText::new("servers · folders · clients").size(12.0).color(MUTED));
                    let w = ui.available_width();
                    ui.allocate_ui_with_layout(vec2(w, 22.0), Layout::right_to_left(Align::Center), |ui| {
                        let enabled = self.state.servers.iter().filter(|s| s.enabled).count();
                        let connected = self.clients.iter().filter(|c| c.status == Status::Connected).count();
                        chip(ui, &format!("{} app{} connected", connected, if connected == 1 { "" } else { "s" }), if connected > 0 { GREEN } else { MUTED });
                        chip(ui, &format!("{} tools", self.total_tools()), TEAL);
                        chip(ui, &format!("{} servers on", enabled), ACCENT);
                        chip(ui, &format!("{} shared folders", self.state.folders.len()), AMBER);
                    });
                });
            });
    }

    fn side_bar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .exact_width(212.0)
            .resizable(false)
            .frame(Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(10, 14)).stroke(Stroke::new(1.0_f32, LINE)))
            .show(ctx, |ui| {
                ui.label(RichText::new("MCP TOOLKIT").size(11.0).color(MUTED).strong());
                ui.add_space(4.0);
                let items = [
                    (Page::Servers, "🗄", "My servers"),
                    (Page::Catalog, "📚", "Catalog"),
                    (Page::Folders, "📁", "Shared folders"),
                    (Page::Clients, "🔌", "Clients"),
                    (Page::Secrets, "🔑", "Secrets"),
                    (Page::Gateway, "⚡", "Gateway"),
                ];
                for (page, icon, label) in items {
                    let active = self.page == page;
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    let fill = if active { ACCENT.gamma_multiply(0.22) } else if resp.hovered() { CARD } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
                    if active {
                        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min + vec2(0.0, 7.0), vec2(3.0, 20.0)), CornerRadius::same(2), ACCENT);
                    }
                    let col = if active { TEXT } else { MUTED };
                    ui.painter().text(rect.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(14.0), col);
                    ui.painter().text(rect.left_center() + vec2(40.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(14.0), col);
                    let badge = match page {
                        Page::Servers => Some(self.state.servers.len()),
                        Page::Folders => Some(self.state.folders.len()),
                        Page::Catalog => Some(self.catalog.len()),
                        _ => None,
                    };
                    if let Some(n) = badge {
                        ui.painter().text(rect.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, n.to_string(), FontId::proportional(12.0), MUTED);
                    }
                    if resp.clicked() {
                        self.page = page;
                    }
                }
                ui.add_space((ui.available_height() - 104.0).max(8.0));
                Frame::new().fill(CARD).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("No volume mounting").strong().size(12.5));
                    ui.label(RichText::new("Every server runs right on this Mac and opens your shared folders directly. No containers, nothing to mount.").size(11.5).color(MUTED));
                });
            });
    }

    // ------------------------------------------------------------------ pages

    fn page_header(ui: &mut egui::Ui, title: &str, sub: &str) {
        ui.label(RichText::new(title).size(22.0).strong());
        ui.label(RichText::new(sub).color(MUTED));
        ui.add_space(8.0);
    }

    fn servers_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| Self::page_header(ui, "My servers", "The MCP servers your AI apps can use through the gateway."));
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                if primary(ui, "+ Add from catalog").clicked() {
                    self.page = Page::Catalog;
                }
                if secondary(ui, "+ Custom server").clicked() {
                    self.custom.open = true;
                }
            });
        });

        if self.custom.open {
            self.custom_form(ui);
            ui.add_space(8.0);
        }

        if self.state.servers.is_empty() {
            card(ui, |ui| {
                ui.label("No servers yet. Add one from the catalog or add your own command.");
            });
            return;
        }

        let avail = ui.available_size();
        ui.horizontal_top(|ui| {
            // List
            ui.allocate_ui_with_layout(vec2(300.0, avail.y), Layout::top_down(Align::LEFT), |ui| {
                egui::ScrollArea::vertical().id_salt("srv_list").auto_shrink([false, false]).show(ui, |ui| {
                    let mut changed = false;
                    let mut pick = None;
                    for s in self.state.servers.iter_mut() {
                        let sel = self.selected.as_deref() == Some(&s.name);
                        let resp = Frame::new()
                            .fill(if sel { CARD_HI } else { CARD })
                            .stroke(Stroke::new(1.0_f32, if sel { ACCENT } else { LINE }))
                            .corner_radius(CornerRadius::same(9))
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    avatar(ui, &s.title, 32.0);
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&s.title).strong());
                                        ui.horizontal(|ui| {
                                            let c = if matches!(s.runner, Runner::Local { .. }) { GREEN } else { ACCENT };
                                            chip(ui, s.runner.label(), c);
                                            if s.uses_folders {
                                                chip(ui, "📁 files", AMBER);
                                            }
                                        });
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if toggle(ui, &mut s.enabled).changed() {
                                            changed = true;
                                        }
                                    });
                                });
                            })
                            .response;
                        if resp.interact(Sense::click()).clicked() {
                            pick = Some(s.name.clone());
                        }
                        ui.add_space(2.0);
                    }
                    if let Some(p) = pick {
                        self.selected = Some(p);
                    }
                    if changed {
                        self.save();
                    }
                });
            });
            ui.add_space(6.0);
            // Details
            ui.allocate_ui_with_layout(vec2(ui.available_width(), avail.y), Layout::top_down(Align::LEFT), |ui| {
                egui::ScrollArea::vertical().id_salt("srv_detail").auto_shrink([false, false]).show(ui, |ui| {
                    self.server_details(ui);
                });
            });
        });
    }

    fn custom_form(&mut self, ui: &mut egui::Ui) {
        let mut add = false;
        let mut cancel = false;
        card(ui, |ui| {
            ui.label(RichText::new("Add your own server").strong().size(15.0));
            ui.label(RichText::new("Any command that speaks MCP over stdio, e.g. npx -y some-mcp-server, uvx tool, or a program on this Mac.").color(MUTED));
            egui::Grid::new("custom").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut self.custom.name).hint_text("my-server").desired_width(320.0));
                ui.end_row();
                ui.label("Command");
                ui.add(egui::TextEdit::singleline(&mut self.custom.command).hint_text("npx").desired_width(320.0));
                ui.end_row();
                ui.label("Arguments");
                ui.add(egui::TextEdit::singleline(&mut self.custom.args).hint_text("-y @scope/server {{folders}}").desired_width(320.0));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.custom.folders, "Give it my shared folders ({{folders}} becomes the folder list)");
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if primary(ui, "Add server").clicked() {
                    add = true;
                }
                if secondary(ui, "Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if add {
            let name: String = self.custom.name.trim().replace(' ', "-").to_lowercase();
            if name.is_empty() || self.custom.command.trim().is_empty() {
                self.notify("Give it a name and a command", AMBER);
            } else if self.state.server_index(&name).is_some() {
                self.notify("A server with that name already exists", AMBER);
            } else {
                let args: Vec<String> = self.custom.args.split_whitespace().map(String::from).collect();
                self.state.servers.push(Server {
                    name: name.clone(),
                    title: self.custom.name.trim().to_string(),
                    description: "Custom server".into(),
                    category: "custom".into(),
                    runner: Runner::Local { command: self.custom.command.trim().to_string(), args },
                    enabled: true,
                    uses_folders: self.custom.folders,
                    env: Default::default(),
                    config: Default::default(),
                    secrets: vec![],
                    known_tools: vec![],
                    disabled_tools: Default::default(),
                });
                self.selected = Some(name);
                self.custom = CustomForm { folders: true, ..Default::default() };
                self.save();
                self.notify("Server added", GREEN);
            }
        }
        if cancel {
            self.custom.open = false;
        }
    }

    fn server_details(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.selected.as_ref().and_then(|n| self.state.server_index(n)) else {
            card(ui, |ui| ui.label("Pick a server on the left."));
            return;
        };
        let mut dirty = false;
        let mut remove = false;
        let mut probe = false;
        let mut open_folders = false;
        let probing = self.probes.contains_key(&self.state.servers[i].name);
        let progress = self.probes.get(&self.state.servers[i].name).map(|(_, p)| p.lock().unwrap().clone());
        let remote = oauth::remote_url(&self.state.servers[i]);
        let signed_in = remote.as_deref().map(oauth::signed_in).unwrap_or(false);
        let mut sign_out = false;
        let mut stop_stuck = false;
        let preview = gateway::command_line(&self.state.servers[i], &self.state);
        let folders: Vec<state::Folder> = self.state.folders.clone();
        let secret_names = self.state.secret_names.clone();
        let mut secret_save: Option<(String, String)> = None;

        let srv = &mut self.state.servers[i];
        card(ui, |ui| {
            ui.horizontal(|ui| {
                avatar(ui, &srv.title, 44.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&srv.title).size(18.0).strong());
                    ui.label(RichText::new(&srv.name).color(MUTED).monospace());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Remove").color(RED)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0_f32, RED.gamma_multiply(0.5)))).clicked() {
                        remove = true;
                    }
                    if toggle(ui, &mut srv.enabled).changed() {
                        dirty = true;
                    }
                    ui.label(RichText::new(if srv.enabled { "On" } else { "Off" }).color(MUTED));
                });
            });
            if !srv.description.is_empty() {
                ui.label(RichText::new(srv.description.lines().next().unwrap_or("")).color(MUTED));
            }
        });
        ui.add_space(6.0);

        // Sign-in (OAuth) for remote servers
        if let Some(url) = &remote {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    section(ui, "Sign-in (OAuth)");
                    if signed_in {
                        chip(ui, "✔ Signed in", GREEN);
                    } else {
                        chip(ui, "Not signed in", AMBER);
                    }
                });
                ui.label(RichText::new(format!("This server lives at {} and needs you to sign in once in your browser. The sign-in is saved on this Mac, and your AI apps use it from then on.", url)).color(MUTED));
                if !signed_in && !probing {
                    ui.label(RichText::new("Until you sign in, the gateway skips this server, so your AI apps never wait for it.").size(12.0).color(MUTED));
                }
                if let Some(p) = &progress {
                    if p.other_signin {
                        ui.label(RichText::new("⚠ Another sign-in for this server is still open (maybe started by an AI app). Finish it in your browser, or stop it and try again.").color(AMBER));
                    } else if let Some(auth) = &p.auth_url {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label(RichText::new("Waiting for you to finish signing in in your browser…").color(TEXT));
                            if primary(ui, "Open sign-in page").clicked() {
                                let _ = std::process::Command::new("open").arg(auth).spawn();
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label(RichText::new("Connecting…").color(MUTED));
                        });
                    }
                }
                ui.horizontal(|ui| {
                    if !probing && !signed_in && primary(ui, "Sign in").clicked() {
                        probe = true;
                    }
                    if signed_in && secondary(ui, "Sign out").clicked() {
                        sign_out = true;
                    }
                    if secondary(ui, "Stop stuck sign-ins").on_hover_text("Stops every running copy of this server's bridge. Use it when sign-in or Check tools waits forever.").clicked() {
                        stop_stuck = true;
                    }
                });
            });
            ui.add_space(6.0);
        }

        // How it runs
        card(ui, |ui| {
            section(ui, "How it runs");
            ui.horizontal(|ui| {
                chip(ui, "On this Mac — opens your files directly", GREEN);
            });
            let Runner::Local { command, args } = &mut srv.runner;
            ui.horizontal(|ui| {
                ui.label("Command");
                if ui.add(egui::TextEdit::singleline(command).desired_width(160.0)).changed() {
                    dirty = true;
                }
                ui.label("Arguments");
                let mut joined = args.join(" ");
                if ui.add(egui::TextEdit::singleline(&mut joined).desired_width(ui.available_width())).changed() {
                    *args = joined.split_whitespace().map(String::from).collect();
                    dirty = true;
                }
            });
            let shown: Vec<String> = preview.1.iter().map(|a| if a.contains(' ') { format!("\"{}\"", a) } else { a.clone() }).collect();
            Frame::new().fill(Color32::from_rgb(0x11, 0x14, 0x18)).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("$ {} {}", preview.0, shown.join(" "))).monospace().color(Color32::from_rgb(0xa5, 0xd6, 0xff)));
            });
        });
        ui.add_space(6.0);

        // Files
        card(ui, |ui| {
            section(ui, "Files and folders");
            if ui.checkbox(&mut srv.uses_folders, "Give this server my shared folders").changed() {
                dirty = true;
            }
            if srv.uses_folders {
                if folders.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⚠ No shared folders yet — this server has nothing to work with.").color(AMBER));
                        if secondary(ui, "Add folders…").clicked() {
                            open_folders = true;
                        }
                    });
                } else {
                    let how = if matches!(srv.runner, Runner::Local { .. }) { "opened directly, no mount" } else { "mounted at the same path" };
                    for f in &folders {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("📁").color(AMBER));
                            ui.label(RichText::new(&f.path).monospace());
                            chip(ui, if f.read_only { "read-only" } else { "read & write" }, if f.read_only { MUTED } else { GREEN });
                            ui.label(RichText::new(how).size(11.5).color(MUTED));
                        });
                    }
                }
            }
        });
        ui.add_space(6.0);

        // Settings, env, secrets
        if !srv.config.is_empty() || !srv.secrets.is_empty() || !srv.env.is_empty() || true {
            card(ui, |ui| {
                section(ui, "Configuration");
                egui::Grid::new("cfg").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                    for (k, v) in srv.config.iter_mut() {
                        ui.label(RichText::new(k).monospace());
                        if ui.add(egui::TextEdit::singleline(v).desired_width(340.0)).changed() {
                            dirty = true;
                        }
                        ui.end_row();
                    }
                    let mut drop_key = None;
                    for (k, v) in srv.env.iter_mut() {
                        ui.label(RichText::new(format!("env {}", k)).monospace());
                        ui.horizontal(|ui| {
                            if ui.add(egui::TextEdit::singleline(v).desired_width(310.0)).changed() {
                                dirty = true;
                            }
                            if ui.small_button("✕").clicked() {
                                drop_key = Some(k.clone());
                            }
                        });
                        ui.end_row();
                    }
                    if let Some(k) = drop_key {
                        srv.env.remove(&k);
                        dirty = true;
                    }
                    for sref in &srv.secrets {
                        let set = secret_names.contains(&sref.name);
                        ui.label(RichText::new(format!("🔑 {}", sref.env)).monospace());
                        ui.horizontal(|ui| {
                            let buf = self.secret_inputs.entry(sref.name.clone()).or_default();
                            ui.add(egui::TextEdit::singleline(buf).password(true).hint_text(if set { "•••••• saved in Keychain" } else { "paste value" }).desired_width(240.0));
                            if secondary(ui, "Save").clicked() && !buf.is_empty() {
                                secret_save = Some((sref.name.clone(), std::mem::take(buf)));
                            }
                            chip(ui, if set { "set" } else { "missing" }, if set { GREEN } else { AMBER });
                        });
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.env_key).hint_text("NEW_VARIABLE").desired_width(150.0));
                    ui.add(egui::TextEdit::singleline(&mut self.env_val).hint_text("value").desired_width(200.0));
                    if secondary(ui, "+ Add variable").clicked() && !self.env_key.trim().is_empty() {
                        srv.env.insert(self.env_key.trim().to_string(), std::mem::take(&mut self.env_val));
                        self.env_key.clear();
                        dirty = true;
                    }
                });
            });
            ui.add_space(6.0);
        }

        // Tools
        card(ui, |ui| {
            ui.horizontal(|ui| {
                section(ui, &format!("Tools ({})", srv.known_tools.len()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if probing {
                        ui.add(egui::Spinner::new());
                        let waiting = progress.as_ref().map(|p| p.auth_url.is_some()).unwrap_or(false);
                        ui.label(RichText::new(if waiting { "waiting for sign-in…" } else { "starting server…" }).color(MUTED));
                    } else if primary(ui, "Check tools").clicked() {
                        probe = true;
                    }
                });
            });
            if srv.known_tools.is_empty() {
                ui.label(RichText::new("Press “Check tools” to start the server once and list what it can do.").color(MUTED));
            }
            for t in &srv.known_tools {
                let mut on = !srv.disabled_tools.contains(&t.name);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, RichText::new(&t.name).monospace()).changed() {
                        if on {
                            srv.disabled_tools.remove(&t.name);
                        } else {
                            srv.disabled_tools.insert(t.name.clone());
                        }
                        dirty = true;
                    }
                    let d: String = t.description.chars().take(110).collect();
                    ui.label(RichText::new(d).size(12.0).color(MUTED));
                });
            }
        });

        let name = srv.name.clone();
        if let Some((sname, value)) = secret_save {
            match state::secret_set(&sname, &value) {
                Ok(()) => {
                    self.state.secret_names.insert(sname);
                    dirty = true;
                    self.notify("Secret saved in your Keychain", GREEN);
                }
                Err(e) => self.notify(&format!("Keychain: {}", e), RED),
            }
        }
        if probe {
            self.start_probe(&name);
        }
        if let Some(url) = &remote {
            if sign_out {
                oauth::sign_out(url);
                self.notify("Signed out — the gateway skips this server until you sign in again", AMBER);
            }
            if stop_stuck {
                let n = oauth::stop_other_signins(url);
                self.notify(&format!("Stopped {} running cop{} — press Sign in to start fresh", n, if n == 1 { "y" } else { "ies" }), GREEN);
            }
        }
        if open_folders {
            if let Some(p) = rfd::FileDialog::new().pick_folders() {
                self.add_folders(p);
            }
        }
        if remove {
            self.state.servers.remove(i);
            self.selected = self.state.servers.first().map(|s| s.name.clone());
            dirty = true;
        }
        if dirty {
            self.save();
        }
    }

    fn catalog_page(&mut self, ui: &mut egui::Ui) {
        Self::page_header(ui, "Catalog", "MCP servers that run straight on this Mac — no Docker, no containers, no mounts.");
        let mut cats: Vec<String> = self.catalog.iter().map(|e| e.category.clone()).filter(|c| !c.is_empty()).collect();
        cats.sort();
        cats.dedup();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Search servers").desired_width(320.0));
            egui::ComboBox::from_id_salt("cat")
                .selected_text(if self.category.is_empty() { "All categories".to_string() } else { self.category.clone() })
                .width(170.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category, String::new(), "All categories");
                    ui.selectable_value(&mut self.category, "__remote".to_string(), "Remote (hosted)");
                    ui.selectable_value(&mut self.category, "__files".to_string(), "Works with files");
                    for c in &cats {
                        ui.selectable_value(&mut self.category, c.clone(), c);
                    }
                });
        });
        ui.add_space(6.0);
        let q = self.search.to_lowercase();
        let list: Vec<usize> = self
            .catalog
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (q.is_empty() || e.title.to_lowercase().contains(&q) || e.name.contains(&q) || e.description.to_lowercase().contains(&q))
                    && (self.category.is_empty()
                        || (self.category == "__remote" && e.kind == "remote")
                        || (self.category == "__files" && e.uses_folders)
                        || e.category == self.category)
            })
            .map(|(i, _)| i)
            .collect();
        ui.label(RichText::new(format!("{} servers", list.len())).color(MUTED));
        let mut add: Option<usize> = None;
        egui::ScrollArea::vertical().id_salt("cat_list").auto_shrink([false, false]).show_rows(ui, 74.0, list.len(), |ui, range| {
            for &idx in &list[range] {
                let e = &self.catalog[idx];
                let added = self.state.server_index(&e.name).is_some();
                Frame::new().fill(CARD).stroke(Stroke::new(1.0_f32, LINE)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(48.0);
                    ui.horizontal(|ui| {
                        avatar(ui, &e.title, 36.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&e.title).strong());
                                if e.kind == "remote" {
                                    chip(ui, "Remote", TEAL);
                                }
                                if e.uses_folders {
                                    chip(ui, "📁 files", AMBER);
                                }
                                if !e.secrets.is_empty() {
                                    chip(ui, "🔑 key", MUTED);
                                }
                            });
                            let d: String = e.description.lines().next().unwrap_or("").chars().take(120).collect();
                            ui.label(RichText::new(d).size(12.0).color(MUTED));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if added {
                                ui.label(RichText::new("✔ Added").color(GREEN));
                            } else if primary(ui, "Add").clicked() {
                                add = Some(idx);
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
        if let Some(idx) = add {
            let srv = self.catalog[idx].to_server();
            let name = srv.name.clone();
            let title = srv.title.clone();
            self.state.servers.push(srv);
            self.selected = Some(name);
            self.save();
            self.notify(&format!("{} added to My servers", title), GREEN);
        }
    }

    fn folders_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::page_header(ui, "Shared folders", "One list of folders for every server that works with files. Add a folder once — no volume mounting, no restarting containers by hand.");
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 110.0), Sense::click());
        let fill = if hovering { ACCENT.gamma_multiply(0.18) } else if resp.hovered() { CARD_HI } else { CARD };
        ui.painter().rect_filled(rect, CornerRadius::same(12), fill);
        ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.5_f32, if hovering { ACCENT } else { LINE }), StrokeKind::Inside);
        ui.painter().text(rect.center() - vec2(0.0, 18.0), Align2::CENTER_CENTER, "📁", FontId::proportional(26.0), AMBER);
        ui.painter().text(rect.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, "Drop folders here from Finder, or click to choose", FontId::proportional(15.0), TEXT);
        ui.painter().text(rect.center() + vec2(0.0, 32.0), Align2::CENTER_CENTER, "They're available to file servers right away", FontId::proportional(12.0), MUTED);
        if resp.clicked() {
            if let Some(p) = rfd::FileDialog::new().pick_folders() {
                self.add_folders(p);
            }
        }
        ui.add_space(10.0);

        let mut remove = None;
        let mut dirty = false;
        if self.state.folders.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new("No folders shared yet.").color(MUTED));
            });
        }
        for (i, f) in self.state.folders.iter_mut().enumerate() {
            let exists = std::path::Path::new(&f.path).is_dir();
            Frame::new().fill(CARD).stroke(Stroke::new(1.0_f32, LINE)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📁").size(18.0).color(AMBER));
                    ui.vertical(|ui| {
                        let name = std::path::Path::new(&f.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(f.path.clone());
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(&f.path).monospace().size(12.0).color(MUTED));
                    });
                    if !exists {
                        chip(ui, "missing", RED);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                        if ui.small_button("Show in Finder").clicked() {
                            let _ = std::process::Command::new("open").arg(&f.path).spawn();
                        }
                        let mut rw = !f.read_only;
                        if toggle(ui, &mut rw).changed() {
                            f.read_only = !rw;
                            dirty = true;
                        }
                        ui.label(RichText::new(if rw { "Can change files" } else { "Read only" }).color(MUTED));
                    });
                });
            });
            ui.add_space(4.0);
        }
        if let Some(i) = remove {
            self.state.folders.remove(i);
            dirty = true;
        }
        if dirty {
            self.save();
        }

        ui.add_space(10.0);
        card(ui, |ui| {
            section(ui, "Servers that see these folders");
            let users: Vec<&Server> = self.state.servers.iter().filter(|s| s.uses_folders).collect();
            if users.is_empty() {
                ui.label(RichText::new("None yet — add the Filesystem server from the catalog.").color(MUTED));
            }
            for s in users {
                ui.horizontal(|ui| {
                    avatar(ui, &s.title, 22.0);
                    ui.label(&s.title);
                    chip(ui, "direct access on this Mac", GREEN);
                    if !s.enabled {
                        chip(ui, "off", MUTED);
                    }
                });
            }
        });
    }

    fn clients_page(&mut self, ui: &mut egui::Ui) {
        Self::page_header(ui, "Clients", "Connect your AI apps to the gateway. One entry gives them every server you turned on.");
        let mut action: Option<(usize, bool)> = None;
        let width = ui.available_width();
        let cols = 3usize;
        let w = (width - 12.0 * (cols as f32 - 1.0)) / cols as f32 - 4.0;
        egui::ScrollArea::vertical().id_salt("clients").auto_shrink([false, false]).show(ui, |ui| {
            for (row, chunk) in CLIENTS.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    for (j, c) in chunk.iter().enumerate() {
                        let i = row * cols + j;
                        let st = self.clients.get(i);
                        ui.allocate_ui_with_layout(vec2(w, 118.0), Layout::top_down(Align::LEFT), |ui| {
                            Frame::new().fill(CARD).stroke(Stroke::new(1.0_f32, LINE)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
                                ui.set_width(w - 26.0);
                                ui.set_height(92.0);
                                ui.horizontal(|ui| {
                                    avatar(ui, c.name, 30.0);
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(c.name).strong());
                                        let (t, col) = match st.map(|s| &s.status) {
                                            Some(Status::Connected) => ("Connected", GREEN),
                                            Some(Status::Installed) => ("Installed", ACCENT),
                                            _ => ("Not found", MUTED),
                                        };
                                        ui.horizontal(|ui| {
                                            chip(ui, t, col);
                                        });
                                    });
                                });
                                if let Some(s) = st {
                                    let p = s.config_path.display().to_string().replace(&state::home().display().to_string(), "~");
                                    ui.add(egui::Label::new(RichText::new(p).size(11.0).color(MUTED).monospace()).truncate());
                                    ui.horizontal(|ui| match s.status {
                                        Status::Connected => {
                                            if secondary(ui, "Disconnect").clicked() {
                                                action = Some((i, false));
                                            }
                                        }
                                        _ => {
                                            if primary(ui, "Connect").clicked() {
                                                action = Some((i, true));
                                            }
                                        }
                                    });
                                }
                            });
                        });
                    }
                });
                ui.add_space(4.0);
            }
            ui.add_space(12.0);
            card(ui, |ui| {
                section(ui, "Other apps — add this by hand");
                let snip = clients::manual_snippet();
                ui.label(RichText::new(&snip).monospace().color(Color32::from_rgb(0xa5, 0xd6, 0xff)));
                if secondary(ui, "Copy").clicked() {
                    ui.ctx().copy_text(snip);
                    self.toast = Some(("Copied".into(), GREEN, 0.0));
                }
            });
        });
        if let Some((i, add)) = action {
            let c = &CLIENTS[i];
            let r = if add { c.connect() } else { c.disconnect() };
            match r {
                Ok(()) => self.notify(&format!("{} {} — restart it to pick up the change", c.name, if add { "connected" } else { "disconnected" }), GREEN),
                Err(e) => self.notify(&format!("{}: {}", c.name, e), RED),
            }
            self.clients_at = -100.0;
        }
    }

    fn secrets_page(&mut self, ui: &mut egui::Ui) {
        Self::page_header(ui, "Secrets", "API keys are kept in your macOS Keychain and handed to servers when they start.");
        let mut wanted: Vec<(String, String, String)> = vec![];
        for s in &self.state.servers {
            for r in &s.secrets {
                wanted.push((r.name.clone(), r.env.clone(), s.title.clone()));
            }
        }
        for n in &self.state.secret_names {
            if !wanted.iter().any(|w| &w.0 == n) {
                wanted.push((n.clone(), String::new(), "—".into()));
            }
        }
        let mut save: Option<(String, String)> = None;
        let mut delete: Option<String> = None;
        card(ui, |ui| {
            if wanted.is_empty() {
                ui.label(RichText::new("None of your servers needs a secret.").color(MUTED));
                return;
            }
            egui::Grid::new("secrets").num_columns(5).striped(true).spacing(vec2(14.0, 8.0)).show(ui, |ui| {
                for h in ["Secret", "Variable", "Used by", "Value", ""] {
                    ui.label(RichText::new(h).color(MUTED).size(12.0));
                }
                ui.end_row();
                for (name, env, by) in &wanted {
                    let set = self.state.secret_names.contains(name);
                    ui.label(RichText::new(name).monospace());
                    ui.label(RichText::new(env).monospace().color(MUTED));
                    ui.label(by);
                    let buf = self.secret_inputs.entry(name.clone()).or_default();
                    ui.add(egui::TextEdit::singleline(buf).password(true).hint_text(if set { "•••••• saved" } else { "not set" }).desired_width(220.0));
                    ui.horizontal(|ui| {
                        if secondary(ui, "Save").clicked() && !buf.is_empty() {
                            save = Some((name.clone(), std::mem::take(buf)));
                        }
                        if set && ui.small_button("Delete").clicked() {
                            delete = Some(name.clone());
                        }
                    });
                    ui.end_row();
                }
            });
        });
        if let Some((n, v)) = save {
            match state::secret_set(&n, &v) {
                Ok(()) => {
                    self.state.secret_names.insert(n);
                    self.save();
                    self.notify("Saved in Keychain", GREEN);
                }
                Err(e) => self.notify(&format!("Keychain: {}", e), RED),
            }
        }
        if let Some(n) = delete {
            state::secret_delete(&n);
            self.state.secret_names.remove(&n);
            self.save();
        }
    }

    fn gateway_page(&mut self, ui: &mut egui::Ui) {
        Self::page_header(ui, "Gateway", "Clients start the gateway; it starts your servers and passes every tool call through.");
        let mut dirty = false;
        let mut test_all = false;
        egui::ScrollArea::vertical().id_salt("gw").auto_shrink([false, false]).show(ui, |ui| {
            ui.columns(2, |cols| {
                card(&mut cols[0], |ui| {
                    section(ui, "What's on this Mac");
                    for (name, path) in &self.runtimes {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                            if path.is_some() {
                                ui.painter().circle_filled(r.center(), 4.5, GREEN);
                            } else {
                                ui.painter().circle_stroke(r.center(), 4.5, Stroke::new(1.2_f32, MUTED));
                            }
                            ui.label(RichText::new(*name).monospace().strong());
                            ui.label(RichText::new(path.clone().unwrap_or_else(|| "not found".into())).size(12.0).color(MUTED));
                        });
                    }
                    ui.label(RichText::new("npx and uvx start the servers right on this Mac. Docker isn't used.").size(12.0).color(MUTED));
                    ui.add_space(6.0);
                    section(ui, "Servers");
                    let on = self.state.servers.iter().filter(|s| s.enabled).count();
                    ui.label(format!("{} of {} servers on · {} tools known", on, self.state.servers.len(), self.total_tools()));
                    ui.horizontal(|ui| {
                        if primary(ui, "Test all servers").clicked() {
                            test_all = true;
                        }
                        if !self.probes.is_empty() {
                            ui.add(egui::Spinner::new());
                            ui.label(RichText::new(format!("{} running…", self.probes.len())).color(MUTED));
                        }
                    });
                });
                card(&mut cols[1], |ui| {
                    section(ui, "Options");
                    let s = &mut self.state.settings;
                    dirty |= ui.checkbox(&mut s.log_calls, "Log tool calls").changed();
                    ui.add_space(4.0);
                    section(ui, "Starting up");
                    ui.label(RichText::new("All servers start side by side. Your app gets the tools that are ready within 10 seconds; slower ones (a sign-in, a first download) are added the moment they're ready.").color(MUTED));
                    ui.add_space(4.0);
                    section(ui, "Menu bar");
                    ui.label(RichText::new("MCP Gateway Desk keeps an icon in the menu bar. Close this window any time — your AI apps keep their tools, and the menu lets you switch servers on or off, add folders and open this window again.").color(MUTED));
                    let mut login = crate::tray::start_at_login();
                    if ui.checkbox(&mut login, "Start the menu-bar icon when I log in").changed() {
                        crate::tray::set_start_at_login(login);
                    }
                    ui.add_space(4.0);
                    section(ui, "Transport");
                    ui.label(RichText::new("stdio — each app starts its own gateway").color(MUTED));
                });
            });
            ui.add_space(8.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    section(ui, "Activity log");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Clear").clicked() {
                            let _ = std::fs::write(state::log_path(), "");
                            self.log.clear();
                        }
                    });
                });
                Frame::new().fill(Color32::from_rgb(0x11, 0x14, 0x18)).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(160.0);
                    if self.log.is_empty() {
                        ui.label(RichText::new("Nothing yet. Connect an app or press “Test all servers”.").monospace().color(MUTED));
                    }
                    for l in &self.log {
                        let col = if l.contains("skipped") || l.contains("failed") { AMBER } else if l.contains("ready") { GREEN } else { MUTED };
                        ui.label(RichText::new(l).monospace().size(12.0).color(col));
                    }
                });
            });
        });
        if dirty {
            self.save();
        }
        if test_all {
            let names: Vec<String> = self.state.servers.iter().filter(|s| s.enabled).map(|s| s.name.clone()).collect();
            for n in names {
                if !self.probes.contains_key(&n) {
                    state::append_log(&format!("test: starting {}", n));
                    self.start_probe(&n);
                }
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        self.frame += 1;
        ctx.request_repaint();
        if self.frame >= 120 && !self.shot_asked {
            self.shot_asked = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            let mut buf = Vec::with_capacity(w * h * 4);
            for p in &img.pixels {
                buf.extend_from_slice(&[p.r(), p.g(), p.b(), 255]);
            }
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match image::save_buffer(&path, &buf, w as u32, h as u32, image::ExtendedColorType::Rgba8) {
                Ok(()) => eprintln!("saved screenshot to {}", path.display()),
                Err(e) => eprintln!("screenshot failed: {}", e),
            }
            std::process::exit(0);
        }
        if self.frame > 600 {
            eprintln!("screenshot timed out");
            std::process::exit(1);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        if now - self.clients_at > 3.0 {
            self.clients = CLIENTS.iter().map(|c| c.state()).collect();
            self.clients_at = now;
        }
        if now - self.log_at > 1.0 {
            self.log = state::read_log_tail(14);
            self.log_at = now;
        }
        self.poll_probes();
        self.sync_with_menu(ctx, now);
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.add_folders(dropped);
            self.page = Page::Folders;
        }
        if !self.probes.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }

        self.top_bar(ctx);
        self.side_bar(ctx);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(22, 18)))
            .show(ctx, |ui| match self.page {
                Page::Servers => self.servers_page(ui),
                Page::Catalog => self.catalog_page(ui),
                Page::Folders => self.folders_page(ui, ctx),
                Page::Clients => self.clients_page(ui),
                Page::Secrets => self.secrets_page(ui),
                Page::Gateway => self.gateway_page(ui),
            });

        if let Some((text, color, t0)) = self.toast.clone() {
            let t0 = if t0 == 0.0 { now } else { t0 };
            self.toast = Some((text.clone(), color, t0));
            if now - t0 > 4.0 {
                self.toast = None;
            } else {
                egui::Area::new(egui::Id::new("toast")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -20.0)).show(ctx, |ui| {
                    Frame::new().fill(CARD_HI).stroke(Stroke::new(1.0_f32, color)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                        ui.label(RichText::new(text).color(TEXT));
                    });
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        self.handle_screenshot(ctx);
    }
}
