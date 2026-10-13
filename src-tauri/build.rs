fn main() {
    // Listing the app's commands makes them subject to capabilities, so only the
    // webview named in capabilities/terminal.json can call them.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "terminal_start",
            "terminal_write",
            "terminal_resize",
            "terminal_drag",
        ]),
    ))
    .expect("tauri-build failed");
}
