//! AI apps the gateway can be connected to. Paths and keys come from the client list built
//! into common MCP toolkits.

use crate::state::{home, ENTRY_NAME};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    /// JSON object under `key`, entry = {command, args} + extra fields.
    JsonMap { key: &'static str, extra: &'static str },
    OpenCode,
    ContinueYaml,
    GooseYaml,
    CodexToml,
}

pub struct Client {
    pub id: &'static str,
    pub name: &'static str,
    pub install_paths: &'static [&'static str],
    pub config_paths: &'static [&'static str],
    pub format: Format,
}

const MCP: Format = Format::JsonMap { key: "mcpServers", extra: "" };

pub const CLIENTS: &[Client] = &[
    Client { id: "claude-desktop", name: "Claude Desktop", install_paths: &["/Applications/Claude.app"], config_paths: &["~/Library/Application Support/Claude/claude_desktop_config.json"], format: MCP },
    Client { id: "claude-code", name: "Claude Code", install_paths: &["$HOME/.claude", "$CLAUDE_CONFIG_DIR"], config_paths: &["$CLAUDE_CONFIG_DIR/.claude.json", "$HOME/.claude.json"], format: MCP },
    Client { id: "cursor", name: "Cursor", install_paths: &["/Applications/Cursor.app"], config_paths: &["~/.cursor/mcp.json"], format: MCP },
    Client { id: "vscode", name: "Visual Studio Code", install_paths: &["/Applications/Visual Studio Code.app"], config_paths: &["~/Library/Application Support/Code/User/mcp.json"], format: Format::JsonMap { key: "servers", extra: r#"{"type":"stdio"}"# } },
    Client { id: "codex", name: "Codex", install_paths: &["~/.codex"], config_paths: &["~/.codex/config.toml"], format: Format::CodexToml },
    Client { id: "gemini", name: "Gemini CLI", install_paths: &["~/.gemini"], config_paths: &["~/.gemini/settings.json"], format: MCP },
    Client { id: "cline", name: "Cline", install_paths: &["~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev", "~/Library/Application Support/Cursor/User/globalStorage/saoudrizwan.claude-dev"], config_paths: &["~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json", "~/Library/Application Support/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"], format: MCP },
    Client { id: "continue", name: "Continue.dev", install_paths: &["~/.continue"], config_paths: &["~/.continue/config.yaml"], format: Format::ContinueYaml },
    Client { id: "zed", name: "Zed", install_paths: &["~/.config/zed"], config_paths: &["~/.config/zed/settings.json"], format: Format::JsonMap { key: "context_servers", extra: r#"{"source":"custom","enabled":true}"# } },
    Client { id: "lmstudio", name: "LM Studio", install_paths: &["~/.lmstudio", "~/.cache/lm-studio"], config_paths: &["~/.cache/lm-studio/mcp.json", "~/.lmstudio/mcp.json"], format: MCP },
    Client { id: "goose", name: "Goose", install_paths: &["~/.config/goose"], config_paths: &["~/.config/goose/config.yaml"], format: Format::GooseYaml },
    Client { id: "opencode", name: "OpenCode", install_paths: &["~/.config/opencode"], config_paths: &["~/.config/opencode/opencode.json"], format: Format::OpenCode },
    Client { id: "kiro", name: "Kiro", install_paths: &["~/.kiro"], config_paths: &["~/.kiro/settings/mcp.json"], format: MCP },
    Client { id: "crush", name: "Crush", install_paths: &["~/.config/crush"], config_paths: &["~/.config/crush/crush.json"], format: Format::JsonMap { key: "mcp", extra: r#"{"type":"stdio"}"# } },
    Client { id: "sema4", name: "Sema4.ai Studio", install_paths: &["~/.sema4ai"], config_paths: &["~/.sema4ai/sema4ai-studio/mcp_servers.json"], format: Format::JsonMap { key: "mcpServers", extra: r#"{"transport":"stdio"}"# } },
];

/// Expand "~/" and $VARIABLES in a path.
/// A path that names a variable that isn't set is skipped (None).
pub fn expand(p: &str) -> Option<PathBuf> {
    let p = match p.strip_prefix("~/") {
        Some(rest) => format!("{}/{}", home().display(), rest),
        None => p.to_string(),
    };
    let mut out = String::new();
    let mut rest = p.as_str();
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        let var = &after[..end];
        let val = if var == "HOME" { home().display().to_string() } else { std::env::var(var).ok()? };
        if val.is_empty() {
            return None;
        }
        out.push_str(&val);
        rest = &after[end..];
    }
    out.push_str(rest);
    Some(PathBuf::from(out))
}

/// Goose's short entry name: letters only, lower case.
pub fn simple_name(name: &str) -> String {
    name.chars().filter(|c| c.is_alphabetic()).flat_map(|c| c.to_lowercase()).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    NotInstalled,
    Installed,
    Connected,
}

pub struct ClientState {
    pub status: Status,
    pub config_path: PathBuf,
}

/// The command clients start: this app in gateway mode.
pub fn gateway_command() -> (String, Vec<String>) {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "mcp-gateway-desk".into());
    (exe, vec!["gateway".into(), "run".into()])
}

pub fn manual_snippet() -> String {
    let (cmd, args) = gateway_command();
    let v = json!({"mcpServers": {ENTRY_NAME: {"command": cmd, "args": args}}});
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

impl Client {
    pub fn config_path(&self) -> PathBuf {
        // First existing config file, else the first one listed.
        let paths: Vec<PathBuf> = self.config_paths.iter().filter_map(|p| expand(p)).collect();
        paths.iter().find(|p| p.exists()).or(paths.first()).cloned().unwrap_or_default()
    }

    pub fn state(&self) -> ClientState {
        let config_path = self.config_path();
        // Installed when any of its check paths exists (the settings file alone doesn't count).
        let installed = self.install_paths.iter().filter_map(|p| expand(p)).any(|p| p.exists());
        let text = std::fs::read_to_string(&config_path).unwrap_or_default();
        let has = |name: &str| match self.format {
            Format::JsonMap { key, .. } => serde_json::from_str::<Value>(&text).ok().map(|v| v[key].get(name).is_some()).unwrap_or(false),
            Format::OpenCode => serde_json::from_str::<Value>(&text).ok().map(|v| v["mcp"].get(name).is_some()).unwrap_or(false),
            Format::CodexToml => text.contains(&format!("[mcp_servers.{}]", name)),
            Format::ContinueYaml => text.contains(name),
            Format::GooseYaml => serde_yaml::from_str::<serde_yaml::Value>(&text)
                .ok()
                .and_then(|v| v.get("extensions").and_then(|e| e.get(simple_name(name).as_str())).cloned())
                .is_some(),
        };
        let connected = has(ENTRY_NAME);
        let status = if connected {
            Status::Connected
        } else if installed {
            Status::Installed
        } else {
            Status::NotInstalled
        };
        ClientState { status, config_path }
    }

    pub fn connect(&self) -> Result<(), String> {
        self.edit(true)
    }

    pub fn disconnect(&self) -> Result<(), String> {
        self.edit(false)
    }

    fn edit(&self, add: bool) -> Result<(), String> {
        let path = self.config_path();
        let (cmd, args) = gateway_command();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if !text.is_empty() {
            // Keep one copy of the file as it was before we first touched it.
            let backup = PathBuf::from(format!("{}.before-mcp-desk", path.display()));
            if !backup.exists() {
                std::fs::write(&backup, &text).map_err(|e| e.to_string())?;
            }
        }
        let new_text = match self.format {
            Format::JsonMap { key, extra } => {
                let mut v = parse_json(&text)?;
                if !v[key].is_object() {
                    v[key] = json!({});
                }
                if add {
                    let mut entry = json!({"command": cmd, "args": args});
                    if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(extra) {
                        for (k, x) in m {
                            entry[k] = x;
                        }
                    }
                    v[key][ENTRY_NAME] = entry;
                } else if let Some(o) = v[key].as_object_mut() {
                    o.remove(ENTRY_NAME);
                }
                serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
            }
            Format::OpenCode => {
                let mut v = parse_json(&text)?;
                if !v["mcp"].is_object() {
                    v["mcp"] = json!({});
                }
                if add {
                    let mut command = vec![cmd.clone()];
                    command.extend(args.clone());
                    v["mcp"][ENTRY_NAME] = json!({"type": "local", "command": command, "enabled": true});
                } else if let Some(o) = v["mcp"].as_object_mut() {
                    o.remove(ENTRY_NAME);
                }
                serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
            }
            Format::ContinueYaml => {
                let mut v: serde_yaml::Value = if text.trim().is_empty() { serde_yaml::Value::Mapping(Default::default()) } else { serde_yaml::from_str(&text).map_err(|e| e.to_string())? };
                let list = v.get("mcpServers").and_then(|x| x.as_sequence()).cloned().unwrap_or_default();
                let mut list: Vec<serde_yaml::Value> = list.into_iter().filter(|i| i.get("name").and_then(|n| n.as_str()) != Some(ENTRY_NAME)).collect();
                if add {
                    list.push(serde_yaml::to_value(json!({"name": ENTRY_NAME, "command": cmd, "args": args})).map_err(|e| e.to_string())?);
                }
                v["mcpServers"] = serde_yaml::Value::Sequence(list);
                serde_yaml::to_string(&v).map_err(|e| e.to_string())?
            }
            Format::GooseYaml => {
                let mut v: serde_yaml::Value = if text.trim().is_empty() { serde_yaml::Value::Mapping(Default::default()) } else { serde_yaml::from_str(&text).map_err(|e| e.to_string())? };
                let simple = simple_name(ENTRY_NAME);
                if v.get("extensions").and_then(|e| e.as_mapping()).is_none() {
                    v["extensions"] = serde_yaml::Value::Mapping(Default::default());
                }
                let ext = v["extensions"].as_mapping_mut().ok_or("bad goose config")?;
                if add {
                    let entry = json!({
                        "args": args, "bundled": null, "cmd": cmd,
                        "description": "MCP Gateway Desk: your MCP servers, with direct access to your shared folders",
                        "enabled": true, "env_keys": [], "envs": {}, "name": simple,
                        "timeout": 300, "type": "stdio"
                    });
                    ext.insert(simple.clone().into(), serde_yaml::to_value(entry).map_err(|e| e.to_string())?);
                } else {
                    ext.remove(simple.as_str());
                }
                serde_yaml::to_string(&v).map_err(|e| e.to_string())?
            }
            Format::CodexToml => {
                let header = format!("[mcp_servers.{}]", ENTRY_NAME);
                let mut out = String::new();
                let mut skipping = false;
                for line in text.lines() {
                    let t = line.trim();
                    if t == header {
                        skipping = true;
                        continue;
                    }
                    if skipping && t.starts_with('[') {
                        skipping = false;
                    }
                    if !skipping {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                if add {
                    let quoted: Vec<String> = args.iter().map(|a| format!("{:?}", a)).collect();
                    if !out.is_empty() && !out.ends_with("\n\n") {
                        out.push('\n');
                    }
                    out.push_str(&format!("{}\ncommand = {:?}\nargs = [{}]\n", header, cmd, quoted.join(", ")));
                }
                out
            }
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, new_text).map_err(|e| e.to_string())
    }
}

fn parse_json(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(text).map_err(|e| format!("couldn't read the settings file safely ({}); nothing was changed", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simple_name_keeps_letters_lowercase() {
        assert_eq!(simple_name("MCP_DOCKER"), "mcpdocker");
        assert_eq!(simple_name("MCP_DESK"), "mcpdesk");
    }

    #[test]
    fn connect_and_disconnect() {
        let dir = std::env::temp_dir().join("mcp-gateway-desk-test-home");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".codex")).unwrap();
        std::env::set_var("HOME", &dir);
        std::fs::write(dir.join(".claude.json"), r#"{"mcpServers":{"MCP_DOCKER":{"command":"docker","args":["mcp","gateway","run"]}},"other":1}"#).unwrap();
        std::fs::write(dir.join(".codex/config.toml"), "model = \"x\"\n\n[mcp_servers.other]\ncommand = \"a\"\n").unwrap();
        for id in ["claude-code", "codex", "vscode", "zed", "continue", "goose", "opencode"] {
            let c = CLIENTS.iter().find(|c| c.id == id).unwrap();
            c.connect().unwrap();
            assert_eq!(c.state().status, Status::Connected, "{}", id);
            c.disconnect().unwrap();
            assert_ne!(c.state().status, Status::Connected, "{}", id);
        }
        // Other entries in the file are left alone.
        let cc = std::fs::read_to_string(dir.join(".claude.json")).unwrap();
        assert!(cc.contains("MCP_DOCKER") && cc.contains("\"other\""));
        let toml = std::fs::read_to_string(dir.join(".codex/config.toml")).unwrap();
        assert!(toml.contains("[mcp_servers.other]") && !toml.contains("MCP_DESK"));
    }
}
