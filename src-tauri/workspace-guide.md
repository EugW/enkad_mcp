# EnkaD MCP workspace

The window this terminal belongs to shows the EnKA AIC planner (the Arknights:
Endfield factory planner). It serves MCP tools under the server name `enkad`.
In this folder `.mcp.json` points Claude Code at it and `opencode.json` points
opencode at it. For Codex, run this once from this terminal:

    codex mcp add enkad --url $env:ENKAD_MCP_URL --bearer-token-env-var ENKAD_MCP_TOKEN

The address and token are in this terminal's environment as `ENKAD_MCP_URL` and
`ENKAD_MCP_TOKEN`.

## Working with the planner

- Call `status` first. Every tool acts on the plan that is open in the window.
- Read with `get_layout`, `get_ports`, `get_graph`, `get_flows`, `get_history`.
  `get_catalog` lists what can be built and each building's recipes.
- Change with `apply_edits`. A batch is all or nothing and is one undo step; the
  previous state is saved as a snapshot first (`snapshot_list`, `snapshot_restore`).
- After a change, read `get_graph` and `get_flows` again to confirm it did what
  you meant. The simulation needs a moment to re-solve (`solving`).
- Positions are planner cells: x grows right, y grows down, and a building's
  position is the top-left cell of its footprint. `status` gives the buildable area.
- A belt or pipe route is `"x,y>x,y>...>x,y"`: the cell it takes from, its own
  first cell, corners and last cell, then the cell it delivers into. `get_ports`
  gives the cell and the outside cell for every port.

## Planner behaviour worth knowing

- Depot unloaders and conduit outlets deliver nothing unless the depot has stock
  or is unlimited (`set_depot`).
- A splitter divides its input evenly among the outputs that accept it.
- A control port passes at most its rate, in steps of 0.5 per minute.
- Pipes cannot run over ground buildings. Where a new belt or pipe crosses an
  existing one, the planner splits both and puts a bridge at the crossing.
- A converger input fed directly by a splitter gets low priority and can be
  starved by the other inputs; `get_flows` lists such ports under `portMarks`.
  One cell of belt or pipe between the two avoids it.
- Rates are per minute at the simulation's last tick. Use `get_history` to see
  start-up behaviour and whether a rate is sustained.
