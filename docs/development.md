# Development

This page is for changing the app. [README.md](../README.md) covers using it and [tools.md](tools.md) describes the tools.

## How it fits together

```
agent --HTTP--> MCP server (mcp.rs) --> DevTools bridge (cdp.rs) --> planner page
                                                                       window.__enkad   adapter/adapter.js
                                                                       window.__h       the planner's own handle
```

The app is a Tauri 2 window with two child webviews. One shows the planner, which is a remote page. The other shows the terminal, which is a local page.

A tool call travels like this:

1. The agent sends the call to the MCP server over Streamable HTTP.
2. The tool's handler in `mcp.rs` turns the arguments into JSON and drops fields that were left out.
3. `cdp.rs` evaluates `window.__enkad.call("<name>", <arguments>)` in the planner page. It calls the DevTools protocol on the app's own webview, so there is no debugging port for another process to use.
4. The adapter does the work through `window.__h`, which the planner installs itself, and returns a JSON value.
5. An error thrown in the adapter becomes a tool error with the same message.

| File | What it holds |
| --- | --- |
| `src-tauri/src/main.rs` | The window, the menus, the two webviews, and the start of the MCP server. |
| `src-tauri/src/mcp.rs` | The tools, their argument types, the snapshots, and the token check. |
| `src-tauri/src/cdp.rs` | Evaluation, screenshots and mouse input in a webview. |
| `src-tauri/src/config.rs` | Settings, data folders, and the workspace the terminal starts in. |
| `src-tauri/src/panes.rs` | The split between the planner and the terminal. |
| `src-tauri/src/terminal.rs` | The shell behind the terminal pane. |
| `src-tauri/workspace-guide.md` | The guide for agents, written to the workspace as `CLAUDE.md` and `AGENTS.md`. |
| `adapter/adapter.js` | Everything that knows the planner's internals. |
| `ui/` | The terminal page, and the vendored xterm.js and Tauri API files it uses. |
| `tools/` | Scripts for calling and testing a running app. |

The agent is told how to work in three places: the server instructions at the end of `mcp.rs`, the tool and argument descriptions in `mcp.rs`, and `workspace-guide.md`. Keep them in step.

## Dev builds and release builds

```powershell
cd src-tauri
cargo run              # dev build
cargo build --release  # release build
```

| | Dev build | Release build |
| --- | --- | --- |
| Console window | Yes | No |
| Adapter | Read from `adapter/adapter.js` when the app starts | Compiled into the app |
| Dev tools | Yes | No |

A dev build adds these tools to the server. They give raw access to the page and the terminal, so they never ship.

| Tool | What it does |
| --- | --- |
| `eval_js` | Evaluates a JavaScript expression in the planner page and returns its JSON value. |
| `reload_adapter` | Reads `adapter/adapter.js` from disk again and loads it into the page. |
| `click` | Clicks at a position in the planner view with real mouse input. |
| `terminal_type` | Shows the terminal pane and types text into its shell. |
| `terminal_eval` | Evaluates an expression in the terminal page. `window.__term` is its xterm instance. |
| `terminal_drag_edge` | Drags the terminal pane's left edge with real mouse input. |

`workspace-guide.md` is compiled into both builds, so rebuild after changing it. The app writes it to the workspace only when `CLAUDE.md` or `AGENTS.md` is missing.

## Testing without touching your plans

Set `ENKAD_MCP_DATA_DIR` to run a second copy of the app with its own folder. That copy has its own planner profile, snapshots, port and token, and it runs beside the real app.

Give it a port of its own first. Without a `config.json` it would take the default port, which the real app may be using:

```powershell
$env:ENKAD_MCP_DATA_DIR = "$env:TEMP\enkad-test"
New-Item -ItemType Directory -Force $env:ENKAD_MCP_DATA_DIR | Out-Null
'{ "port": 47833, "token": "test-token", "channel": "beta" }' | Set-Content "$env:ENKAD_MCP_DATA_DIR\config.json"
cd src-tauri
cargo run
```

The scripts in `tools/` read the same variable, so set it in the shell you run them from as well. They need Python 3 and nothing else.

| Command | What it does |
| --- | --- |
| `python tools\mcp_call.py` | Lists the server's tools. |
| `python tools\mcp_call.py status` | Calls a tool that takes no arguments. |
| `python tools\mcp_call.py get_catalog '{"type": "Moulding Unit"}'` | Calls a tool with arguments. |
| `python tools\mcp_call.py eval_js --js probe.js` | Evaluates the expression in a file. |
| `python tools\mcp_call.py screenshot '{}' --save shot.jpg` | Saves a screenshot. |
| `python tools\regress_read.py` | Calls every read tool on the open plan. It changes nothing. |
| `python tools\regress_write.py` | Builds, edits, pairs, restores and checks the results. |

`mcp_call.py` prints at most 20,000 characters of a result.

`regress_write.py` edits the plan. It refuses to run unless `ENKAD_MCP_DATA_DIR` is set, and it needs an empty plan, which a new data folder starts with. Run it soon after the app starts: one check counts undo steps, and the planner keeps only 20.

## Changing the adapter

1. Edit `adapter/adapter.js`.
2. Run `python tools\mcp_call.py reload_adapter`. A dev build loads the new file into the page without a restart.
3. Explore the planner with `eval_js`. `window.__h.area` is the open plan and `window.__h.production` is its simulation.
4. Raise `VERSION` at the top of the file when the behaviour of the adapter changes. `status` reports it as `adapter.adapter`.

To add an edit to `apply_edits`:

1. Add a function to `OPS` in `adapter/adapter.js`. Throw an `Error` with a plain message to fail the whole batch.
2. Add a variant to `EditOp` in `mcp.rs`. Its doc comments become the descriptions the agent reads.
3. Add a check to `tools/regress_write.py`, and describe the edit in `tools.md` and, if agents need to know about it, in `workspace-guide.md`.

To add a tool, add a function to `api` in the adapter, an argument type in `mcp.rs`, and a `#[tool]` method that calls `self.adapter("<name>", args)`.

## What the adapter relies on in the planner

The planner's internals are not a public interface. These are the parts the adapter uses and what is worth knowing about them.

**The handle.** The planner installs `window.__h` shortly after the page has mounted. `status` returns `adapter.ok: false` until it is there.

**Edits.** Edits call the planner's own operations, the ones behind its mouse and keyboard handling, so its collision and connection rules apply. New entities are placed the way a paste is.

**Undo history.**

- `area.undoStack.past` holds one blueprint string per state. The last entry is the current state.
- The planner keeps at most 20 entries and drops the oldest. Never use the length of the list to tell whether something was recorded.
- About 100 ms after a change the planner serialises the plan again and records the result, unless `area.serializeGuard` is set. `nextSerialise()` in the adapter waits for that moment.

**Conduit pairs.** A pair is two conduits with the same non-zero `state.channel`. The adapter uses the planner's own `connectConduits`, `disconnectConduit`, `conduitsCompatible` and `conduitPartner`. The simulation shares a pair's flow among all its ports, so `get_graph` reports a pair as one link.

**Power.** Every building has a `powered` flag. The planner sets the flags when `area.power` is read, so the adapter reads that first. The simulation ignores the flag.

**Blueprints.** The adapter writes entities in the planner's blueprint format, version 4.

When the planner changes, a tool usually fails with a message that names what is missing, or that says the planner build needs an adapter update. Start from that message, look at the planner with `eval_js`, and fix the adapter.

## Build problems

**`cargo build` cannot replace `enkad-mcp.exe`.** The file is in use. Close the app. If the error stays, look at what else is running: Arknights: Endfield once kept the file of a copy that was running when the game started open, even after that copy had been closed.

## Vendored files

`ui/vendor/` holds unmodified copies of `@xterm/xterm` 6.0.0, `@xterm/addon-fit` 0.11.0 and `@tauri-apps/api` 2.12.3. `ui/vendor/README.md` lists them.
