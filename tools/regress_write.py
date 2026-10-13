"""Write-path regression. Run ONLY against the isolated test instance (ENKAD_MCP_DATA_DIR set)."""
import json
import os
import sys
import time

if not os.environ.get("ENKAD_MCP_DATA_DIR"):
    raise SystemExit("refusing to run: ENKAD_MCP_DATA_DIR is not set, this would edit the real plan")

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mcp_call  # noqa: E402

client = mcp_call.Client()
failures = []


def call(name, arguments=None, expect_error=False):
    result = client.request("tools/call", {"name": name, "arguments": arguments or {}})["result"]
    text = next(block["text"] for block in result["content"] if block["type"] == "text")
    if bool(result.get("isError")) != expect_error:
        raise SystemExit(f"{name} {'succeeded' if expect_error else 'failed'} unexpectedly: {text[:300]}")
    return text if expect_error else json.loads(text)


def check(label, ok, detail=""):
    print(("  ok   " if ok else "  FAIL ") + label + (f"  [{detail}]" if detail else ""))
    if not ok:
        failures.append(label)


def settle():
    for _ in range(40):
        time.sleep(0.5)
        if not call("get_flows", {"uids": [0]})["solving"]:
            return


def flows():
    data = call("get_flows")
    return {row[0]: (row[2], row[3]) for row in data["buildings"]["rows"]}, data


status = call("status")
plan = status["plan"]
print("instance:", status["adapter"], "| plan", plan["name"], plan["counts"], "readOnly", plan["readOnly"])
check("adapter reaches the planner through window.__h", status["adapter"].get("harness") == "window.__h" and status["adapter"]["ok"])
if any(plan["counts"].values()):
    raise SystemExit("the test plan is not empty; refusing to continue")
depth0 = plan["undoDepth"]

print("1. build a pipe network in one batch")
built = call("apply_edits", {"ops": [
    {"op": "set_depot", "unlimited": True},
    {"op": "add_building", "type": "Conduit Outlet", "x": 30, "y": 10},
    {"op": "add_building", "type": "Pipe Control Port", "x": 26, "y": 11, "rotation": 270},
    {"op": "add_building", "type": "Pipe Splitter", "x": 22, "y": 11, "rotation": 270},
    {"op": "add_building", "type": "Fluid Supply Unit", "x": 15, "y": 10},
    {"op": "add_building", "type": "Fluid Supply Unit", "x": 21, "y": 5},
    {"op": "add_building", "type": "Fluid Supply Unit", "x": 21, "y": 15},
]})
uid = {r["result"]["added"][0]["name"] + str(i): r["result"]["added"][0]["uid"] for i, r in enumerate(built["results"]) if r["op"] == "add_building"}
outlet, port, splitter = uid["Conduit Outlet1"], uid["Pipe Control Port2"], uid["Pipe Splitter3"]
sinks = [uid["Fluid Supply Unit4"], uid["Fluid Supply Unit5"], uid["Fluid Supply Unit6"]]
check("six buildings placed", built["counts"]["building"] == 6, str(built["counts"]))

ports = call("get_ports", {"uids": [outlet, port, splitter] + sinks})["rows"]
find = lambda u, side: [r for r in ports if r[0] == u and r[2] == side]
routes = []
routes.append(f"{find(outlet, 'out')[0][5]}>{find(outlet, 'out')[0][6]}>27,11>{find(port, 'in')[0][5]}")
routes.append(f"{find(port, 'out')[0][5]}>{find(port, 'out')[0][6]}>23,11>{find(splitter, 'in')[0][5]}")
routes += ["22,11>21,11>18,11>17,11", "22,11>22,10>22,9>24,9>24,6>23,6", "22,11>22,12>22,13>24,13>24,16>23,16"]
wired = call("apply_edits", {"ops": [
    {"op": "configure", "uid": outlet, "item": "Clean Water", "infinite": True},
    {"op": "configure", "uid": port, "item": "Clean Water", "rate": 6},
] + [{"op": "add_path", "kind": "pipe", "route": r} for r in routes]})
check("five pipes laid", wired["counts"]["pipe"] == 5, str(wired["counts"]))
check("two batches are two undo steps", wired["undoDepth"] == depth0 + 2, f"{depth0} -> {wired['undoDepth']}")

settle()
by_uid, data = flows()
check("depot supplies 6/min through the control port", data["depot"]["consumedPerMinute"].startswith("Clean Water 6"), data["depot"]["consumedPerMinute"])
check("splitter gives each sink 2/min", all(by_uid.get(s, ("", ""))[0] == "Clean Water 2" for s in sinks), str([by_uid.get(s) for s in sinks]))
graph = call("get_graph")["rows"]
check("graph has the five connections", len(graph) == 5 and all(r[7] for r in graph), f"{len(graph)} rows")
layout = call("get_layout")
check("layout shows the settings", any("rate=6/m" in b[8] for b in layout["buildings"]["rows"]))

print("2. change a setting")
call("apply_edits", {"ops": [{"op": "configure", "uid": port, "rate": 3}]})
settle()
by_uid, _ = flows()
check("halving the limit gives each sink 1/min", all(by_uid.get(s, ("", ""))[0] == "Clean Water 1" for s in sinks), str([by_uid.get(s) for s in sinks]))

print("3. a failing batch changes nothing")
before = call("status")["plan"]
error = call("apply_edits", {"ops": [
    {"op": "delete", "uids": [sinks[0]]},
    {"op": "add_building", "type": "Fluid Tank", "x": 30, "y": 10},
]}, expect_error=True)
after = call("status")["plan"]
check("the error names the blocking building", "Conduit Outlet" in error, error[:140])
check("counts and undo depth are unchanged", (before["counts"], before["undoDepth"]) == (after["counts"], after["undoDepth"]), f"{before['undoDepth']} -> {after['undoDepth']}")

print("4. move, rotate, delete, re-lay")
moved = call("apply_edits", {"ops": [{"op": "move", "uids": [sinks[2]], "dx": 0, "dy": 3}]})
check("a building moves", moved["results"][0]["result"][0]["y"] == 18)
call("apply_edits", {"ops": [{"op": "move", "uids": [sinks[2]], "dx": 0, "dy": -3}]})
blocked = call("apply_edits", {"ops": [{"op": "move", "uids": [sinks[2]], "dx": 0, "dy": -9}]}, expect_error=True)
check("a blocked move is refused", "cannot move" in blocked, blocked[:100])
pipe = next(p for p in call("get_layout")["paths"]["rows"] if p[3] == routes[4])
relaid = call("apply_edits", {"ops": [{"op": "delete", "uids": [pipe[0]]}, {"op": "add_path", "kind": "pipe", "route": routes[4]}]})
settle()
by_uid, _ = flows()
check("a pipe deleted and re-laid carries the same flow", by_uid.get(sinks[2], ("", ""))[0] == "Clean Water 1", str(by_uid.get(sinks[2])))

print("5. crossing pipe, snapshot and restore")
snap = call("snapshot_create", {"label": "regression"})
crossed = call("apply_edits", {"ops": [{"op": "add_path", "kind": "pipe", "route": "28,13>28,12>28,10>28,9"}]})
result = crossed["results"][0]["result"]
check("the planner's bridge is reported", any(a.get("name") == "Pipe Bridge" for a in result["added"]) and result.get("replaced"), json.dumps(result)[:160])
restored = call("snapshot_restore", {"id": snap["id"]})
check("restore brings the plan back", restored["plan"]["counts"] == snap["counts"], str(restored["plan"]["counts"]))

print("6. blueprint round trip")
exported = call("export_blueprint")
imported = call("import_blueprint", {"blueprint": exported["blueprint"]})
check("export then import keeps the plan", imported["plan"]["counts"] == exported["counts"])
settle()
_, data = flows()
rate = float(data["depot"]["consumedPerMinute"].replace("Clean Water ", "").rstrip("~") or 0)
check("and it still flows at about 3/min", abs(rate - 3) < 0.15, data["depot"]["consumedPerMinute"])
bad = call("import_blueprint", {"blueprint": "not-a-blueprint"}, expect_error=True)
check("a bad blueprint is refused", "not a readable blueprint" in bad)

print("7. recipes and catalog")
catalog = call("get_catalog")
check("catalog lists the building types", len(catalog["rows"]) >= 70, f"{len(catalog['rows'])} types")
recipes = call("get_catalog", {"type": "Moulding Unit"})["recipes"]["rows"]
made = call("apply_edits", {"ops": [{"op": "add_building", "type": "Moulding Unit", "x": 40, "y": 30, "recipe": "Amethyst Bottle"}]})
check("a recipe can be set by item name", any("Amethyst Bottle" in (b[7] or "") for b in call("get_layout")["buildings"]["rows"]), f"{len(recipes)} recipes for the type")

print()
print("FAILED: " + ", ".join(failures) if failures else "all checks passed")
sys.exit(1 if failures else 0)
