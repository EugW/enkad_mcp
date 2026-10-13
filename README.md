# EnkaD MCP

EnkaD MCP is a Windows app that shows the [EnKA AIC planner](https://beta.enka.network/endfield/aic/), the factory planner for Arknights: Endfield, in its own window and runs an MCP server beside it. An agent such as Claude Code, Codex or opencode connects to that server and can then read the plan that is open in the window, read its simulation, and change the plan.

Everything runs on your machine. The planner page is the only thing loaded from the network.

## What an agent can do

| Tool | What it does |
| --- | --- |
| `status` | Says whether the planner can be reached and summarises the open plan, including how many buildings lack power. |
| `get_layout` | Lists the buildings, belts and pipes, with their positions and settings, and marks buildings without power. |
| `get_ports` | Lists where the ports of given buildings sit, for writing routes. |
| `get_graph` | Lists which building feeds which, through which belts or pipes, at what rate. |
| `get_flows` | Reports each building's simulated input and output, and what is stalled. |
| `get_history` | Reports what moved through given buildings over time. |
| `get_catalog` | Lists the building types that can be placed and their power use, a building's recipes with crafting times, or every recipe and building for an item. |
| `apply_edits` | Adds, deletes, moves, rotates and configures things, pairs conduits, sets the depot. |
| `export_blueprint`, `import_blueprint` | Reads the plan as a blueprint string, or replaces it with one. |
| `snapshot_create`, `snapshot_list`, `snapshot_restore` | Saves the plan to disk and brings a saved state back. |
| `screenshot` | Captures the planner view as an image. |

[docs/tools.md](docs/tools.md) describes every tool, every edit and the results they return.

## Requirements

- Windows 10 or 11 with the WebView2 runtime. Windows 11 includes it.
- A current stable Rust toolchain, to build the app.
- An internet connection, for the planner page.

## Build and run

```powershell
cd src-tauri
cargo build --release
.\target\release\enkad-mcp.exe
```

Node.js and the Tauri CLI are not needed. The local UI is a few plain files in `ui/`.

The window title shows the planner channel and the address of the MCP server, for example `EnkaD MCP - beta - MCP on 127.0.0.1:47800`.

## Connect an agent

The server speaks MCP over Streamable HTTP at `http://127.0.0.1:47800/mcp` and expects a bearer token. The app creates the token on first run. The server is part of the app, so the app has to be running for an agent to connect.

### From the built-in terminal

Choose **Terminal > Show terminal**. A shell opens beside the planner, in a workspace folder that the app has prepared:

- `.mcp.json` points Claude Code at the server. Run `claude`.
- `opencode.json` points opencode at the server. Run `opencode`.
- For Codex, register the server once, then run `codex`:

  ```powershell
  codex mcp add enkad --url $env:ENKAD_MCP_URL --bearer-token-env-var ENKAD_MCP_TOKEN
  ```

The shell has the address and token in `ENKAD_MCP_URL` and `ENKAD_MCP_TOKEN`. The folder also holds `CLAUDE.md` and `AGENTS.md`, a short guide that tells the agent how to work with the planner.

### From anywhere else

The **MCP** menu copies a ready-made command, with your address and token filled in, for each of the three agents. They have this form:

```powershell
claude mcp add --transport http enkad http://127.0.0.1:47800/mcp --header "Authorization: Bearer <token>"
codex mcp add enkad --url http://127.0.0.1:47800/mcp --bearer-token-env-var ENKAD_MCP_TOKEN
opencode mcp add enkad --url http://127.0.0.1:47800/mcp --header "Authorization=Bearer <token>"
```

Codex reads the token from the `ENKAD_MCP_TOKEN` environment variable. Only the app's own terminal sets it, so set it yourself when you run Codex elsewhere.

**MCP > Copy server config (JSON)** copies the entry for clients that read a JSON file:

```json
{
  "mcpServers": {
    "enkad": {
      "type": "http",
      "url": "http://127.0.0.1:47800/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

## How changes are kept safe

- Every tool acts on the plan that is open in the window.
- `apply_edits` applies a batch of edits completely or not at all. If one edit fails, the plan is left as it was.
- A batch is one step in the planner's own undo history.
- Before `apply_edits`, `import_blueprint` and `snapshot_restore` change anything, the app saves the plan as a snapshot on disk. `snapshot_restore` brings any snapshot back. The app keeps the 40 newest automatic snapshots and every snapshot made with `snapshot_create`.
- A read-only plan, such as a shared one, cannot be edited. Press **Edit** in the planner to work on a copy.

## Power is not simulated

The planner needs no power supply. Its simulation runs every building whether or not the building has power, so a plan can show good flows and still stand still in the game. The agent has to check power itself, and the server tells it to: `status` and `apply_edits` report the number of buildings that need power and have none as `unpowered`, and `get_layout` marks each of them. Some buildings need no power, conduits among them. [docs/tools.md](docs/tools.md#power) has the details.

## Menus

| Menu | Item | What it does |
| --- | --- | --- |
| Channel | Beta | The planner the app works with. Release is listed but has no adapter yet. |
| MCP | Copy Claude Code command, Copy Codex command, Copy opencode command | Copies the command that registers the server with that agent. |
| MCP | Copy server config (JSON) | Copies the server entry for a JSON config file. |
| MCP | Open snapshots folder | Opens the folder of saved plan states. |
| Terminal | Show terminal | Shows or hides the terminal pane. |
| Terminal | Open workspace folder | Opens the folder the terminal starts in. |
| Planner | Reload | Reloads the planner page. |

## The terminal pane

The pane runs PowerShell 7 when it is installed and Windows PowerShell otherwise. Drag its left edge to resize it. The app remembers whether it was showing and how wide it was.

Ctrl+Enter and Shift+Enter send what Alt+Enter sends, which Claude Code, opencode and Codex read as a new line instead of as "send". If the shell exits, press Enter to start it again.

## Files and settings

The app keeps its data in `%LOCALAPPDATA%\dev.enkad-mcp.planner`:

| Path | Contents |
| --- | --- |
| `config.json` | The MCP port (47800 by default), the token, the channel, and the state of the terminal pane. |
| `profile\` | The planner's own storage, including your saved plans. It is separate from any browser. |
| `snapshots\` | One JSON file per snapshot. The file name is the UTC time it was taken. |
| `workspace\` | The folder the terminal starts in. |

To change the port, edit `port` in `config.json` and start the app again. The app rewrites the `enkad` entry in the workspace's `.mcp.json` and `opencode.json` on every start and keeps anything else in those files. Agents registered with a copied command keep the old address until you register them again.

The app writes `CLAUDE.md` and `AGENTS.md` only when they are missing, so your edits to them stay. Delete them to get the current version of the guide on the next start.

Only one copy of the app runs at a time. Starting it again brings the first window forward.

## Security

- The server listens on `127.0.0.1` only.
- Every request needs the bearer token.
- Requests that carry an `Origin` header, or a `Host` other than localhost, are refused. Browsers always send an `Origin`, so a web page cannot call the server.
- The planner page is remote content. It cannot call the app's commands, and the app opens no debugging port.
- An agent in the terminal pane runs with your own user rights, like in any other terminal.

## Limits

- The app runs on Windows only, because it drives the planner through WebView2.
- Only the beta planner is supported. The release planner has no simulation and no handle for the app to use.
- The app reads the planner's internals, which are not a public interface. A planner update can break a tool. The fix then belongs in `adapter/adapter.js`.

## Troubleshooting

| What you see | What to do |
| --- | --- |
| The agent reports that the connection was refused. | Start the app. The server only runs while the app is open. |
| `missing or invalid bearer token` | Register the server again with a command from the **MCP** menu. |
| The window title says `MCP server not running`. | Another program has the port. Change `port` in `config.json` and restart. |
| `status` returns `adapter.ok: false`, or a tool says the planner has not finished loading. | Wait for the planner to load, or choose **Planner > Reload**. |
| `the open plan is read-only` | Press **Edit** in the planner to work on a copy. |
| A tool fails with a message that the planner build needs an adapter update. | The planner changed. See [docs/development.md](docs/development.md). |

## Development

[docs/development.md](docs/development.md) covers how the app is put together, the dev build and its extra tools, and how to test changes without touching your real plans.
