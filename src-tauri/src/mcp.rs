//! The MCP server: tools that read and drive the planner webview.

use std::{fs, path::PathBuf, sync::Arc};

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{cdp::Cdp, config::Channel};

#[derive(Clone)]
pub struct Planner {
    cdp: Cdp,
    channel: Channel,
    /// Folder holding saved plan states, one JSON file each.
    snapshots: PathBuf,
    router: ToolRouter<Self>,
}

impl Planner {
    pub fn new(cdp: Cdp, channel: Channel, snapshots: PathBuf) -> Self {
        let mut router = Self::tool_router();
        // Raw page access is for adapter development only and never ships.
        if cfg!(debug_assertions) {
            router += Self::dev_tool_router();
        }
        Self { cdp, channel, snapshots, router }
    }

    /// Run one adapter function in the planner page and hand its result to the client.
    async fn adapter(&self, name: &str, args: impl Serialize) -> CallToolResult {
        match self.adapter_value(name, args).await {
            Ok(value) => ok(value),
            Err(e) => fail(e),
        }
    }

    async fn adapter_value(&self, name: &str, args: impl Serialize) -> Result<Value, String> {
        // Absent options must stay absent so the adapter's defaults apply.
        let mut args = serde_json::to_value(args).unwrap_or(Value::Null);
        if let Value::Object(fields) = &mut args {
            fields.retain(|_, value| !value.is_null());
        }
        let expression = format!(
            "window.__enkad ? window.__enkad.call({}, {args}) : Promise.reject(new Error('the adapter is not loaded in the planner page'))",
            json!(name)
        );
        self.cdp.eval(&expression).await
    }

    /// Write the open plan to the snapshot folder and return its summary.
    async fn save_snapshot(&self, label: &str, automatic: bool) -> Result<Value, String> {
        let mut snapshot = self.adapter_value("exportBlueprint", json!({})).await?;
        // "2026-10-13T12:34:56.789Z" becomes "20261013-123456-789".
        let digits: String = snapshot["time"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        if digits.len() != 17 {
            return Err("the planner returned no usable timestamp".to_owned());
        }
        let id = format!("{}-{}-{}", &digits[..8], &digits[8..14], &digits[14..]);
        snapshot["id"] = json!(id);
        snapshot["label"] = json!(label);
        snapshot["automatic"] = json!(automatic);

        let text = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.snapshots)
            .and_then(|()| fs::write(self.snapshots.join(format!("{id}.json")), text))
            .map_err(|e| format!("cannot write snapshot: {e}"))?;
        if automatic {
            self.prune_automatic_snapshots();
        }
        Ok(summary(&snapshot))
    }

    /// Keep only the newest automatic snapshots; ones the user asked for are never removed.
    fn prune_automatic_snapshots(&self) {
        let mut automatic: Vec<PathBuf> = fs::read_dir(&self.snapshots)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                fs::read_to_string(path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .is_some_and(|snapshot| snapshot["automatic"] == json!(true))
            })
            .collect();
        // File names are timestamps, so name order is age order.
        automatic.sort();
        let excess = automatic.len().saturating_sub(KEEP_AUTOMATIC_SNAPSHOTS);
        for path in automatic.into_iter().take(excess) {
            let _ = fs::remove_file(path);
        }
    }

    /// Replace the open plan, saving its current state first so nothing is lost.
    async fn replace_plan(&self, blueprint: &str, reason: &str) -> CallToolResult {
        let saved = match self.save_snapshot(reason, true).await {
            Ok(saved) => saved,
            Err(e) => return fail(format!("plan left unchanged, could not save it first: {e}")),
        };
        let imported = self
            .adapter_value("importBlueprint", json!({ "blueprint": blueprint }))
            .await;
        match imported {
            Ok(plan) => ok(json!({ "plan": plan, "previousStateSavedAs": saved["id"] })),
            Err(e) => fail(e),
        }
    }
}

const KEEP_AUTOMATIC_SNAPSHOTS: usize = 40;

/// A snapshot without its blueprint string.
fn summary(snapshot: &Value) -> Value {
    let mut summary = snapshot.clone();
    if let Value::Object(fields) = &mut summary {
        fields.remove("blueprint");
    }
    summary
}

fn ok(value: Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

fn fail(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

/// A rectangle of planner cells.
#[derive(Deserialize, Serialize, JsonSchema)]
struct Region {
    /// Left cell.
    x: i32,
    /// Top cell.
    y: i32,
    /// Width in cells.
    w: i32,
    /// Height in cells.
    h: i32,
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct LayoutArgs {
    /// Only entities whose footprint touches this rectangle. Omit for the whole plan.
    region: Option<Region>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct GraphArgs {
    /// Only connections into or out of these building uids. Omit for the whole plan.
    uids: Option<Vec<u32>>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct FlowsArgs {
    /// Only these building uids. Omit for the whole plan.
    uids: Option<Vec<u32>>,
    /// Only buildings whose footprint touches this rectangle.
    region: Option<Region>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct HistoryArgs {
    /// Building uids to report.
    uids: Vec<u32>,
    /// First simulation tick. Defaults to 0.
    from: Option<u32>,
    /// Last simulation tick. Defaults to the end of the simulated run.
    to: Option<u32>,
    /// Window length in ticks. Defaults to 1200 (5 minutes).
    window: Option<u32>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct CatalogArgs {
    /// A building type name or id. Give it to list the recipes of that type; omit to list all types.
    #[serde(rename = "type")]
    building: Option<String>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum PathKind {
    Belt,
    Pipe,
}

/// One change to the open plan.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
enum EditOp {
    /// Place a new building.
    AddBuilding {
        /// Building type name or id, from `get_catalog`.
        #[serde(rename = "type")]
        building: String,
        /// Left cell of its footprint.
        x: i32,
        /// Top cell of its footprint.
        y: i32,
        /// 0, 90, 180 or 270. Defaults to 0. Copy the rotation of a similar building in `get_layout` to match its port sides.
        rotation: Option<i32>,
        /// Recipe id, or the name of an item the recipe makes. Omit to leave unset.
        recipe: Option<String>,
    },
    /// Lay a new belt or pipe.
    AddPath {
        /// Defaults to belt.
        kind: Option<PathKind>,
        /// "x,y>x,y>...>x,y": the cell to take items from (the output side of a building), then the first cell of the path, its corners and its last cell, then the cell to deliver into. Same format as `route` in `get_layout`.
        route: String,
    },
    /// Remove buildings, belts or pipes.
    Delete { uids: Vec<u32> },
    /// Shift entities together by a number of cells.
    Move {
        uids: Vec<u32>,
        /// Cells to the right (negative for left).
        dx: i32,
        /// Cells down (negative for up).
        dy: i32,
    },
    /// Turn entities a quarter turn, as a group around their common centre.
    Rotate {
        uids: Vec<u32>,
        /// Defaults to true.
        clockwise: Option<bool>,
    },
    /// Choose what a building makes.
    SetRecipe {
        uid: u32,
        /// Recipe id, or the name of an item the recipe makes. Omit to clear.
        recipe: Option<String>,
    },
}

#[derive(Deserialize, Serialize, JsonSchema)]
struct EditArgs {
    /// Edits to apply in order.
    ops: Vec<EditOp>,
}

#[derive(Deserialize, JsonSchema)]
struct ImportArgs {
    /// Blueprint string as produced by `export_blueprint` or the planner's own copy and share.
    blueprint: String,
}

#[derive(Deserialize, JsonSchema)]
struct SnapshotCreateArgs {
    /// Short note on why this state is worth keeping, e.g. "before compacting the splitter array".
    label: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct SnapshotRestoreArgs {
    /// Snapshot id from `snapshot_create` or `snapshot_list`.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct ScreenshotArgs {
    /// Downscale factor between 0.1 and 1. Defaults to 0.5.
    scale: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
struct EvalArgs {
    /// JavaScript expression. Its value must be JSON-serialisable; promises are awaited.
    expression: String,
}

#[derive(Deserialize, JsonSchema)]
struct ClickArgs {
    /// Horizontal CSS pixel position in the planner view.
    x: f64,
    /// Vertical CSS pixel position in the planner view.
    y: f64,
}

/// Adapter health plus a summary of the open plan, in one round trip.
const STATUS_SCRIPT: &str = r#"(async () => {
  if (!window.__enkad) return { adapter: null };
  const out = { adapter: window.__enkad.api.health() };
  if (out.adapter.ok) {
    try { Object.assign(out, await window.__enkad.call('overview')); }
    catch (e) { out.plan = { error: e.message }; }
  }
  return out;
})()"#;

#[tool_router]
impl Planner {
    #[tool(
        description = "Report the planner channel, whether the adapter can reach the planner's internals, and a summary of the open plan (size, entity counts, simulation state). Call this first: the other tools only work when `adapter.ok` is true."
    )]
    async fn status(&self) -> CallToolResult {
        match self.cdp.eval(STATUS_SCRIPT).await {
            Ok(mut value) => {
                value["app"] = json!(env!("CARGO_PKG_VERSION"));
                value["channel"] = json!(self.channel.name());
                ok(value)
            }
            Err(e) => fail(e),
        }
    }

    #[tool(
        description = "List what is placed in the open plan: buildings (uid, name, position, size, rotation, active recipe) and belts and pipes (uid, kind, length, route as corner cells from source to sink). Positions are planner cells: x grows right, y grows down, and an entity's position is the top-left cell of its footprint."
    )]
    async fn get_layout(&self, Parameters(args): Parameters<LayoutArgs>) -> CallToolResult {
        self.adapter("layout", args).await
    }

    #[tool(
        description = "List the connections of the open plan: which building output port feeds which building input port, through which belts or pipes (`via` uids, empty when the two buildings touch), and the items per minute flowing there."
    )]
    async fn get_graph(&self, Parameters(args): Parameters<GraphArgs>) -> CallToolResult {
        self.adapter("graph", args).await
    }

    #[tool(
        description = "Report simulated throughput at the current tick, in items per minute: what each building takes in and puts out, which buildings are stalled (idle or clogged), ports the simulation has marked (for example a low-priority input that is being starved), and what the depot gains and loses overall. A `~` after a rate means it has not settled."
    )]
    async fn get_flows(&self, Parameters(args): Parameters<FlowsArgs>) -> CallToolResult {
        self.adapter("flows", args).await
    }

    #[tool(
        description = "Report how many items moved into and out of the given buildings over the simulated run, summed per time window. Use it to check start-up behaviour and whether a rate is sustained rather than reached once. The simulation runs at 240 ticks per minute."
    )]
    async fn get_history(&self, Parameters(args): Parameters<HistoryArgs>) -> CallToolResult {
        self.adapter("history", args).await
    }

    #[tool(
        description = "List the building and path types that can be placed (id, name, layer, size, port counts), or with `type` the recipes of one building type."
    )]
    async fn get_catalog(&self, Parameters(args): Parameters<CatalogArgs>) -> CallToolResult {
        self.adapter("catalog", args).await
    }

    #[tool(
        description = "Change the open plan: add buildings, belts and pipes, delete, move, rotate, set recipes. The edits are applied in order as one batch: if any of them fails (for example the target cells are occupied) none is applied. The batch is one undo step in the planner, and the previous state of the plan is saved as a snapshot first. The planner applies its own placement rules. Afterwards check the result with `get_graph` and `get_flows`: the simulation needs a moment to re-solve, shown as `solving`. Fails on a read-only plan."
    )]
    async fn apply_edits(&self, Parameters(args): Parameters<EditArgs>) -> CallToolResult {
        let saved = match self.save_snapshot("before apply_edits", true).await {
            Ok(saved) => saved,
            Err(e) => return fail(format!("plan left unchanged, could not save it first: {e}")),
        };
        match self.adapter_value("edit", args).await {
            Ok(mut result) => {
                result["previousStateSavedAs"] = saved["id"].clone();
                ok(result)
            }
            Err(e) => fail(e),
        }
    }

    #[tool(
        description = "Return the open plan as a blueprint string, the format the planner's own paste and import accept."
    )]
    async fn export_blueprint(&self) -> CallToolResult {
        self.adapter("exportBlueprint", json!({})).await
    }

    #[tool(
        description = "Replace the contents of the open plan with a blueprint string. The current state is saved as a snapshot first, and the change is one undo step in the planner. The plan keeps its own area size. Entities may get new uids, so read the layout again afterwards. Fails on a read-only plan."
    )]
    async fn import_blueprint(&self, Parameters(args): Parameters<ImportArgs>) -> CallToolResult {
        self.replace_plan(&args.blueprint, "before import_blueprint").await
    }

    #[tool(
        description = "Save the current state of the open plan to disk so it can be restored later. Take one before a series of changes."
    )]
    async fn snapshot_create(
        &self,
        Parameters(args): Parameters<SnapshotCreateArgs>,
    ) -> CallToolResult {
        match self.save_snapshot(args.label.as_deref().unwrap_or(""), false).await {
            Ok(saved) => ok(saved),
            Err(e) => fail(e),
        }
    }

    #[tool(description = "List saved snapshots, newest first.")]
    async fn snapshot_list(&self) -> CallToolResult {
        let mut rows: Vec<Value> = fs::read_dir(&self.snapshots)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|snapshot| {
                let counts = &snapshot["counts"];
                json!([
                    snapshot["id"],
                    snapshot["label"],
                    snapshot["plan"],
                    counts["building"],
                    counts["belt"],
                    counts["pipe"],
                ])
            })
            .collect();
        rows.sort_by(|a, b| b[0].as_str().cmp(&a[0].as_str()));
        ok(json!({
            "columns": ["id", "label", "plan", "buildings", "belts", "pipes"],
            "rows": rows,
            "note": "the id is the UTC time the snapshot was taken",
        }))
    }

    #[tool(
        description = "Put the open plan back to a saved snapshot. The state being replaced is itself saved first, and the change is one undo step in the planner. Entities may get new uids, so read the layout again afterwards. Fails on a read-only plan."
    )]
    async fn snapshot_restore(
        &self,
        Parameters(args): Parameters<SnapshotRestoreArgs>,
    ) -> CallToolResult {
        // Ids are digits and dashes only, which also keeps the path inside the folder.
        if args.id.is_empty() || !args.id.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return fail("not a snapshot id");
        }
        let path = self.snapshots.join(format!("{}.json", args.id));
        let snapshot = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let Some(blueprint) = snapshot.as_ref().and_then(|s| s["blueprint"].as_str()) else {
            return fail(format!("no snapshot with id {}", args.id));
        };
        self.replace_plan(blueprint, &format!("before restoring {}", args.id))
            .await
    }

    #[tool(description = "Capture the planner view as a JPEG image.")]
    async fn screenshot(&self, Parameters(args): Parameters<ScreenshotArgs>) -> CallToolResult {
        let scale = args.scale.unwrap_or(0.5).clamp(0.1, 1.0);
        match self.cdp.screenshot(scale).await {
            Ok(data) => CallToolResult::success(vec![ContentBlock::image(data, "image/jpeg")]),
            Err(e) => fail(e),
        }
    }
}

#[tool_router(router = dev_tool_router)]
impl Planner {
    #[tool(
        description = "Dev builds only. Evaluate a JavaScript expression in the planner page and return its JSON value."
    )]
    async fn eval_js(&self, Parameters(args): Parameters<EvalArgs>) -> CallToolResult {
        match self.cdp.eval(&args.expression).await {
            Ok(value) => ok(value),
            Err(e) => fail(e),
        }
    }

    #[tool(
        description = "Dev builds only. Re-read adapter/adapter.js from disk and load it into the planner page."
    )]
    async fn reload_adapter(&self) -> CallToolResult {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../adapter/adapter.js");
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(e) => return fail(format!("cannot read {path}: {e}")),
        };
        match self.cdp.eval(&format!("{source}\n;window.__enkad.api.health()")).await {
            Ok(value) => ok(value),
            Err(e) => fail(e),
        }
    }

    #[tool(description = "Dev builds only. Left-click at CSS pixel coordinates in the planner view.")]
    async fn click(&self, Parameters(args): Parameters<ClickArgs>) -> CallToolResult {
        match self.cdp.click(args.x, args.y).await {
            Ok(()) => ok(json!({ "clicked": [args.x, args.y] })),
            Err(e) => fail(e),
        }
    }
}

#[tool_handler(
    router = self.router,
    name = "enkad-mcp",
    instructions = "Reads and edits the EnKA AIC planner (the Arknights: Endfield factory planner) shown in the EnkaD MCP window. Call `status` first. Read with `get_layout`, `get_graph` and `get_flows`; change with `apply_edits`, then read again to confirm the result. Every change is saved as a snapshot first and can be undone with `snapshot_restore`. All tools act on the plan that is open in the window. Entities are identified by `uid`, which is stable while a plan stays open."
)]
impl ServerHandler for Planner {}

/// Serve MCP over Streamable HTTP on localhost until the app exits.
pub async fn serve(planner: Planner, port: u16, token: String) -> std::io::Result<()> {
    // Hosts are limited to localhost by default. Rejecting every Origin header
    // keeps web pages out: browsers always send one, MCP clients never do.
    let config = StreamableHttpServerConfig::default().enforce_origin_validation();
    let service = StreamableHttpService::new(
        move || Ok(planner.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let token: Arc<str> = token.into();
    let app = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(token, require_token));

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, app).await
}

async fn require_token(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match presented {
        Some(presented) if same_bytes(presented.as_bytes(), token.as_bytes()) => {
            next.run(request).await
        }
        _ => (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response(),
    }
}

/// Comparison that does not stop at the first differing byte.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}
