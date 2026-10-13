"""Read-only regression over the open plan: every read tool, full results, short summary."""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mcp_call  # noqa: E402

client = mcp_call.Client()


def call(name, arguments=None):
    result = client.request("tools/call", {"name": name, "arguments": arguments or {}})["result"]
    text = next(block["text"] for block in result["content"] if block["type"] == "text")
    if result.get("isError"):
        raise SystemExit(f"{name} failed: {text}")
    return json.loads(text), len(text)


status, _ = call("status")
print("adapter:", status["adapter"])
print("plan   :", status["plan"]["name"], status["plan"]["counts"], "| sim", status["sim"])

layout, n = call("get_layout")
buildings, paths = layout["buildings"]["rows"], layout["paths"]["rows"]
print(f"layout : {len(buildings)} buildings, {len(paths)} paths, {n} chars;"
      f" {sum(1 for b in buildings if b[7])} with a recipe, {sum(1 for b in buildings if b[8])} with settings")

graph, n = call("get_graph")
print(f"graph  : {len(graph['rows'])} connections, {sum(1 for r in graph['rows'] if r[7])} carrying flow, {n} chars")

flows, n = call("get_flows")
print(f"flows  : {len(flows['buildings']['rows'])} buildings, {len(flows['portMarks']['rows'])} marked ports, {n} chars")
print("         depot gains:", flows["depot"]["producedPerMinute"])
print("         depot loses:", flows["depot"]["consumedPerMinute"])

busy = [b[0] for b in flows["buildings"]["rows"] if b[2] and b[3]][:2]
history, n = call("get_history", {"uids": busy, "window": 3600})
print(f"history: {len(history['rows'])} rows for uids {busy}; first {history['rows'][:2]}")

ports, n = call("get_ports", {"uids": busy})
print(f"ports  : {len(ports['rows'])} ports for uids {busy}; first {ports['rows'][0]}")

catalog, n = call("get_catalog")
print(f"catalog: {len(catalog['rows'])} types")
