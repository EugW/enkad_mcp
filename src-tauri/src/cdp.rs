//! In-process bridge to the planner webview's DevTools protocol.
//!
//! WebView2 lets the host call protocol methods directly on its own webview, so
//! there is no debugging port for other local processes to connect to.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{json, Value};
use tauri::{webview::PlatformWebview, Webview};
use tokio::sync::oneshot;
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
use windows_core::HSTRING;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);

type Reply = Result<String, String>;

#[derive(Clone)]
pub struct Cdp {
    webview: Webview,
}

impl Cdp {
    pub fn new(webview: Webview) -> Self {
        Self { webview }
    }

    /// Call one DevTools protocol method and return its result object.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel::<Reply>();
        let (method_name, params) = (method.to_owned(), params.to_string());
        // COM calls have to run on the UI thread; with_webview hops there.
        self.webview
            .with_webview(move |platform| dispatch(platform, &method_name, &params, tx))
            .map_err(|e| e.to_string())?;

        let reply = tokio::time::timeout(CALL_TIMEOUT, rx)
            .await
            .map_err(|_| format!("{method} timed out"))?
            .map_err(|_| format!("{method} was dropped before it completed"))?;
        let text = reply.map_err(|e| format!("{method} failed: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{method} returned invalid JSON: {e}"))
    }

    /// Evaluate an expression in the page and return its value. Promises are awaited.
    pub async fn eval(&self, expression: &str) -> Result<Value, String> {
        let reply = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        if let Some(details) = reply.get("exceptionDetails") {
            // The description is "Error: message" followed by a stack trace.
            let description = details
                .pointer("/exception/description")
                .or_else(|| details.get("text"))
                .and_then(Value::as_str)
                .and_then(|text| text.lines().next())
                .unwrap_or("unknown exception");
            return Err(description.trim_start_matches("Error: ").to_owned());
        }
        Ok(reply.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    /// Click at CSS pixel coordinates in the viewport, as real (trusted) mouse input.
    pub async fn click(&self, x: f64, y: f64) -> Result<(), String> {
        let event = |kind: &str, buttons: u8| {
            json!({ "type": kind, "x": x, "y": y, "button": "left", "buttons": buttons, "clickCount": 1 })
        };
        self.call("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y }))
            .await?;
        self.call("Input.dispatchMouseEvent", event("mousePressed", 1)).await?;
        self.call("Input.dispatchMouseEvent", event("mouseReleased", 0)).await?;
        Ok(())
    }

    /// Capture the viewport as a base64 JPEG, optionally scaled down.
    pub async fn screenshot(&self, scale: f64) -> Result<String, String> {
        let viewport = self
            .eval("({ width: innerWidth, height: innerHeight })")
            .await?;
        let clip = json!({
            "x": 0, "y": 0,
            "width": viewport["width"], "height": viewport["height"],
            "scale": scale,
        });
        let reply = self
            .call(
                "Page.captureScreenshot",
                json!({ "format": "jpeg", "quality": 80, "clip": clip }),
            )
            .await?;
        reply["data"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "Page.captureScreenshot returned no data".to_owned())
    }
}

fn dispatch(platform: PlatformWebview, method: &str, params: &str, tx: oneshot::Sender<Reply>) {
    // The sender is shared so that a call that fails synchronously can still report.
    let slot = Arc::new(Mutex::new(Some(tx)));
    let send = |slot: &Mutex<Option<oneshot::Sender<Reply>>>, reply: Reply| {
        if let Some(tx) = slot.lock().unwrap().take() {
            let _ = tx.send(reply);
        }
    };

    let handler_slot = slot.clone();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
        move |status, json: String| {
            let reply = match status {
                Ok(()) => Ok(json),
                Err(e) if json.is_empty() => Err(e.to_string()),
                Err(_) => Err(json),
            };
            send(&handler_slot, reply);
            Ok(())
        },
    ));

    let started = unsafe {
        platform.controller().CoreWebView2().and_then(|core| {
            core.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params), &handler)
        })
    };
    if let Err(e) = started {
        send(&slot, Err(e.to_string()));
    }
}
