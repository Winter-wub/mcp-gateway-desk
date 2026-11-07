//! Everything the app remembers: servers, shared folders, gateway settings.
//! Stored as JSON in ~/.mcp-gateway-desk/state.json (see MECHANICS.md).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Name the gateway is registered under in each client (original uses MCP_DOCKER).
pub const ENTRY_NAME: &str = "MCP_DESK";
/// Keychain service name used for secrets.
pub const KEYCHAIN_SERVICE: &str = "mcp-gateway-desk";

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MCP_DESK_HOME") {
        return PathBuf::from(d);
    }
    home().join(".mcp-gateway-desk")
}

pub fn log_path() -> PathBuf {
    data_dir().join("gateway.log")
}

/// How a server starts. Every server runs straight on this Mac as a normal program:
/// no Docker, no containers, so files are reached directly.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum Runner {
    Local { command: String, args: Vec<String> },
}

impl Runner {
    pub fn label(&self) -> &'static str {
        "On this Mac"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub env: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    pub runner: Runner,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Gets the shared folders as arguments ({{folders}} in its arguments).
    #[serde(default)]
    pub uses_folders: bool,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: Vec<SecretRef>,
    #[serde(default)]
    pub known_tools: Vec<ToolInfo>,
    #[serde(default)]
    pub disabled_tools: BTreeSet<String>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub path: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "yes")]
    pub log_calls: bool,
}

impl Default for Settings {
    // Tool calls are logged by default.
    fn default() -> Self {
        Settings { log_calls: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub secret_names: BTreeSet<String>,
    #[serde(default)]
    pub seeded: bool,
}

impl State {
    pub fn path() -> PathBuf {
        data_dir().join("state.json")
    }

    pub fn load() -> State {
        let Some(mut v) = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        else {
            return State::default();
        };
        // Older saves could hold Docker servers. Docker is no longer used: switch a server
        // to its run-on-Mac command when it had one, otherwise leave it out.
        if let Some(list) = v.get_mut("servers").and_then(|x| x.as_array_mut()) {
            list.retain_mut(|srv| {
                if srv["runner"]["kind"] == "Local" {
                    return true;
                }
                if srv["alt_runner"]["kind"] == "Local" {
                    srv["runner"] = srv["alt_runner"].take();
                    return true;
                }
                false
            });
        }
        serde_json::from_value(v).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join("state.json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, Self::path()).map_err(|e| e.to_string())
    }

    pub fn server_index(&self, name: &str) -> Option<usize> {
        self.servers.iter().position(|s| s.name == name)
    }

    pub fn add_folder(&mut self, path: &str) -> bool {
        let p = path.trim_end_matches('/').to_string();
        if p.is_empty() || self.folders.iter().any(|f| f.path == p) {
            return false;
        }
        self.folders.push(Folder { path: p, read_only: false });
        true
    }
}

/// Small time stamp for the log ("HH:MM:SS", local offset not applied).
pub fn clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let d = secs % 86_400;
    format!("{:02}:{:02}:{:02}", d / 3600, (d / 60) % 60, d % 60)
}

pub fn append_log(line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = writeln!(f, "{} {}", clock(), line);
    }
}

pub fn read_log_tail(max_lines: usize) -> Vec<String> {
    let text = std::fs::read_to_string(log_path()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].iter().map(|s| s.to_string()).collect()
}

// ---------- Secrets: the macOS Keychain, like the original's "local OS Keychain" ----------

pub fn secret_set(name: &str, value: &str) -> Result<(), String> {
    let out = std::process::Command::new("security")
        .args(["add-generic-password", "-U", "-s", KEYCHAIN_SERVICE, "-a", name, "-w", value])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn secret_get(name: &str) -> Option<String> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", name, "-w"])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    } else {
        None
    }
}

pub fn secret_delete(name: &str) {
    let _ = std::process::Command::new("security")
        .args(["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", name])
        .output();
}
