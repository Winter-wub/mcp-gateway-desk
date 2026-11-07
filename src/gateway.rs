//! The gateway: one MCP server (stdio) that starts every enabled server and merges their
//! tools, like `docker mcp gateway run`. Every server runs as a plain program on this Mac
//! and gets the shared folders directly — no containers, nothing to mount.

use crate::oauth::{self, Progress};
use crate::state::{append_log, secret_get, Runner, Server, State};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// clientInfo the gateway gives each server.
const CLIENT_NAME: &str = "mcp-gateway-desk";
const CLIENT_VERSION: &str = "1.0.0";
/// Variables a program needs to run at all on macOS (npx/uvx use HOME for their caches).
/// Not in the original: there a server gets only its own settings plus PATH.
const BASE_ENV: &[&str] = &["HOME", "USER", "LOGNAME", "TMPDIR", "LANG", "SHELL"];
const START_TIMEOUT: Duration = Duration::from_secs(120);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);

/// Expand `{{...}}` templates. Any template naming `paths`/`folders` becomes the shared
/// folder list (one argument each); others are looked up in the server's settings.
fn expand_arg(arg: &str, server: &Server, folders: &[String]) -> Vec<String> {
    let Some(start) = arg.find("{{") else { return vec![arg.to_string()] };
    let Some(end_rel) = arg[start..].find("}}") else { return vec![arg.to_string()] };
    let end = start + end_rel;
    let expr = arg[start + 2..end].trim();
    let key = expr.split('|').next().unwrap_or("").trim();
    if key == "folders" || key.ends_with(".paths") || key == "paths" {
        if arg.trim() == &arg[start..end + 2] {
            return folders.to_vec();
        }
        let joined = folders.join(",");
        return vec![format!("{}{}{}", &arg[..start], joined, &arg[end + 2..])];
    }
    let short = key.strip_prefix(&format!("{}.", server.name)).unwrap_or(key);
    let value = server.config.get(short).or_else(|| server.config.get(key)).cloned().unwrap_or_default();
    let rest = expand_arg(&arg[end + 2..], server, folders).join("");
    let whole = format!("{}{}{}", &arg[..start], value, rest);
    if whole.is_empty() {
        vec![]
    } else {
        vec![whole]
    }
}

fn expand_all(args: &[String], server: &Server, folders: &[String]) -> Vec<String> {
    args.iter().flat_map(|a| expand_arg(a, server, folders)).collect()
}

/// The command line the gateway would run for a server (also shown in the UI).
pub fn command_line(server: &Server, state: &State) -> (String, Vec<String>, Vec<(String, String)>) {
    let folders: Vec<String> = if server.uses_folders {
        state.folders.iter().map(|f| f.path.clone()).collect()
    } else {
        vec![]
    };
    let mut env: Vec<(String, String)> = server
        .env
        .iter()
        .map(|(k, v)| {
            let e = expand_arg(v, server, &folders).join("");
            (k.clone(), e)
        })
        .collect();
    for sref in &server.secrets {
        if !sref.env.is_empty() {
            env.push((sref.env.clone(), format!("<secret:{}>", sref.name)));
        }
    }
    let Runner::Local { command, args } = &server.runner;
    (command.clone(), expand_all(args, server, &folders), env)
}

/// GUI apps on macOS get a short PATH; add the usual places npx/uvx live.
pub fn full_path() -> String {
    let home = crate::state::home();
    let mut parts: Vec<String> = std::env::var("PATH").unwrap_or_default().split(':').map(String::from).collect();
    for extra in [
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
        format!("{}/.local/bin", home.display()),
        format!("{}/.cargo/bin", home.display()),
        format!("{}/.volta/bin", home.display()),
        "/usr/bin".to_string(),
        "/bin".to_string(),
    ] {
        if !parts.contains(&extra) {
            parts.push(extra);
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join(":")
}

pub fn find_program(name: &str) -> Option<String> {
    if name.contains('/') {
        return std::path::Path::new(name).exists().then(|| name.to_string());
    }
    full_path()
        .split(':')
        .map(|d| std::path::Path::new(d).join(name))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

/// Every server process the gateway started, so they can all be stopped on exit.
static CHILD_PIDS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub struct Upstream {
    pub name: String,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
    next_id: i64,
    /// Shared folders offered to the server as MCP roots.
    roots: Vec<String>,
    /// Set when the server says its tool list changed.
    pub tools_changed: bool,
}

/// Stop a server together with everything it started (npx starts node, for example).
/// Each server runs in its own process group, so the whole group is stopped.
fn stop_group(pid: u32) {
    let _ = Command::new("kill").args(["-TERM", &format!("-{}", pid)]).status();
}

impl Drop for Upstream {
    fn drop(&mut self) {
        stop_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Upstream {
    pub fn spawn(server: &Server, state: &State) -> Result<Upstream, String> {
        Self::spawn_with(server, state, None)
    }

    /// Start a server; `progress` (if given) follows the sign-in messages it prints.
    pub fn spawn_with(server: &Server, state: &State, progress: Option<Arc<Mutex<Progress>>>) -> Result<Upstream, String> {
        let (cmd, args, env) = command_line(server, state);
        if cmd.trim().is_empty() {
            return Err("no command set".into());
        }
        if server.uses_folders && state.folders.is_empty() {
            return Err("this server needs at least one shared folder".into());
        }
        let program = find_program(&cmd).unwrap_or(cmd.clone());
        let mut c = Command::new(&program);
        // Environment: the server gets only its own
        // variables, plus PATH when they don't set it — not everything in our environment.
        c.env_clear();
        for k in BASE_ENV {
            if let Ok(v) = std::env::var(k) {
                c.env(k, v);
            }
        }
        if !env.iter().any(|(k, _)| k == "PATH") {
            c.env("PATH", full_path());
        }
        // Own process group, so stopping it also stops what it started.
        std::os::unix::process::CommandExt::process_group(&mut c, 0);
        c.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &env {
            if let Some(name) = v.strip_prefix("<secret:").and_then(|x| x.strip_suffix('>')) {
                match secret_get(name) {
                    Some(val) => {
                        c.env(k, val);
                    }
                    None => return Err(format!("secret '{}' is not set", name)),
                }
            } else {
                c.env(k, v);
            }
        }
        if let Some(first) = state.folders.first() {
            if server.uses_folders {
                c.current_dir(&first.path);
            }
        }
        let mut child = c.spawn().map_err(|e| format!("could not start {}: {}", cmd, e))?;
        CHILD_PIDS.lock().unwrap().push(child.id());
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let name = server.name.clone();
        if let Some(err) = stderr {
            let n = name.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    if let Some(p) = &progress {
                        p.lock().unwrap().feed(&line);
                    }
                    // Log each line as "- <server>: <line>".
                    append_log(&format!("- {}: {}", n, line));
                }
            });
        }
        let roots = if server.uses_folders { state.folders.iter().map(|f| f.path.clone()).collect() } else { vec![] };
        Ok(Upstream { name, child, stdin, rx, next_id: 1, roots, tools_changed: false })
    }

    fn send(&mut self, v: &Value) -> Result<(), String> {
        let mut s = serde_json::to_string(v).map_err(|e| e.to_string())?;
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())
    }

    pub fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(format!("{} timed out", method));
            }
            let line = match self.rx.recv_timeout(left) {
                Ok(l) => l,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(format!("{} timed out", method)),
                Err(_) => return Err("the server stopped".into()),
            };
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if msg.get("method").and_then(|m| m.as_str()) == Some("notifications/tools/list_changed") {
                self.tools_changed = true;
                continue;
            }
            if msg.get("id").and_then(|x| x.as_i64()) == Some(id) && msg.get("method").is_none() {
                if let Some(err) = msg.get("error") {
                    return Err(err.get("message").and_then(|m| m.as_str()).unwrap_or("error").to_string());
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
            // A request from the server to us (roots/list, ping...): answer politely.
            if let (Some(mid), Some(m)) = (msg.get("id").cloned(), msg.get("method").and_then(|x| x.as_str())) {
                let reply = if m == "roots/list" {
                    let roots: Vec<Value> = self
                        .roots
                        .iter()
                        .map(|p| {
                            let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            json!({"uri": format!("file://{}", p), "name": name})
                        })
                        .collect();
                    json!({"jsonrpc":"2.0","id":mid,"result":{"roots":roots}})
                } else if m == "ping" {
                    json!({"jsonrpc":"2.0","id":mid,"result":{}})
                } else {
                    json!({"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"not supported"}})
                };
                let _ = self.send(&reply);
            }
        }
    }

    pub fn initialize(&mut self) -> Result<(), String> {
        self.initialize_within(START_TIMEOUT)
    }

    pub fn initialize_within(&mut self, timeout: Duration) -> Result<(), String> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"roots": {"listChanged": true}},
                "clientInfo": {"name": CLIENT_NAME, "version": CLIENT_VERSION}
            }),
            timeout,
        )
        // Error wording from stdioMCPClient.Initialize: "failed to connect: %w".
        .map_err(|e| format!("failed to connect: {}", e))?;
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
    }

    pub fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let r = self.request("tools/list", params, START_TIMEOUT)?;
            if let Some(tools) = r.get("tools").and_then(|t| t.as_array()) {
                all.extend(tools.iter().cloned());
            }
            cursor = r.get("nextCursor").and_then(|c| c.as_str()).map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(all)
    }
}

/// Start a server, ask for its tools, stop it. Used by "Check tools" in the window.
/// A remote server that isn't signed in yet gets time to finish the browser sign-in.
pub fn probe(server: &Server, state: &State, progress: Arc<Mutex<Progress>>) -> Result<Vec<crate::state::ToolInfo>, String> {
    let needs_sign_in = oauth::remote_url(server).map(|u| !oauth::signed_in(&u)).unwrap_or(false);
    let mut up = Upstream::spawn_with(server, state, Some(progress.clone()))?;
    let wait = if needs_sign_in { Duration::from_secs(oauth::SIGN_IN_TIMEOUT_SECS) } else { START_TIMEOUT };
    up.initialize_within(wait).map_err(|e| {
        if progress.lock().unwrap().other_signin {
            "another sign-in for this server is still open — finish it in your browser, or press “Stop stuck sign-ins”".to_string()
        } else if needs_sign_in && e.contains("timed out") {
            "sign-in wasn't finished in time".to_string()
        } else {
            e
        }
    })?;
    let tools = up.list_tools()?;
    Ok(tools
        .iter()
        .map(|t| crate::state::ToolInfo {
            name: t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            description: t
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .to_string(),
        })
        .collect())
}

/// Shared stdout: replies come from several threads (calls run side by side).
#[derive(Clone)]
struct Out(Arc<Mutex<std::io::Stdout>>);

impl Out {
    fn send(&self, v: Value) {
        let mut o = self.0.lock().unwrap();
        let _ = writeln!(o, "{}", v);
        let _ = o.flush();
    }
}

#[derive(Default)]
struct Registry {
    tools: Vec<Value>,
    route: HashMap<String, (Arc<Mutex<Upstream>>, String, String)>,
    pending: usize,
    /// The client has asked for tools at least once, so later arrivals need a notice.
    listed: bool,
    /// Servers the gateway wants running: name -> (settings fingerprint, start number).
    wanted: HashMap<String, (String, u64)>,
    next_start: u64,
}

impl Registry {
    /// Take one server's tools out (it was switched off, removed or is restarting).
    fn unregister(&mut self, server: &str) {
        let owned: Vec<String> = self.route.iter().filter(|(_, v)| v.2 == server).map(|(k, _)| k.clone()).collect();
        for k in &owned {
            self.route.remove(k);
        }
        self.tools.retain(|t| !owned.iter().any(|o| t["name"] == o.as_str()));
    }

    /// Add (or replace) one server's tools. A name another server already uses gets the
    /// server name in front ("server__tool"). Returns how many tools were added.
    fn register(&mut self, server: &str, disabled: &std::collections::BTreeSet<String>, up: &Arc<Mutex<Upstream>>, list: Vec<Value>) -> usize {
        let owned: Vec<String> = self.route.iter().filter(|(_, v)| v.2 == server).map(|(k, _)| k.clone()).collect();
        for k in &owned {
            self.route.remove(k);
        }
        self.tools.retain(|t| !owned.iter().any(|o| t["name"] == o.as_str()));
        let mut count = 0;
        for mut t in list {
            let name = t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if name.is_empty() || disabled.contains(&name) {
                continue;
            }
            let public = if self.route.contains_key(&name) { format!("{}__{}", server, name) } else { name.clone() };
            t["name"] = Value::String(public.clone());
            self.route.insert(public, (up.clone(), name, server.to_string()));
            self.tools.push(t);
            count += 1;
        }
        count
    }
}

/// How long the first tools/list waits for slow servers before answering with what's ready.
const FIRST_LIST_WAIT: Duration = Duration::from_secs(10);
/// Settings changes are applied once the file has been quiet this long; each new change
/// restarts the wait.
const RELOAD_SETTLE: Duration = Duration::from_millis(300);
/// How often the settings file is checked (the original is told by fsnotify; we look).
const WATCH_POLL: Duration = Duration::from_millis(100);
/// "Tools changed" notices are batched: the MCP library waits 0x989680 ns = 10 ms and
/// restarts the wait on each new change.
const NOTIFY_DELAY: Duration = Duration::from_millis(10);

/// Everything that decides how a server runs; a change means it must be restarted.
fn fingerprint(server: &Server, state: &State) -> String {
    let folders: Vec<&str> = if server.uses_folders { state.folders.iter().map(|f| f.path.as_str()).collect() } else { vec![] };
    let signed_in = oauth::remote_url(server).map(|u| oauth::signed_in(&u));
    json!([server.runner, server.env, server.config, server.secrets.len(), folders, server.disabled_tools, signed_in]).to_string()
}

/// Sends "notifications/tools/list_changed", batching changes that come close together.
#[derive(Clone)]
struct Notifier(std::sync::mpsc::Sender<()>);

impl Notifier {
    fn start(out: Out, reg: Arc<Mutex<Registry>>) -> Notifier {
        let (tx, rx) = channel::<()>();
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                // Restart the 10 ms wait while more changes keep coming.
                while rx.recv_timeout(NOTIFY_DELAY).is_ok() {}
                if reg.lock().unwrap().listed {
                    out.send(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}));
                }
            }
        });
        Notifier(tx)
    }

    fn changed(&self) {
        let _ = self.0.send(());
    }
}

/// Start one server in the background and add its tools when it's ready.
fn start_server(server: Server, state: Arc<State>, reg: Arc<Mutex<Registry>>, notify: Notifier) {
    let start_no = {
        let mut r = reg.lock().unwrap();
        r.next_start += 1;
        let n = r.next_start;
        r.wanted.insert(server.name.clone(), (fingerprint(&server, &state), n));
        r.pending += 1;
        n
    };
    std::thread::spawn(move || {
        // Like the original's token check: never start a sign-in from here, it would
        // hold the app up and block the callback port. Signing in is done in the window.
        let started = match oauth::remote_url(&server) {
            Some(url) if !oauth::signed_in(&url) => Err("needs sign-in — open MCP Gateway Desk and press “Sign in”".to_string()),
            _ => Upstream::spawn(&server, &state).and_then(|mut u| {
                u.initialize()?;
                let list = u.list_tools()?;
                Ok((u, list))
            }),
        };
        let mut r = reg.lock().unwrap();
        r.pending -= 1;
        // Switched off or changed again while it was starting: drop this copy.
        if r.wanted.get(&server.name).map(|w| w.1) != Some(start_no) {
            return;
        }
        match started {
            Ok((u, list)) => {
                let up = Arc::new(Mutex::new(u));
                let count = r.register(&server.name, &server.disabled_tools, &up, list);
                append_log(&format!("gateway: {} ready ({} tools, {})", server.name, count, server.runner.label()));
                if count > 0 {
                    notify.changed();
                }
            }
            Err(e) => append_log(&format!("gateway: {} skipped: {}", server.name, e)),
        }
        if r.pending == 0 {
            append_log(&format!("gateway: all servers started, {} tools", r.tools.len()));
        }
    });
}

/// Bring the running servers in line with the settings: start new or changed ones, stop
/// the ones switched off, then tell the app its tools changed.
fn apply(state: &Arc<State>, reg: &Arc<Mutex<Registry>>, notify: &Notifier) {
    let want: HashMap<String, (Server, String)> = state
        .servers
        .iter()
        .filter(|s| s.enabled)
        .map(|s| (s.name.clone(), (s.clone(), fingerprint(s, state))))
        .collect();
    let mut stopped = false;
    let to_start: Vec<Server> = {
        let mut r = reg.lock().unwrap();
        let running: Vec<String> = r.wanted.keys().cloned().collect();
        for name in running {
            let keep = want.get(&name).map(|(_, fp)| Some(fp) == r.wanted.get(&name).map(|w| &w.0)).unwrap_or(false);
            if !keep {
                r.wanted.remove(&name);
                r.unregister(&name); // its last handle goes, so the process group is stopped
                append_log(&format!("gateway: {} stopped (settings changed)", name));
                stopped = true;
            }
        }
        want.values().filter(|(s, _)| !r.wanted.contains_key(&s.name)).map(|(s, _)| s.clone()).collect()
    };
    if stopped {
        notify.changed();
    }
    for s in to_start {
        start_server(s, state.clone(), reg.clone(), notify.clone());
    }
}

fn state_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(State::path()).and_then(|m| m.modified()).ok()
}

/// `mcp-gateway-desk gateway run`: the stdio gateway AI apps connect to.
/// It answers the app straight away and starts every server side by side in the
/// background, so one slow server (a sign-in, a download) can't hide the others' tools.
/// It keeps watching the settings (like the original's --watch, on by default), so
/// switching a server on or off — from the window or the menu bar — reaches the app
/// without restarting it.
pub fn run_stdio() {
    let state = Arc::new(State::load());
    append_log("gateway: starting");
    let out = Out(Arc::new(Mutex::new(std::io::stdout())));
    let reg = Arc::new(Mutex::new(Registry::default()));
    let started_at = std::time::Instant::now();
    let notify = Notifier::start(out.clone(), reg.clone());
    let current = Arc::new(Mutex::new(state.clone()));

    apply(&state, &reg, &notify);

    {
        let (reg, notify, current) = (reg.clone(), notify.clone(), current.clone());
        std::thread::spawn(move || {
            let mut seen = state_mtime();
            let mut changed_at: Option<std::time::Instant> = None;
            loop {
                std::thread::sleep(WATCH_POLL);
                let m = state_mtime();
                if m != seen {
                    seen = m;
                    changed_at = Some(std::time::Instant::now()); // restart the 300 ms wait
                }
                if changed_at.map(|t| t.elapsed() >= RELOAD_SETTLE).unwrap_or(false) {
                    changed_at = None;
                    let fresh = Arc::new(State::load());
                    append_log("gateway: settings changed, reloading");
                    *current.lock().unwrap() = fresh.clone();
                    apply(&fresh, &reg, &notify);
                }
            }
        });
    }

    let stdin = std::io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
        match method.as_str() {
            "initialize" => {
                let ver = msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION).to_string();
                out.send(json!({"jsonrpc":"2.0","id":id,"result":{
                    "protocolVersion": ver,
                    "capabilities": {"tools": {"listChanged": true}},
                    "serverInfo": {"name": "MCP Gateway Desk", "version": env!("CARGO_PKG_VERSION")}
                }}));
            }
            "ping" => out.send(json!({"jsonrpc":"2.0","id":id,"result":{}})),
            "prompts/list" => out.send(json!({"jsonrpc":"2.0","id":id,"result":{"prompts": []}})),
            "resources/list" => out.send(json!({"jsonrpc":"2.0","id":id,"result":{"resources": []}})),
            "tools/list" => {
                let (reg, out) = (reg.clone(), out.clone());
                std::thread::spawn(move || {
                    // Give servers a moment on the first ask; never wait for a stuck one.
                    while started_at.elapsed() < FIRST_LIST_WAIT && reg.lock().unwrap().pending > 0 {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    let mut r = reg.lock().unwrap();
                    r.listed = true;
                    if r.pending > 0 {
                        append_log(&format!("gateway: listing {} tools; {} server(s) still starting", r.tools.len(), r.pending));
                    }
                    out.send(json!({"jsonrpc":"2.0","id":id,"result":{"tools": r.tools}}));
                });
            }
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
                let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
                let target = reg.lock().unwrap().route.get(&name).cloned();
                let notify_call = notify.clone();
                let state = current.lock().unwrap().clone();
                let (out, log_calls, reg) = (out.clone(), state.settings.log_calls, reg.clone());
                std::thread::spawn(move || {
                    let result = match target {
                        Some((up, real, server)) => {
                            if log_calls {
                                append_log(&format!("call: {} -> {}", name, server));
                            }
                            let mut u = up.lock().unwrap();
                            let r = u.request("tools/call", json!({"name": real, "arguments": args}), CALL_TIMEOUT);
                            // The server said its tools changed: read them again and tell the app.
                            if u.tools_changed {
                                u.tools_changed = false;
                                if let Ok(list) = u.list_tools() {
                                    drop(u);
                                    let disabled = state.servers.iter().find(|s| s.name == server).map(|s| s.disabled_tools.clone()).unwrap_or_default();
                                    let n = reg.lock().unwrap().register(&server, &disabled, &up, list);
                                    append_log(&format!("gateway: {} tools changed ({} tools)", server, n));
                                    let _ = &out;
                                    notify_call.changed();
                                }
                            }
                            r
                        }
                        None => Err(format!("unknown tool {}", name)),
                    };
                    match result {
                        Ok(r) => out.send(json!({"jsonrpc":"2.0","id":id,"result":r})),
                        Err(e) => {
                            append_log(&format!("call: {} failed: {}", name, e));
                            out.send(json!({"jsonrpc":"2.0","id":id,"result":{
                                "content":[{"type":"text","text":format!("Error: {}", e)}],"isError":true}}))
                        }
                    }
                });
            }
            _ => out.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}})),
        }
    }
    append_log("gateway: client closed, stopping");
    // Stop child servers even if startup threads still hold them.
    for pid in CHILD_PIDS.lock().unwrap().iter() {
        stop_group(*pid);
    }
    std::process::exit(0);
}
