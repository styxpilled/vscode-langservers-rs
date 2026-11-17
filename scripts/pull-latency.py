"""Time of a pull diagnostic and of a hover, each `delay` ms after a change of a JSON
document (`docs/experimental.md`, "Pull diagnostics").

python -I scripts/pull-latency.py <server.exe> <document.json> <delay ms> [rounds]
"""
import json, subprocess, sys, threading, time, queue

server, path, delay = sys.argv[1], sys.argv[2], int(sys.argv[3]) / 1000
rounds = int(sys.argv[4]) if len(sys.argv) > 4 else 6
text = open(path, encoding="utf-8").read()
uri = "file:///large.json"

p = subprocess.Popen([server, "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
responses = queue.Queue()


def reader():
    out = p.stdout
    while True:
        length = None
        while True:
            line = out.readline()
            if not line:
                return
            line = line.strip()
            if not line:
                break
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":")[1])
        msg = json.loads(out.read(length))
        if "id" in msg and "method" in msg:
            send({"jsonrpc": "2.0", "id": msg["id"], "result": None})
        elif "id" in msg:
            responses.put((time.perf_counter(), msg))


lock = threading.Lock()


def send(msg):
    body = json.dumps(msg).encode()
    with lock:
        p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        p.stdin.flush()


next_id = 0


def request(method, params):
    global next_id
    next_id += 1
    start = time.perf_counter()
    send({"jsonrpc": "2.0", "id": next_id, "method": method, "params": params})
    while True:
        t, msg = responses.get(timeout=120)
        if msg["id"] == next_id:
            return (t - start) * 1000, msg


def notify(method, params):
    send({"jsonrpc": "2.0", "method": method, "params": params})


threading.Thread(target=reader, daemon=True).start()
request("initialize", {"capabilities": {"textDocument": {"diagnostic": {}}}})
notify("initialized", {})
notify("workspace/didChangeConfiguration", {"settings": {"json": {"validate": {"enable": True}}}})
notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "json", "version": 1, "text": text}})
doc = {"textDocument": {"uri": uri}}
request("textDocument/diagnostic", doc)

version = 1
pulls, hovers = [], []
for i in range(rounds):
    for kind in ("pull", "hover"):
        version += 1
        notify("textDocument/didChange", {
            "textDocument": {"uri": uri, "version": version},
            "contentChanges": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "text": " "}],
        })
        time.sleep(delay)
        if kind == "pull":
            ms, _ = request("textDocument/diagnostic", doc)
            pulls.append(ms)
        else:
            ms, _ = request("textDocument/hover", {**doc, "position": {"line": 0, "character": 1}})
            hovers.append(ms)
            request("textDocument/diagnostic", doc)
        time.sleep(0.3)

request("shutdown", None)
notify("exit", None)
fmt = lambda v: " ".join("%.0f" % x for x in v)
print("pull ms:", fmt(pulls), "| hover ms:", fmt(hovers))
