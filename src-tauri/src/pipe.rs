//! The named pipe `codenotch-hook` talks to: `\\.\pipe\codenotch-<sid>`.
//!
//! One instance per connection. Every hook event is folded into the live
//! session picture; `PermissionRequest` is the only one that keeps its
//! connection open, waiting for the notch's decision (see `approvals`) and
//! writing it back on the same pipe.
//!
//! The SID in the name keeps two accounts on one machine apart, and the relay
//! checks the server really runs as its own user before sending anything.
//! Remote clients are refused.

use std::time::Duration;

use serde_json::Value;
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// No hook payload is anywhere near this; the relay caps every field.
const MAX_PAYLOAD: usize = 1 << 20;

/// Must match `codenotch-hook`'s `pipe_path()` exactly.
pub fn pipe_name() -> String {
    let key = crate::platform::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\codenotch-{key}")
}

/// Start serving. Returns immediately; the server runs on the async runtime.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let name = pipe_name();
        // `first_pipe_instance` refuses to join a pipe somebody else already
        // owns under our name, rather than serving on top of it.
        let mut server = match ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
        {
            Ok(server) => server,
            Err(err) => {
                tracing::warn!(%err, "cannot open the hook pipe; Claude Code hooks will be ignored");
                return;
            }
        };
        loop {
            if server.connect().await.is_err() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            // Hand the connected instance to a task and listen on a fresh one.
            let next = match ServerOptions::new()
                .reject_remote_clients(true)
                .create(&name)
            {
                Ok(server) => server,
                Err(err) => {
                    tracing::warn!(%err, "cannot reopen the hook pipe");
                    return;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            let app = app.clone();
            tauri::async_runtime::spawn(async move { serve(app, connected).await });
        }
    });
}

async fn serve(app: AppHandle, mut pipe: NamedPipeServer) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') || buf.len() > MAX_PAYLOAD {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let line = match buf.iter().position(|b| *b == b'\n') {
        Some(i) => &buf[..i],
        None => &buf[..],
    };
    let Ok(payload) = serde_json::from_slice::<Value>(line) else {
        return;
    };

    let is_permission =
        payload.get("hook_event_name").and_then(Value::as_str) == Some("PermissionRequest");

    if !is_permission {
        let _ = pipe.disconnect();
        crate::hooks::on_event(&app, &payload);
        return;
    }

    // No decision: say nothing at all. The relay then prints nothing and
    // Claude Code asks in the terminal, exactly as if CodeNotch were closed.
    if let Some(decision) = crate::approvals::handle(&app, payload).await {
        let _ = pipe.write_all(format!("{decision}\n").as_bytes()).await;
        let _ = pipe.flush().await;
    }
    let _ = pipe.disconnect();
}
