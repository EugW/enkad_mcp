// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("enkad-mcp drives the planner through WebView2 and only builds on Windows");

mod cdp;
mod config;
mod mcp;

use tauri::{webview::WebviewBuilder, window::WindowBuilder, PhysicalPosition, WebviewUrl};

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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let cfg = config::load(app.handle())?;

            let window = WindowBuilder::new(app, "main")
                .title(format!(
                    "EnkaD MCP - {} - MCP on 127.0.0.1:{}",
                    cfg.channel.name(),
                    cfg.port
                ))
                .inner_size(1600.0, 1000.0)
                .build()?;
            // The planner is a child webview so other panes can sit beside it later.
            // It is remote content and gets no access to the app's own commands.
            let planner = window.add_child(
                WebviewBuilder::new("planner", WebviewUrl::External(cfg.channel.url().parse()?))
                    .data_directory(cfg.profile_dir())
                    .initialization_script(adapter_source())
                    .auto_resize(),
                PhysicalPosition::new(0, 0),
                window.inner_size()?,
            )?;

            let server =
                mcp::Planner::new(cdp::Cdp::new(planner), cfg.channel, cfg.snapshots_dir());
            let (port, token) = (cfg.port, cfg.token);
            tauri::async_runtime::spawn(async move {
                if let Err(e) = mcp::serve(server, port, token).await {
                    eprintln!("MCP server stopped: {e}");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start EnkaD MCP");
}
