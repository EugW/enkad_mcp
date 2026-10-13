// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("enkad-mcp drives the planner through WebView2 and only builds on Windows");

mod cdp;
mod config;
mod mcp;
mod panes;
mod terminal;

use std::{path::Path, sync::Arc, time::Duration};

use tauri::{
    menu::{CheckMenuItemBuilder, MenuBuilder, SubmenuBuilder},
    webview::WebviewBuilder,
    window::WindowBuilder,
    Manager, PhysicalPosition, WebviewUrl, WindowEvent,
};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::{panes::Panes, terminal::Shell};

const MENU_CHANNEL_BETA: &str = "channel-beta";
const MENU_CHANNEL_RELEASE: &str = "channel-release";
const MENU_COPY_COMMAND: &str = "copy-claude-command";
const MENU_COPY_CODEX: &str = "copy-codex-command";
const MENU_COPY_OPENCODE: &str = "copy-opencode-command";
const MENU_COPY_JSON: &str = "copy-json-config";
const MENU_OPEN_SNAPSHOTS: &str = "open-snapshots";
const MENU_TERMINAL_SHOW: &str = "terminal-show";
const MENU_OPEN_WORKSPACE: &str = "open-workspace";
const MENU_RELOAD: &str = "reload-planner";

/// Dev builds read the adapter from disk at start-up so it can change without a recompile.
fn adapter_source() -> String {
    #[cfg(debug_assertions)]
    if let Ok(source) =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../adapter/adapter.js"))
    {
        return source;
    }
    include_str!("../../adapter/adapter.js").to_owned()
}

fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

fn main() {
    let mut builder = tauri::Builder::default();
    // A second launch would find the MCP port taken; bring the first window forward
    // instead. A copy pointed at its own data folder is a separate instance on purpose.
    if std::env::var_os(config::DATA_DIR_OVERRIDE).is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }));
    }
    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            terminal::terminal_start,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_drag,
        ])
        .setup(|app| {
            let cfg = config::load(app.handle())?;
            cfg.prepare_workspace()?;
            let title = format!("EnkaD MCP - {} - MCP on 127.0.0.1:{}", cfg.channel.name(), cfg.port);

            let beta = CheckMenuItemBuilder::with_id(MENU_CHANNEL_BETA, "Beta")
                .checked(true)
                .build(app)?;
            let release = CheckMenuItemBuilder::with_id(MENU_CHANNEL_RELEASE, "Release (no adapter yet)")
                .checked(false)
                .enabled(false)
                .build(app)?;
            let show_terminal = CheckMenuItemBuilder::with_id(MENU_TERMINAL_SHOW, "Show terminal")
                .checked(cfg.terminal_visible)
                .build(app)?;
            let menu = MenuBuilder::new(app)
                .item(
                    &SubmenuBuilder::new(app, "Channel")
                        .item(&beta)
                        .item(&release)
                        .build()?,
                )
                .item(
                    &SubmenuBuilder::new(app, "MCP")
                        .text(MENU_COPY_COMMAND, "Copy Claude Code command")
                        .text(MENU_COPY_CODEX, "Copy Codex command")
                        .text(MENU_COPY_OPENCODE, "Copy opencode command")
                        .text(MENU_COPY_JSON, "Copy server config (JSON)")
                        .separator()
                        .text(MENU_OPEN_SNAPSHOTS, "Open snapshots folder")
                        .build()?,
                )
                .item(
                    &SubmenuBuilder::new(app, "Terminal")
                        .item(&show_terminal)
                        .separator()
                        .text(MENU_OPEN_WORKSPACE, "Open workspace folder")
                        .build()?,
                )
                .item(
                    &SubmenuBuilder::new(app, "Planner")
                        .text(MENU_RELOAD, "Reload")
                        .build()?,
                )
                .build()?;

            let window = WindowBuilder::new(app, "main")
                .title(&title)
                .inner_size(1600.0, 1000.0)
                .menu(menu)
                .build()?;
            // Two child webviews share the window. The planner is remote content and
            // gets no access to the app's commands; the terminal is a local page that
            // may only call the commands listed in capabilities/terminal.json.
            let origin = PhysicalPosition::new(0, 0);
            let planner = window.add_child(
                WebviewBuilder::new("planner", WebviewUrl::External(cfg.channel.url().parse()?))
                    .data_directory(cfg.profile_dir())
                    .initialization_script(adapter_source()),
                origin,
                window.inner_size()?,
            )?;
            let terminal = window.add_child(
                WebviewBuilder::new("terminal", WebviewUrl::App("terminal.html".into()))
                    .data_directory(cfg.profile_dir()),
                origin,
                window.inner_size()?,
            )?;
            let panes = Arc::new(Panes::new(
                window.clone(),
                planner.clone(),
                terminal,
                cfg.terminal_visible,
                cfg.terminal_width,
                cfg.data_dir.clone(),
            ));
            panes.layout();
            app.manage(panes.clone());
            app.manage(Shell::new(
                cfg.workspace_dir(),
                vec![
                    ("ENKAD_MCP_URL".to_owned(), cfg.mcp_url()),
                    ("ENKAD_MCP_TOKEN".to_owned(), cfg.token.clone()),
                ],
            ));
            window.on_window_event({
                let panes = panes.clone();
                move |event| {
                    if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
                        panes.layout();
                    }
                }
            });

            let claude_command = format!(
                "claude mcp add --transport http enkad {} --header \"Authorization: Bearer {}\"",
                cfg.mcp_url(),
                cfg.token
            );
            // Codex takes the token from the environment; the app's terminal sets that variable.
            let codex_command = format!(
                "codex mcp add enkad --url {} --bearer-token-env-var ENKAD_MCP_TOKEN",
                cfg.mcp_url()
            );
            let opencode_command = format!(
                "opencode mcp add enkad --url {} --header \"Authorization=Bearer {}\"",
                cfg.mcp_url(),
                cfg.token
            );
            let json_config = serde_json::to_string_pretty(&cfg.mcp_servers_json())?;
            let (snapshots, workspace) = (cfg.snapshots_dir(), cfg.workspace_dir());
            let reloadable = planner.clone();
            let restore_title = title.clone();
            window.on_menu_event(move |window, event| {
                let copy = |text: &str| {
                    if window.app_handle().clipboard().write_text(text).is_ok() {
                        // The window has no status bar; say so in the title for a moment.
                        let _ = window.set_title("EnkaD MCP - copied to clipboard");
                        let (window, title) = (window.clone(), restore_title.clone());
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            let _ = window.set_title(&title);
                        });
                    }
                };
                match event.id().as_ref() {
                    MENU_COPY_COMMAND => copy(&claude_command),
                    MENU_COPY_CODEX => copy(&codex_command),
                    MENU_COPY_OPENCODE => copy(&opencode_command),
                    MENU_COPY_JSON => copy(&json_config),
                    MENU_OPEN_SNAPSHOTS => open_folder(&snapshots),
                    MENU_OPEN_WORKSPACE => open_folder(&workspace),
                    MENU_TERMINAL_SHOW => {
                        // The click has already flipped the tick.
                        panes.set_visible(show_terminal.is_checked().unwrap_or(!panes.visible()));
                    }
                    MENU_RELOAD => {
                        let _ = reloadable.reload();
                    }
                    // Beta is the only channel with an adapter, so it stays ticked.
                    MENU_CHANNEL_BETA => {
                        let _ = beta.set_checked(true);
                    }
                    _ => {}
                }
            });

            let server = mcp::Planner::new(
                cdp::Cdp::new(planner),
                cfg.channel,
                cfg.snapshots_dir(),
                app.handle().clone(),
            );
            let (port, token) = (cfg.port, cfg.token);
            tauri::async_runtime::spawn(async move {
                if let Err(e) = mcp::serve(server, port, token).await {
                    eprintln!("MCP server stopped: {e}");
                    let _ = window.set_title(&format!("EnkaD MCP - MCP server not running: {e}"));
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start EnkaD MCP");
}
