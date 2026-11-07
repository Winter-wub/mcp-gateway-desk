//! The server catalog: servers that run straight on this Mac (npx / uvx, or remote servers
//! through the mcp-remote bridge). Remote servers' descriptions come from the original's
//! catalog file when it is on this Mac.

use crate::state::{home, Runner, SecretRef, Server};
use serde_yaml::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    /// "remote" for hosted servers, otherwise "local".
    pub kind: String,
    pub uses_folders: bool,
    pub secrets: Vec<SecretRef>,
    pub env: BTreeMap<String, String>,
    /// Scalar settings the server asks for (key -> description).
    pub config_keys: Vec<(String, String)>,
    pub local: (String, Vec<String>),
}

/// Servers we know how to start directly on the Mac (npx / uvx). `{{folders}}` becomes
/// one argument per shared folder. Package names are our own recipes (estimated).
pub fn local_recipe(name: &str) -> Option<(&'static str, Vec<&'static str>)> {
    Some(match name {
        "filesystem" => ("npx", vec!["-y", "@modelcontextprotocol/server-filesystem", "{{folders}}"]),
        "memory" => ("npx", vec!["-y", "@modelcontextprotocol/server-memory"]),
        "sequentialthinking" | "sequential-thinking" => {
            ("npx", vec!["-y", "@modelcontextprotocol/server-sequential-thinking"])
        }
        "everything" => ("npx", vec!["-y", "@modelcontextprotocol/server-everything"]),
        "fetch" => ("uvx", vec!["mcp-server-fetch"]),
        "git" => ("uvx", vec!["mcp-server-git"]),
        "time" => ("uvx", vec!["mcp-server-time"]),
        "playwright" => ("npx", vec!["-y", "@playwright/mcp@latest"]),
        "context7" => ("npx", vec!["-y", "@upstash/context7-mcp"]),
        "tavily" => ("npx", vec!["-y", "tavily-mcp"]),
        "desktop-commander" => ("npx", vec!["-y", "@wonderwhy-er/desktop-commander"]),
        _ => return None,
    })
}

/// Servers that should get the shared folders even if the catalog doesn't say so.
pub fn wants_folders(name: &str) -> bool {
    matches!(name, "filesystem" | "git" | "desktop-commander")
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(|x| x.as_sequence())
        .map(|seq| seq.iter().filter_map(|i| i.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn load() -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    // Server descriptions from the catalog file the original keeps on this Mac.
    let dir = home().join(".docker/mcp/catalogs");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for f in rd.flatten() {
            let p = f.path();
            if p.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    if let Ok(doc) = serde_yaml::from_str::<Value>(&text) {
                        parse_registry(&doc, &mut out);
                    }
                }
            }
        }
    }
    // Add recipes / sample entries that the Docker catalog didn't provide.
    for (name, title, desc, cat) in SAMPLES {
        if !out.iter().any(|e| e.name == *name) {
            let Some(local) = local_recipe(name).map(|(c, a)| (c.to_string(), a.iter().map(|x| x.to_string()).collect())) else { continue };
            out.push(Entry {
                name: name.to_string(),
                title: title.to_string(),
                description: desc.to_string(),
                category: cat.to_string(),
                kind: "local".into(),
                uses_folders: wants_folders(name),
                secrets: vec![],
                env: BTreeMap::new(),
                config_keys: vec![],
                local,
            });
        }
    }
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    out
}

fn parse_registry(doc: &Value, out: &mut Vec<Entry>) {
    let Some(reg) = doc.get("registry").and_then(|r| r.as_mapping()) else { return };
    for (k, v) in reg {
        let Some(name) = k.as_str() else { continue };
        if out.iter().any(|e| e.name == name) {
            continue;
        }
        let kind = s(v, "type");
        let volumes = str_list(v.get("volumes"));
        let command = str_list(v.get("command"));
        let uses_folders = volumes.iter().chain(command.iter()).any(|x| x.contains("paths"))
            || wants_folders(name);
        let secrets = v
            .get("secrets")
            .and_then(|x| x.as_sequence())
            .map(|seq| {
                seq.iter()
                    .map(|i| SecretRef { name: s(i, "name"), env: s(i, "env") })
                    .filter(|r| !r.name.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let mut env = BTreeMap::new();
        if let Some(seq) = v.get("env").and_then(|x| x.as_sequence()) {
            for i in seq {
                let n = s(i, "name");
                if !n.is_empty() {
                    env.insert(n, s(i, "value"));
                }
            }
        }
        let mut config_keys = Vec::new();
        let cfg = v.get("config");
        let cfg_first = match cfg {
            Some(Value::Sequence(seq)) => seq.first(),
            other => other,
        };
        if let Some(props) = cfg_first.and_then(|c| c.get("properties")).and_then(|p| p.as_mapping()) {
            for (pk, pv) in props {
                let Some(pk) = pk.as_str() else { continue };
                if s(pv, "type") == "array" {
                    continue; // lists of paths become the shared folders
                }
                config_keys.push((pk.to_string(), s(pv, "description")));
            }
        }
        let remote_url = v.get("remote").map(|r| s(r, "url")).unwrap_or_default();
        // Only servers that can run on this Mac are kept: our recipes, and remote
        // servers through the mcp-remote bridge. Container-only servers are left out.
        let Some(local) = local_recipe(name)
            .map(|(c, a)| (c.to_string(), a.iter().map(|x| x.to_string()).collect::<Vec<_>>()))
            .or_else(|| {
                (!remote_url.is_empty()).then(|| ("npx".to_string(), vec!["-y".into(), "mcp-remote".into(), remote_url.clone()]))
            })
        else {
            continue;
        };
        let category = v
            .get("metadata")
            .map(|m| s(m, "category"))
            .unwrap_or_default();
        out.push(Entry {
            name: name.to_string(),
            title: { let t = s(v, "title"); if t.is_empty() { name.to_string() } else { t } },
            description: s(v, "description"),
            category,
            kind: if kind == "remote" { "remote".into() } else { "local".into() },
            uses_folders,
            secrets,
            env,
            config_keys,
            local,
        });
    }
}

impl Entry {
    pub fn to_server(&self) -> Server {
        let runner = Runner::Local { command: self.local.0.clone(), args: self.local.1.clone() };
        let mut config = BTreeMap::new();
        for (k, _) in &self.config_keys {
            config.insert(k.clone(), String::new());
        }
        Server {
            name: self.name.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            category: self.category.clone(),
            runner,
            enabled: true,
            uses_folders: self.uses_folders,
            env: self.env.clone(),
            config,
            secrets: self.secrets.clone(),
            known_tools: vec![],
            disabled_tools: Default::default(),
        }
    }
}

/// Shown when Docker's catalog isn't on this Mac (and to fill in run-on-Mac servers).
const SAMPLES: &[(&str, &str, &str, &str)] = &[
    ("filesystem", "Filesystem", "Read, write and search files in your shared folders.", "devops"),
    ("memory", "Memory", "A small knowledge graph the assistant can remember things in.", "productivity"),
    ("fetch", "Fetch", "Fetch web pages and turn them into text.", "search"),
    ("git", "Git", "Read and work with git repositories in your shared folders.", "devops"),
    ("time", "Time", "Current time and time-zone conversion.", "productivity"),
    ("sequentialthinking", "Sequential Thinking", "Step-by-step problem solving.", "productivity"),
    ("playwright", "Playwright", "Drive a web browser.", "devops"),
    ("context7", "Context7", "Up-to-date library documentation.", "devops"),
    ("desktop-commander", "Desktop Commander", "Run terminal commands and edit files on this Mac.", "devops"),
    ("everything", "Everything (test)", "A test server that exercises every MCP feature.", "devops"),
];
