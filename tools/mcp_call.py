"""Call the running app's MCP server from the command line (dev aid, stdlib only).

  python tools/mcp_call.py                     list tools
  python tools/mcp_call.py status              call a tool with no arguments
  python tools/mcp_call.py eval_js '{"expression": "1+1"}'
  python tools/mcp_call.py eval_js --js probe.js      expression read from a file
  python tools/mcp_call.py screenshot '{}' --save shot.jpg

Port and token come from the app's config.json.
"""
import base64
import json
import os
import sys
import urllib.error
import urllib.request

# The same variable that points the app at another data folder points this at it too.
DATA_DIR = os.environ.get("ENKAD_MCP_DATA_DIR") or os.path.join(os.environ["LOCALAPPDATA"], "dev.enkad-mcp.planner")
CONFIG = os.path.join(DATA_DIR, "config.json")


class Client:
    def __init__(self):
        with open(CONFIG, encoding="utf-8") as f:
            cfg = json.load(f)
        self.url = f"http://127.0.0.1:{cfg['port']}/mcp"
        self.headers = {
            "Authorization": "Bearer " + cfg["token"],
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        self.next_id = 0
        reply = self.request("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "mcp_call", "version": "0"},
        })
        self.server = reply["result"]
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, message):
        req = urllib.request.Request(self.url, json.dumps(message).encode(), self.headers)
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                session = resp.headers.get("Mcp-Session-Id")
                if session:
                    self.headers["Mcp-Session-Id"] = session
                body = resp.read().decode("utf-8")
                kind = resp.headers.get("Content-Type", "")
        except urllib.error.HTTPError as e:
            raise SystemExit(f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:500]}")
        if "text/event-stream" in kind:
            # The reply to a request is the last data line carrying our id.
            for line in reversed(body.splitlines()):
                if line.startswith("data:") and line[5:].strip():
                    return json.loads(line[5:])
            return None
        return json.loads(body) if body.strip() else None

    def request(self, method, params=None):
        self.next_id += 1
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method}
        if params is not None:
            message["params"] = params
        reply = self.send(message)
        if reply is None or "error" in reply:
            raise SystemExit(f"{method} failed: {reply}")
        return reply


def main():
    args = sys.argv[1:]
    save = None
    if "--save" in args:
        i = args.index("--save")
        save = args[i + 1]
        del args[i:i + 2]
    if "--js" in args:
        # Shorthand for eval_js with the expression read from a file.
        i = args.index("--js")
        with open(args[i + 1], encoding="utf-8") as f:
            args[i:i + 2] = [json.dumps({"expression": f.read()})]
    client = Client()
    if not args:
        print("server:", json.dumps(client.server.get("serverInfo")))
        for tool in client.request("tools/list")["result"]["tools"]:
            print(f"  {tool['name']}: {tool.get('description', '')[:90]}")
        return
    arguments = json.loads(args[1]) if len(args) > 1 else {}
    result = client.request("tools/call", {"name": args[0], "arguments": arguments})["result"]
    if result.get("isError"):
        print("TOOL ERROR")
    for block in result.get("content", []):
        if block["type"] == "text":
            try:
                print(json.dumps(json.loads(block["text"]), indent=1)[:20000])
            except ValueError:
                print(block["text"][:20000])
        elif block["type"] == "image":
            raw = base64.b64decode(block["data"])
            if save:
                with open(save, "wb") as f:
                    f.write(raw)
            print(f"image {block.get('mimeType')} {len(raw)} bytes" + (f" -> {save}" if save else ""))
        else:
            print(block)


if __name__ == "__main__":
    main()
