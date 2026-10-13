//! The terminal pane: a shell in a pseudo-terminal, shown by the local `terminal` webview.
//!
//! Only that webview may call these commands (see capabilities/terminal.json).
//! The planner webview is remote content and has no access to them.

use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    State,
};

use crate::panes::Panes;

pub struct Shell {
    /// Folder the shell starts in.
    workspace: PathBuf,
    /// Extra environment, so an agent started in the shell can find the MCP server.
    env: Vec<(String, String)>,
    session: Mutex<Option<Session>>,
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Shell {
    pub fn new(workspace: PathBuf, env: Vec<(String, String)>) -> Self {
        Self {
            workspace,
            env,
            session: Mutex::new(None),
        }
    }

    /// Type text into the running shell. Only the dev tools use this; the app
    /// itself never drives the terminal.
    pub fn type_text(&self, text: &str) {
        if let Some(session) = self.session.lock().unwrap().as_mut() {
            let _ = session.writer.write_all(text.as_bytes());
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        if let Some(mut session) = self.session.lock().unwrap().take() {
            let _ = session.killer.kill();
        }
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// PowerShell 7 when installed, otherwise the Windows PowerShell every system has.
fn shell_program() -> &'static str {
    let on_path = std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("pwsh.exe").is_file())
    });
    if on_path {
        "pwsh.exe"
    } else {
        "powershell.exe"
    }
}

/// Start a shell, replacing any that is running. Output arrives on `output` as raw
/// bytes; a final `{"exited":true}` says the shell has ended.
#[tauri::command]
pub fn terminal_start(
    shell: State<'_, Shell>,
    output: Channel<InvokeResponseBody>,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let mut session = shell.session.lock().unwrap();
    if let Some(mut old) = session.take() {
        let _ = old.killer.kill();
    }

    let pair = native_pty_system()
        .openpty(size(cols, rows))
        .map_err(|e| e.to_string())?;
    let mut command = CommandBuilder::new(shell_program());
    command.cwd(&shell.workspace);
    for (key, value) in &shell.env {
        command.env(key, value);
    }
    let mut child = pair.slave.spawn_command(command).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let killer = child.clone_killer();

    let shown = output.clone();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 || shown.send(InvokeResponseBody::Raw(buffer[..n].to_vec())).is_err() {
                break;
            }
        }
    });
    // On Windows the reader does not end when the shell exits, so watch the process itself.
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = output.send(InvokeResponseBody::Json(r#"{"exited":true}"#.to_owned()));
    });

    *session = Some(Session {
        master: pair.master,
        writer,
        killer,
    });
    Ok(())
}

#[tauri::command]
pub fn terminal_write(shell: State<'_, Shell>, data: String) {
    if let Some(session) = shell.session.lock().unwrap().as_mut() {
        let _ = session.writer.write_all(data.as_bytes());
    }
}

#[tauri::command]
pub fn terminal_resize(shell: State<'_, Shell>, cols: u16, rows: u16) {
    if let Some(session) = shell.session.lock().unwrap().as_ref() {
        let _ = session.master.resize(size(cols, rows));
    }
}

/// The user is dragging the pane's left edge: `delta` is how far, in physical
/// pixels, the edge still is from the pointer. `done` marks the end of the drag.
#[tauri::command]
pub fn terminal_drag(panes: State<'_, Arc<Panes>>, delta: f64, done: bool) {
    panes.drag(delta);
    if done {
        panes.remember();
    }
}
