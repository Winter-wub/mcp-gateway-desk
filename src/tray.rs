//! Menu-bar (tray) icon: `mcp-gateway-desk tray`. A small background process with no
//! window and no Dock icon, so the person doesn't have to keep the window open.
//!
//! The AI apps start their own gateway (`gateway run`), so tools work whether or not this
//! menu or the window is running; the menu is for seeing and changing things quickly.

use crate::clients::{Status, CLIENTS};
use crate::oauth;
use crate::state::{self, data_dir, State};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// How often the menu re-reads the saved settings (servers, folders, sign-ins).
const REFRESH: Duration = Duration::from_secs(3);
const LAUNCH_AGENT_ID: &str = "com.craftanything.mcp-gateway-desk";

fn pid_file(name: &str) -> PathBuf {
    data_dir().join(name)
}

pub fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The process id saved in a pid file, if that process is still running.
pub fn running(name: &str) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pid_file(name)).ok()?.trim().parse().ok()?;
    (pid != std::process::id() && pid_alive(pid)).then_some(pid)
}

pub fn write_pid(name: &str) {
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(pid_file(name), std::process::id().to_string());
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "mcp-gateway-desk".into())
}

/// Start the menu-bar icon in the background unless it is already running.
pub fn ensure_running() {
    if running("tray.pid").is_some() {
        return;
    }
    let mut c = std::process::Command::new(exe());
    c.arg("tray")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    let _ = c.spawn();
}

/// Bring the window up (on a page), starting it if it isn't running.
fn open_window(page: &str) {
    if running("window.pid").is_some() {
        // The open window checks this file every second and comes to the front.
        let _ = std::fs::write(data_dir().join("show-request"), page);
        return;
    }
    let mut c = std::process::Command::new(exe());
    c.env("MCP_DESK_PAGE", page)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    let _ = c.spawn();
}

fn launch_agent_path() -> PathBuf {
    state::home().join("Library/LaunchAgents").join(format!("{}.plist", LAUNCH_AGENT_ID))
}

pub fn start_at_login() -> bool {
    launch_agent_path().exists()
}

pub fn set_start_at_login(on: bool) {
    let path = launch_agent_path();
    if on {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{}</string>
  <key>ProgramArguments</key><array><string>{}</string><string>tray</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
            LAUNCH_AGENT_ID,
            exe().replace('&', "&amp;").replace('<', "&lt;")
        );
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, plist);
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

/// Menu-bar icon: three linked dots, drawn as a template image so macOS colours it to
/// match the menu bar (light or dark).
fn icon() -> Icon {
    const S: usize = 44;
    let mut px = vec![0u8; S * S * 4];
    let dots = [(11.0f32, 31.0f32), (22.0, 13.0), (33.0, 31.0)];
    let seg_dist = |p: (f32, f32), a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
    };
    for y in 0..S {
        for x in 0..S {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let mut a: f32 = 0.0;
            for d in dots {
                let r = ((p.0 - d.0).powi(2) + (p.1 - d.1).powi(2)).sqrt();
                a = a.max((6.5 - r).clamp(0.0, 1.0));
            }
            for (i, j) in [(0, 1), (1, 2), (0, 2)] {
                a = a.max((2.0 - seg_dist(p, dots[i], dots[j])).clamp(0.0, 1.0));
            }
            let k = (y * S + x) * 4;
            px[k + 3] = (a * 255.0) as u8;
        }
    }
    Icon::from_rgba(px, S as u32, S as u32).expect("icon")
}

/// Everything the menu shows, read from the saved settings.
#[derive(PartialEq, Clone)]
struct Snapshot {
    servers: Vec<(String, String, bool, bool)>, // name, title, enabled, needs sign-in
    folders: usize,
    tools: usize,
    connected: Vec<String>,
    start_at_login: bool,
}

fn snapshot() -> Snapshot {
    let st = State::load();
    Snapshot {
        servers: st
            .servers
            .iter()
            .map(|s| {
                let needs = oauth::remote_url(s).map(|u| !oauth::signed_in(&u)).unwrap_or(false);
                (s.name.clone(), s.title.clone(), s.enabled, needs)
            })
            .collect(),
        folders: st.folders.len(),
        tools: st
            .servers
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.known_tools.iter().filter(|t| !s.disabled_tools.contains(&t.name)).count())
            .sum(),
        connected: CLIENTS.iter().filter(|c| c.state().status == Status::Connected).map(|c| c.name.to_string()).collect(),
        start_at_login: start_at_login(),
    }
}

struct Ui {
    menu: Menu,
    toggles: Vec<(CheckMenuItem, String)>,
    open: MenuItem,
    folders: MenuItem,
    add_folder: MenuItem,
    clients: MenuItem,
    log: MenuItem,
    login: CheckMenuItem,
    about: MenuItem,
    quit: MenuItem,
}

fn build(s: &Snapshot) -> Ui {
    let menu = Menu::new();
    let on = s.servers.iter().filter(|x| x.2).count();
    let head = MenuItem::new("MCP Gateway Desk", false, None);
    let status = MenuItem::new(format!("{} of {} servers on · {} tools", on, s.servers.len(), s.tools), false, None);
    let apps = MenuItem::new(
        if s.connected.is_empty() { "No AI apps connected".to_string() } else { format!("Connected: {}", s.connected.join(", ")) },
        false,
        None,
    );
    let servers = Submenu::new("Servers", true);
    let mut toggles = vec![];
    for (name, title, enabled, needs) in &s.servers {
        let label = if *needs { format!("{} — needs sign-in", title) } else { title.clone() };
        let item = CheckMenuItem::new(label, true, *enabled, None);
        let _ = servers.append(&item);
        toggles.push((item, name.clone()));
    }
    if s.servers.is_empty() {
        let _ = servers.append(&MenuItem::new("No servers yet", false, None));
    }
    let open = MenuItem::new("Open MCP Gateway Desk…", true, None);
    let folders = MenuItem::new(format!("Shared folders ({})…", s.folders), true, None);
    let add_folder = MenuItem::new("Add a folder…", true, None);
    let clients = MenuItem::new("Connect an AI app…", true, None);
    let log = MenuItem::new("Activity log", true, None);
    let login = CheckMenuItem::new("Start at login", true, s.start_at_login, None);
    let about = MenuItem::new("About MCP Gateway Desk", true, None);
    let quit = MenuItem::new("Quit menu-bar icon", true, None);
    let _ = menu.append_items(&[
        &head,
        &status,
        &apps,
        &PredefinedMenuItem::separator(),
        &open,
        &servers,
        &folders,
        &add_folder,
        &clients,
        &log,
        &PredefinedMenuItem::separator(),
        &login,
        &about,
        &quit,
    ]);
    Ui { menu, toggles, open, folders, add_folder, clients, log, login, about, quit }
}

enum UserEvent {
    Menu(MenuEvent),
}

pub fn run() {
    if running("tray.pid").is_some() {
        return; // one menu-bar icon is enough
    }
    write_pid("tray.pid");

    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // No Dock icon and no app switcher entry: it lives in the menu bar only.
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));

    let mut tray: Option<TrayIcon> = None;
    let mut ui: Option<Ui> = None;
    let mut shown = snapshot();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + REFRESH);
        match event {
            Event::NewEvents(StartCause::Init) => {
                let u = build(&shown);
                tray = TrayIconBuilder::new()
                    .with_menu(Box::new(u.menu.clone()))
                    .with_tooltip("MCP Gateway Desk")
                    .with_icon(icon())
                    .with_icon_as_template(true)
                    .build()
                    .map_err(|e| state::append_log(&format!("menu: couldn't add the menu-bar icon: {}", e)))
                    .ok();
                if tray.is_some() {
                    state::append_log("menu: menu-bar icon is up");
                }
                ui = Some(u);
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                let now = snapshot();
                if now != shown {
                    shown = now;
                    let u = build(&shown);
                    if let Some(t) = &tray {
                        t.set_menu(Some(Box::new(u.menu.clone())));
                    }
                    ui = Some(u);
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                let Some(u) = &ui else { return };
                if e.id == *u.open.id() {
                    open_window("servers");
                } else if e.id == *u.folders.id() {
                    open_window("folders");
                } else if e.id == *u.clients.id() {
                    open_window("clients");
                } else if e.id == *u.about.id() {
                    open_window("gateway");
                } else if e.id == *u.add_folder.id() {
                    if let Some(paths) = rfd::FileDialog::new().set_title("Share folders with your MCP servers").pick_folders() {
                        let mut st = State::load();
                        let mut added = 0;
                        for p in paths {
                            if st.add_folder(&p.display().to_string()) {
                                added += 1;
                            }
                        }
                        if added > 0 && st.save().is_ok() {
                            state::append_log(&format!("menu: added {} shared folder(s)", added));
                        }
                    }
                } else if e.id == *u.log.id() {
                    let _ = std::process::Command::new("open").args(["-e", &state::log_path().display().to_string()]).spawn();
                } else if e.id == *u.login.id() {
                    set_start_at_login(u.login.is_checked());
                } else if e.id == *u.quit.id() {
                    let _ = std::fs::remove_file(pid_file("tray.pid"));
                    *control_flow = ControlFlow::Exit;
                    return;
                } else if let Some((item, name)) = u.toggles.iter().find(|(i, _)| e.id == *i.id()) {
                    let mut st = State::load();
                    if let Some(i) = st.server_index(name) {
                        st.servers[i].enabled = item.is_checked();
                        let _ = st.save();
                        state::append_log(&format!("menu: {} turned {}", name, if item.is_checked() { "on" } else { "off" }));
                    }
                }
                // Show the change at once.
                shown = snapshot();
                let nu = build(&shown);
                if let Some(t) = &tray {
                    t.set_menu(Some(Box::new(nu.menu.clone())));
                }
                ui = Some(nu);
            }
            _ => {}
        }
    });
}
