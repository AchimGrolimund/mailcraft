//! MailCraft's MCP server (Model Context Protocol, JSON-RPC 2.0 over stdio).
//!
//! Two backends: [`Headless`] runs an in-process session on a mailbox (no window);
//! [`backend::Remote`] talks to a running MailCraft app through its control channel
//! (`mailcraft --control PORT`), so agents can also click, type and screenshot the real UI.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod backend;
pub mod server;
pub mod tools;

pub use backend::{Backend, Remote};
pub use server::Server;

use mailcraft_engine::Session;
use serde_json::{Value, json};

/// Methods that need the desktop app.
pub const NEEDS_APP: &str = "needs the desktop app: start `mailcraft --control 7991` and run `mailcraft-cli mcp --connect 127.0.0.1:7991`";

/// An in-process session.
pub struct Headless {
    pub session: Session,
}

impl Default for Headless {
    fn default() -> Self {
        Headless { session: Session::empty() }
    }
}

impl Backend for Headless {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "engine.execute" | "command" => {
                let id = params.get("command").and_then(Value::as_str).ok_or("missing `command`")?.to_string();
                let p = params.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
                self.session.run(&id, &p).map_err(|e| e.to_string())
            }
            "engine.commands" => Ok(self.session.registry.describe()),
            "mailbox.inspect" => self.session.run("file.info", &json!({})).map_err(|e| e.to_string()),
            "ui.parity" => Ok(mailcraft_engine::catalog::parity(&self.session.registry)),
            m if m.starts_with("ui.") => Err(format!("`{m}` {NEEDS_APP}")),
            other => self.session.run(other, &params).map_err(|e| e.to_string()),
        }
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "headless session (no window)".into()
    }
}

#[cfg(test)]
mod tests;
