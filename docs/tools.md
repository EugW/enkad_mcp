# Tool reference

This page describes every tool the `enkad` MCP server offers, what each takes and what each returns. [README.md](../README.md) covers installing the app and connecting an agent.

- [Conventions](#conventions)
- [Reading the plan](#reading-the-plan): `status`, `get_layout`, `get_ports`, `get_graph`, `get_flows`, `get_history`, `get_catalog`
- [Changing the plan](#changing-the-plan): `apply_edits` and its edits
- [Power](#power)
- [Conduits](#conduits)
- [Blueprints and snapshots](#blueprints-and-snapshots): `export_blueprint`, `import_blueprint`, `snapshot_create`, `snapshot_list`, `snapshot_restore`
- [Screenshot](#screenshot)
- [Worked example](#worked-example)
- [Planner behaviour worth knowing](#planner-behaviour-worth-knowing)

## Conventions

**The open plan.** Every tool acts on the plan that is open in the app's window. Call `status` first: the other tools only work when it returns `adapter.ok: true`.

**Cells.** Positions are planner cells. `x` grows to the right and `y` grows downward. The position of a building is the top-left cell of its footprint. `status` returns the buildable rectangle as `plan.buildable`.

**Uids.** Every building, belt and pipe has a `uid`. A uid stays the same while a plan stays open, with two exceptions:

- `import_blueprint` and `snapshot_restore` may give entities new uids.
- Where a new belt or pipe crosses an old one, the planner splits both. The old uid is gone and the pieces have new ones.

**Routes.** A belt or pipe is written as a route, `"x,y>x,y>...>x,y"`:

1. The cell the path takes items from. This is a cell of the building that feeds it.
2. The first cell of the path itself, then each corner, then its last cell.
3. The cell the path delivers into. This is a cell of the building it feeds.

Each step runs along one axis. The first and last cells must touch the path. `get_ports` returns the two cells you need for every port: `cell` and `outsideCell`. A route that leaves an output starts `cell>outsideCell>...`, and a route that enters an input ends `...>outsideCell>cell`.

**Tables.** Results are JSON. Lists come as a table with `columns` and `rows`, which keeps a large plan short:

```json
{ "columns": ["uid", "kind", "cells", "route"],
  "rows": [[78, "pipe", 21, "10,51>9,51>9,48>23,48>23,51>22,51"]] }
```

**Rates.** Rates are items per minute, written as text such as `"Clean Water 120, Sewage 60"`. A `~` after a number means the rate has not settled. The simulation runs at 240 ticks per minute.

**Errors.** A failed call returns an error result with a plain message, for example `no entity with uid 12`.

## Reading the plan

None of these tools changes anything.

### `status`

Takes no arguments. Returns the state of the adapter and a summary of the open plan.

| Field | Meaning |
| --- | --- |
| `adapter.ok` | `true` when the app can reach the planner's internals. |
| `adapter.adapter` | The adapter's version number. |
| `app`, `channel` | The app version and the planner channel (`beta`). |
| `plan.name` | The name of the open plan. |
| `plan.buildable` | The rectangle `{x, y, w, h}` that can be built on. |
| `plan.contentBounds` | The rectangle around everything that is placed. |
| `plan.counts` | The number of buildings, belts and pipes. |
| `plan.unpowered` | The number of buildings that need power and have none. See [Power](#power). |
| `plan.readOnly` | `true` for a plan that cannot be edited. |
| `plan.undoDepth` | The number of states in the planner's undo history. The planner keeps at most 20. |
| `depot` | `unlimited`, and `stock` as text such as `"Clean Water 500 +30/m"`. |
| `sim.tick`, `sim.ticks` | The tick the simulation shows and the length of the simulated run. |
| `sim.solving` | `true` while the simulation is working out a change. |

### `get_layout`

Lists what is placed.

| Argument | Meaning |
| --- | --- |
| `region` | Optional `{x, y, w, h}`. Only entities whose footprint touches the rectangle are listed. |

Returns two tables:

- `buildings`: `uid`, `name`, `x`, `y`, `w`, `h`, `rotation`, `recipe`, `settings`.
- `paths`: `uid`, `kind` (`belt` or `pipe`), `cells` (its length), `route`.

`recipe` names what the building makes, for example `"Amethyst Bottle"`. `settings` is text with the parts that are not at their default:

| Part | Meaning |
| --- | --- |
| `item=Clean Water` | The item the building hands out. Buildings with several such outputs show `item[0]=...`. |
| `rate=6/m` | The rate limit of a control port. |
| `count=5` | The count setting of a control port. |
| `infinite` | The outlet's infinite setting is on. |
| `disabled` | The building is switched off. |
| `mode=...` | The building's mode, as the planner labels it. |
| `pair=76` | The uid of the conduit this conduit is paired with. |
| `unpowered` | The building needs power and has none. See [Power](#power). |

### `get_ports`

Lists where the ports of buildings sit.

| Argument | Meaning |
| --- | --- |
| `uids` | The buildings to report. |

Returns a table with `uid`, `name`, `side` (`in` or `out`), `port` (the port's number), `carries` (`belt` or `pipe`), `cell`, `outsideCell` and `faces` (`N`, `E`, `S` or `W`).

`cell` is the building cell the port is on. `outsideCell` is the cell just outside it, where a path begins or ends.

### `get_graph`

Lists the connections: which output feeds which input.

| Argument | Meaning |
| --- | --- |
| `uids` | Optional. Only connections into or out of these buildings are listed. |

Returns a table with `from`, `fromName`, `outPort`, `to`, `toName`, `inPort`, `via` and `perMinute`.

`via` holds the uids of the belts or pipes between the two buildings. It is empty when the buildings touch. It is `conduit` for a [paired inlet and outlet](#conduits). That row has no port numbers, because a pair shares its flow among all its ports.

### `get_flows`

Reports simulated throughput at the current tick.

| Argument | Meaning |
| --- | --- |
| `uids` | Optional. Only these buildings. |
| `region` | Optional `{x, y, w, h}`. Only buildings whose footprint touches the rectangle. |

Returns:

- `buildings`: `uid`, `name`, `inPerMinute`, `outPerMinute`, `stalled`. `stalled` is `idle`, `clogged` or empty. A building with no flow that is not stalled is left out.
- `portMarks`: ports the simulation has marked, for example a low-priority input that is being starved.
- `depot`: what the depot gains (`producedPerMinute`) and loses (`consumedPerMinute`) overall.
- `tick`, `ticks`, `solving`.

After a change the simulation needs a moment. Read again while `solving` is `true`.

### `get_history`

Reports how many items moved into and out of buildings over the simulated run, summed per window of time. Use it to see start-up behaviour and whether a rate is sustained.

| Argument | Meaning |
| --- | --- |
| `uids` | The buildings to report. |
| `from` | Optional first tick. Defaults to 0. |
| `to` | Optional last tick. Defaults to the end of the run. |
| `window` | Optional window length in ticks. Defaults to 1200, which is 5 minutes. |

Returns a table with `uid`, `windowStartTick`, `item`, `in` and `out`. The counts are items, not rates.

### `get_catalog`

Lists what can be placed, the recipes of a building, or the recipes for an item. The data is the game data the planner page carries, so nothing is fetched.

| Argument | Meaning |
| --- | --- |
| `type` | Optional building type, by name or id. Lists that building's recipes. |
| `item` | Optional item name, part of a name, or item id. Lists every recipe that makes or uses the item. |

Without `type` it returns a table with these columns:

| Column | Meaning |
| --- | --- |
| `id`, `name` | The type. Either one works wherever a tool takes a type. |
| `kind` | `building`, `belt` or `pipe`. |
| `layer` | `ground` or `pipe`. |
| `w`, `h` | The size of the footprint in cells. |
| `inputs`, `outputs` | The number of input and output ports. |
| `recipes` | The number of recipes the type has. |
| `powerUse` | The power the building needs. 0 means it needs none. |
| `powerArea` | For a pylon or relay, the rectangle of cells it powers, such as `12x12`, centred on it. Empty for everything else. |

With `type` it returns that type's recipes as a table with `recipeId`, `inputs`, `outputs` and `seconds`:

```json
{ "id": 24, "name": "Moulding Unit",
  "recipes": { "columns": ["recipeId", "inputs", "outputs", "seconds"],
               "rows": [[13, "2 Amethyst Fiber", "1 Amethyst Bottle", 2]] } }
```

`seconds` is the time one run of the recipe takes, so a building runs a recipe at most 60 / `seconds` times a minute. The recipe above makes at most 30 bottles a minute from 60 fibers. In a test the simulation gave exactly that once the unit was fed enough, and 15 bottles a minute when one belt brought it 30 fibers.

With `item` it searches every building's recipes by item name:

```json
{ "items": ["Origocrust"],
  "recipes": { "columns": ["building", "recipeId", "inputs", "outputs", "seconds", "role"],
    "rows": [
      ["Refining Unit", 93, "1 Originium Ore", "1 Origocrust", 2, "makes"],
      ["Refining Unit", 88, "1 Origocrust Powder", "1 Origocrust", 2, "makes"],
      ["Gearing Unit", 63, "5 Origocrust + 5 Amethyst Fiber", "1 Amethyst Component", 10, "uses"],
      ["Gearing Unit", 62, "10 Origocrust + 10 Ferrium", "1 Ferrium Component", 10, "uses"],
      ["Shredding Unit", 23, "1 Origocrust", "1 Origocrust Powder", 2, "uses"]
    ] },
  "other": { "columns": ["building", "item", "role"], "rows": [] } }
```

- **Matching.** The search ignores case. A name that matches an item exactly finds that item only. Otherwise it finds every item whose name contains the text, and `items` lists what it found. A search across all buildings that matches more than 12 items is refused, with the first names, so give more of the name.
- **`role`** is `makes`, `uses`, or both. Recipes that make the item come first.
- **`other`** lists buildings that deal with the item without a recipe. A `source` hands the item out once `configure` sets it as the building's item: mining rigs, pumps and gas extractors. A `consumes` building takes it in. `fuel` is burned for power.
- **With `type` as well**, only that building's recipes are searched, and any number of items may match.

For a raw item the sources are the answer. `get_catalog` with `{"item": "Amethyst Ore"}` returns the three mining rigs that can mine it under `other`, and the one recipe that uses it.

## Changing the plan

### `apply_edits`

Applies a list of edits in order, as one batch.

| Argument | Meaning |
| --- | --- |
| `ops` | The edits. Each is an object whose `op` names the kind of edit. |

What a batch guarantees:

- **All or nothing.** If any edit fails, none is applied. The error names the edit, for example `edit 2 (add_building): cannot place it there: blocked at 30,10 (Conduit Outlet uid 48). No edits were applied.`
- **One undo step.** The whole batch is one step in the planner's undo history.
- **A snapshot first.** The app saves the plan before it changes anything. The result names that snapshot in `previousStateSavedAs`.
- **The planner's own rules.** Edits go through the planner's own operations, so its collision and connection rules apply.

The tool fails on a read-only plan.

The result has `applied` (the number of edits), `results` (one entry per edit, as `{op, result}`), `counts`, `unpowered`, `undoDepth` and `previousStateSavedAs`.

After a batch, do two checks:

- Read `get_graph` and `get_flows` to confirm that the change did what you meant.
- Look at `unpowered`. It counts the buildings that need power and have none, and the simulation does not show them. See [Power](#power).

### The edits

#### `add_building`

Places a new building.

| Field | Meaning |
| --- | --- |
| `type` | The building type, by name or id, from `get_catalog`. |
| `x`, `y` | The top-left cell of its footprint. |
| `rotation` | Optional: 0, 90, 180 or 270. Defaults to 0. Copy the rotation of a similar building from `get_layout` to get the same port sides. |
| `recipe` | Optional recipe id, or the name of an item the recipe makes. |

The result lists what was added: `{"added": [{"uid": 74, "name": "Conduit Outlet", "x": 10, "y": 50, "w": 3, "h": 3, "rotation": 0}]}`. It carries a `warning` when the building lies outside the buildable area.

#### `add_path`

Lays a new belt or pipe.

| Field | Meaning |
| --- | --- |
| `kind` | Optional: `belt` or `pipe`. Defaults to `belt`. |
| `route` | The [route](#conventions). |

The result lists what was added. Where the new path crosses an existing one, the planner splits both and adds a bridge. The result then also has `replaced`, the uids that no longer exist.

#### `delete`

Removes buildings, belts or pipes.

| Field | Meaning |
| --- | --- |
| `uids` | What to remove. |

#### `move`

Shifts entities together.

| Field | Meaning |
| --- | --- |
| `uids` | What to move. |
| `dx` | Cells to the right. Negative moves left. |
| `dy` | Cells down. Negative moves up. |

The edit fails when the target cells are occupied or outside the plan.

#### `rotate`

Turns entities a quarter turn, as a group around their common centre.

| Field | Meaning |
| --- | --- |
| `uids` | What to rotate. |
| `clockwise` | Optional. Defaults to `true`. |

#### `set_recipe`

Chooses what a building makes.

| Field | Meaning |
| --- | --- |
| `uid` | The building. |
| `recipe` | A recipe id, or the name of an item the recipe makes. Leave it out to clear the recipe. |

When an item name matches several recipes, the error lists their ids.

#### `configure`

Changes the settings of a building. Only the fields you give are changed.

| Field | Meaning |
| --- | --- |
| `uid` | The building. |
| `item` | The item the building hands out, by name or id. For depot unloaders, pumps, unpaired conduit outlets and control ports. |
| `port` | Which output `item` applies to, for buildings with several. Defaults to the first. |
| `rate` | Control ports only. The most items per minute let through, in steps of 0.5. 0 removes the limit. |
| `count` | Control ports only. The count setting shown next to the rate in the planner. |
| `infinite` | Conduit outlets only. The outlet's infinite setting. |
| `disabled` | For buildings that can be switched off. |
| `mode` | For buildings that have modes. The label as the planner shows it. |

The result shows the building's new `settings` text.

#### `pair_conduits`

Pairs a conduit inlet with a conduit outlet. See [Conduits](#conduits).

| Field | Meaning |
| --- | --- |
| `inlet` | The uid of a Conduit Inlet or Conduit Inlet Manifold. |
| `outlet` | The uid of a Conduit Outlet or Conduit Outlet Manifold. |

The result is `{"inlet": 75, "outlet": 76}`. A conduit has one partner. If either conduit was already paired with another, that pairing ends, and the result lists the released conduits in `unpaired`.

The edit fails when the two are not one inlet and one outlet.

#### `unpair_conduit`

Ends a pairing.

| Field | Meaning |
| --- | --- |
| `uid` | The uid of either conduit of the pair. |

The result is `{"unpaired": [76, 75]}`. The list is empty when the conduit was not paired.

#### `set_depot`

Changes what the plan's depot can supply.

| Field | Meaning |
| --- | --- |
| `unlimited` | `true` makes every item available without limit. |
| `stock` | A list of `{item, count, rate}`. `count` is the units in the depot at the start of the simulation. `rate` is the units arriving per minute, in steps of 0.5. |

Depot unloaders and unpaired conduit outlets deliver nothing from an empty depot.

## Power

The planner needs no power supply. Its simulation runs every building whether or not the building has power, so `get_flows` and `get_graph` look the same either way. The game does need power, so a plan can simulate well and still stand still once built.

Always verify that every building that uses power is powered:

- `status` returns the number of buildings that need power and have none as `plan.unpowered`.
- `apply_edits` returns the same number as `unpowered` after each batch.
- `get_layout` marks each such building with `unpowered` in its `settings`.

The number must be 0 before a plan is finished.

A building is powered when at least one of its cells lies in the area of a pylon or relay. `get_catalog` gives the area as `powerArea`, centred on the pylon or relay:

| Building | Size | `powerArea` | Reach beyond its own edge |
| --- | --- | --- | --- |
| Electric Pylon, Xiranite Pylon | 2x2 | `12x12` | 5 cells on every side |
| Relay Tower, Xiranite Relay | 3x3 | `7x7` | 2 cells on every side |

An Electric Pylon placed at `x, y` therefore powers cells `x-5` to `x+6` and `y-5` to `y+6`.

Some buildings use no power and need no pylon. Conduits, depot loaders and unloaders, tanks, splitters, convergers, bridges and control ports are among them. `get_catalog` gives the power use of every type as `powerUse`.

## Conduits

Conduits move fluid between two places without a pipe between them. There are four buildings:

| Building | Pipe ports |
| --- | --- |
| Conduit Inlet | 1 input |
| Conduit Inlet Manifold | 2 inputs |
| Conduit Outlet | 1 output |
| Conduit Outlet Manifold | 2 outputs |

**Paired.** `pair_conduits` links an inlet to an outlet. Whatever flows into the inlet then comes out of the outlet. A pair is always one inlet and one outlet, and either may be a manifold.

**Unpaired.** An unpaired conduit trades with the depot instead. An inlet sends what it receives to the depot. An outlet hands out the item chosen with `configure`, from the depot. The planner calls this depot mode and notes that the game does not allow it, so pair the conduits of a plan you mean to build.

How a pair behaves:

- **The flow is shared.** A pair is one link, not one link per port. In a test, 120 per minute into one input of a manifold pair came out as 60 per minute on each of the two outputs.
- **A paired outlet has no item of its own.** `configure` with `item` fails on it. The item it had comes back when it is unpaired.
- **Deleting one conduit frees its partner.**
- **Moving or rotating a conduit keeps its pairing.** So do blueprints and snapshots.

Where the read tools show a pair:

- `get_layout` shows `pair=<uid>` in the `settings` of both conduits.
- `get_graph` shows one row from the inlet to the outlet with `via` set to `conduit`.
- `get_flows` lists both conduits with what passes through them.

## Blueprints and snapshots

### `export_blueprint`

Takes no arguments. Returns the open plan as a blueprint string, the format the planner's own paste and import accept, with the plan's name, size and counts.

### `import_blueprint`

Replaces the contents of the open plan.

| Argument | Meaning |
| --- | --- |
| `blueprint` | A blueprint string from `export_blueprint` or from the planner's own copy and share. |

The app saves the current state as a snapshot first. The change is one undo step. Entities may get new uids, so read the layout again afterwards. A string that cannot be read is refused and the plan is left unchanged. The tool fails on a read-only plan.

### `snapshot_create`

Saves the current state of the open plan to disk.

| Argument | Meaning |
| --- | --- |
| `label` | Optional short note on why this state is worth keeping. |

Returns the snapshot's `id`, which is the UTC time it was taken, such as `20261013-165700-129`.

### `snapshot_list`

Takes no arguments. Returns the saved snapshots, newest first, as a table with `id`, `label`, `plan`, `buildings`, `belts` and `pipes`.

The app makes a snapshot by itself before each `apply_edits`, `import_blueprint` and `snapshot_restore`, labelled with the reason, such as `before apply_edits`. It keeps the 40 newest of these. Snapshots made with `snapshot_create` are never removed.

### `snapshot_restore`

Puts the open plan back to a saved snapshot.

| Argument | Meaning |
| --- | --- |
| `id` | A snapshot id from `snapshot_create` or `snapshot_list`. |

The state being replaced is itself saved first, and its id is returned in `previousStateSavedAs`. The change is one undo step. Entities may get new uids. The tool fails on a read-only plan.

## Screenshot

### `screenshot`

Captures the planner view as a JPEG image.

| Argument | Meaning |
| --- | --- |
| `scale` | Optional downscale factor between 0.1 and 1. Defaults to 0.5. |

## Worked example

This builds a water source, sends the water through a conduit pair, delivers it to a Fluid Supply Unit, and powers that unit. The results shown are from a real run on an empty plan.

**1. Place the buildings.**

```json
{ "ops": [
  { "op": "set_depot", "unlimited": true },
  { "op": "add_building", "type": "Conduit Outlet", "x": 10, "y": 50 },
  { "op": "add_building", "type": "Conduit Inlet", "x": 20, "y": 50 },
  { "op": "add_building", "type": "Conduit Outlet", "x": 30, "y": 50 },
  { "op": "add_building", "type": "Fluid Supply Unit", "x": 40, "y": 50 }
] }
```

The result gives the new uids, 107 to 110, and `"unpowered": 1`: the supply unit needs power.

**2. Find the ports** with `get_ports` and `{"uids": [107, 108, 109, 110]}`:

```json
{ "columns": ["uid", "name", "side", "port", "carries", "cell", "outsideCell", "faces"],
  "rows": [
    [107, "Conduit Outlet", "out", 0, "pipe", "10,51", "9,51", "W"],
    [108, "Conduit Inlet", "in", 0, "pipe", "22,51", "23,51", "E"],
    [109, "Conduit Outlet", "out", 0, "pipe", "30,51", "29,51", "W"],
    [110, "Fluid Supply Unit", "in", 0, "pipe", "42,51", "43,51", "E"]
  ] }
```

**3. Wire and pair.** The first outlet hands out Clean Water from the depot. A pipe carries it around to the inlet, which is paired with the second outlet. A second pipe runs from that outlet to the supply unit. Each route starts `cell>outsideCell` and ends `outsideCell>cell`.

```json
{ "ops": [
  { "op": "configure", "uid": 107, "item": "Clean Water" },
  { "op": "add_path", "kind": "pipe", "route": "10,51>9,51>9,48>23,48>23,51>22,51" },
  { "op": "add_path", "kind": "pipe", "route": "30,51>29,51>29,54>43,54>43,51>42,51" },
  { "op": "pair_conduits", "inlet": 108, "outlet": 109 }
] }
```

**4. Check the flow** with `get_graph`. The water reaches the supply unit, and the pair shows as the `conduit` row:

```json
{ "columns": ["from", "fromName", "outPort", "to", "toName", "inPort", "via", "perMinute"],
  "rows": [
    [107, "Conduit Outlet", 0, 108, "Conduit Inlet", 0, "111", "Clean Water 120"],
    [109, "Conduit Outlet", 0, 110, "Fluid Supply Unit", 0, "112", "Clean Water 120"],
    [108, "Conduit Inlet", null, 109, "Conduit Outlet", null, "conduit", "Clean Water 120"]
  ] }
```

**5. Check the power.** The flow looks finished, but the last result still said `"unpowered": 1`, and `get_layout` marks the supply unit:

```json
[110, "Fluid Supply Unit", 40, 50, 3, 3, 0, "", "unpowered"]
```

The unit covers cells 40 to 42. An Electric Pylon at `x` 47 powers cells 42 to 53, which reaches it:

```json
{ "ops": [ { "op": "add_building", "type": "Electric Pylon", "x": 47, "y": 50 } ] }
```

The result now says `"unpowered": 0`. The three conduits were never counted, because they use no power.

## Planner behaviour worth knowing

- The simulation runs buildings that have no power. Check `unpowered` after every change.
- Depot unloaders and unpaired conduit outlets deliver nothing unless the depot has stock or is unlimited. An outlet's `infinite` setting does not change that.
- A splitter divides its input evenly among the outputs that accept it.
- A control port passes at most its rate, in steps of 0.5 per minute.
- Pipes cannot run over ground buildings.
- A converger input fed directly by a splitter gets low priority and can be starved by the other inputs. `get_flows` lists such ports under `portMarks`. One cell of belt or pipe between the two avoids it.
- Rates are per minute at the simulation's last tick. Use `get_history` to see start-up behaviour and whether a rate is sustained.
