//! Sign-in (OAuth) for remote servers. Remote servers run through the `mcp-remote` bridge,
//! which does the browser sign-in and keeps the token in ~/.mcp-auth/mcp-remote-v1/
//! as `<md5 of the server URL>_tokens.json`.
//!
//! Like the original, the gateway checks that a token exists before it uses a server that
//! needs one, so a missing sign-in never holds
//! up an AI app; signing in happens here in the window instead.

use crate::state::{home, Runner, Server};
use std::path::PathBuf;

/// How long a sign-in may take before we give up.
pub const SIGN_IN_TIMEOUT_SECS: u64 = 300;

/// What we learn from the bridge's messages while a server starts.
#[derive(Default, Clone)]
pub struct Progress {
    /// The sign-in page the bridge asked to open.
    pub auth_url: Option<String>,
    /// Another copy of the bridge is already signing in to this server.
    pub other_signin: bool,
    want_url: bool,
}

impl Progress {
    /// Read one stderr line from the bridge.
    pub fn feed(&mut self, line: &str) {
        let t = line.trim();
        if t.contains("Please authorize this client by visiting") {
            self.want_url = true;
            return;
        }
        if self.want_url && t.starts_with("http") {
            self.auth_url = Some(t.to_string());
            self.want_url = false;
        }
        if t.contains("Another instance is running the sign-in") {
            self.other_signin = true;
        }
    }
}

/// The remote address when a server runs through `mcp-remote`.
pub fn remote_url(server: &Server) -> Option<String> {
    let Runner::Local { args, .. } = &server.runner;
    let i = args.iter().position(|a| a.starts_with("mcp-remote"))?;
    args[i + 1..].iter().find(|a| a.starts_with("http")).cloned()
}

fn config_dir() -> PathBuf {
    let base = std::env::var_os("MCP_REMOTE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".mcp-auth"));
    base.join("mcp-remote-v1")
}

fn token_file(url: &str) -> PathBuf {
    config_dir().join(format!("{:x}_tokens.json", md5::compute(url.as_bytes())))
}

pub fn signed_in(url: &str) -> bool {
    token_file(url).is_file()
}

/// Forget the saved token, so the next start asks to sign in again.
pub fn sign_out(url: &str) {
    let _ = std::fs::remove_file(token_file(url));
}

/// Stop every bridge copy for this server — a forgotten sign-in keeps the callback port
/// busy and makes every later start wait.
pub fn stop_other_signins(url: &str) -> usize {
    let pattern = format!("mcp-remote {}", url);
    let found = std::process::Command::new("pgrep")
        .args(["-f", &pattern])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0);
    let _ = std::process::Command::new("pkill").args(["-f", &pattern]).status();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_file_name_matches_the_bridge() {
        let f = token_file("https://mcp.atlassian.com/v1/mcp");
        assert!(f.ends_with("mcp-remote-v1/01910c24c5f2edcaf999bd1eaaeaeee8_tokens.json"), "{}", f.display());
    }

    #[test]
    fn reads_sign_in_messages() {
        let mut p = Progress::default();
        p.feed("[482] Connecting to remote server");
        p.feed("Please authorize this client by visiting:");
        p.feed("https://mcp.atlassian.com/v1/authorize?response_type=code");
        assert_eq!(p.auth_url.as_deref(), Some("https://mcp.atlassian.com/v1/authorize?response_type=code"));
        p.feed("[7039] Another instance is running the sign-in for this server on port 3736");
        assert!(p.other_signin);
    }
}
