#!/usr/bin/env python3
"""A small, deterministic language server for YACE's integration tests.

Speaks JSON-RPC over stdio. Every received message is appended (one JSON
object per line) to $FAKE_LSP_LOG so tests can assert what YACE sent.
Positions are UTF-16 code units; the tests only use ASCII.

Behaviour:
- diagnostics: a warning on every "TODO", an error on every "ERROR"
- completion: alpha (textEdit), beta (snippet), gamma (auto-import via resolve)
- hover, signatureHelp, definition ($FAKE_LSP_DEF = "path:line:col"),
  references and rename (open docs + files under the root), document and
  workspace symbols ("def name" / "class Name"), a quick fix for TODO,
  formatting (strips trailing whitespace)
- a document containing "CRASH" makes the server exit with status 3
"""
import json, os, re, sys

LOG = os.environ.get("FAKE_LSP_LOG")
DEF = os.environ.get("FAKE_LSP_DEF")
# FAKE_LSP_PULL=1: pull diagnostics only (textDocument/diagnostic), like TypeScript 7.
PULL = os.environ.get("FAKE_LSP_PULL") == "1"
docs = {}
root = None


def log(msg):
    if LOG:
        with open(LOG, "a") as f:
            f.write(json.dumps(msg) + "\n")


def send(msg):
    msg["jsonrpc"] = "2.0"
    body = json.dumps(msg).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            if length is not None:
                break
            continue
        k, _, v = line.decode().partition(":")
        if k.lower() == "content-length":
            length = int(v)
    return json.loads(sys.stdin.buffer.read(length))


def uri_path(uri):
    from urllib.parse import unquote
    return unquote(uri[len("file://"):])


def path_uri(p):
    from urllib.parse import quote
    return "file://" + quote(p)


def offset(text, pos):
    lines = text.split("\n")
    line = min(pos["line"], len(lines) - 1)
    return sum(len(l) + 1 for l in lines[:line]) + min(pos["character"], len(lines[line]))


def position(text, off):
    before = text[:off]
    return {"line": before.count("\n"), "character": off - (before.rfind("\n") + 1)}


def rng(text, a, b):
    return {"start": position(text, a), "end": position(text, b)}


def word_at(text, off):
    s = off
    while s > 0 and (text[s - 1].isalnum() or text[s - 1] == "_"):
        s -= 1
    e = off
    while e < len(text) and (text[e].isalnum() or text[e] == "_"):
        e += 1
    return s, e, text[s:e]


def diagnostics_for(uri):
    text = docs.get(uri, "")
    diags = []
    for m in re.finditer(r"TODO|ERROR", text):
        sev = 2 if m.group() == "TODO" else 1
        diags.append({"range": rng(text, m.start(), m.end()), "severity": sev, "source": "fake",
                      "code": "T1" if sev == 2 else 7, "message": f"{m.group()} found"})
    return diags


def publish(uri):
    if not PULL:
        send({"method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": diagnostics_for(uri)}})


def files_under_root():
    out = []
    if root:
        for d, _, fs in os.walk(root):
            for f in fs:
                if f.endswith((".rs", ".ts", ".py", ".txt", ".fake")):
                    out.append(os.path.join(d, f))
    return out


def text_of(uri):
    if uri in docs:
        return docs[uri]
    try:
        with open(uri_path(uri)) as f:
            return f.read()
    except OSError:
        return ""


def occurrences(word):
    uris = set(docs) | {path_uri(p) for p in files_under_root()}
    out = []
    for u in sorted(uris):
        t = text_of(u)
        for m in re.finditer(r"\b%s\b" % re.escape(word), t):
            out.append((u, t, m.start(), m.end()))
    return out


def symbols(text):
    out = []
    for m in re.finditer(r"^(\s*)(def|class) (\w+)", text, re.M):
        a, b = m.start(3), m.end(3)
        out.append({"name": m.group(3), "kind": 12 if m.group(2) == "def" else 5,
                    "range": rng(text, m.start(), m.end()), "selectionRange": rng(text, a, b)})
    return out


def handle(msg):
    global root
    method, mid, p = msg.get("method"), msg.get("id"), msg.get("params") or {}
    if method == "initialize":
        folders = p.get("workspaceFolders") or []
        root = uri_path(folders[0]["uri"]) if folders else (uri_path(p["rootUri"]) if p.get("rootUri") else None)
        send({"id": mid, "result": {"capabilities": {
            "textDocumentSync": {"openClose": True, "change": 2, "save": {"includeText": False}},
            "completionProvider": {"triggerCharacters": ["."], "resolveProvider": True},
            "hoverProvider": True,
            "signatureHelpProvider": {"triggerCharacters": ["(", ","]},
            "definitionProvider": True, "referencesProvider": True, "renameProvider": True,
            "documentSymbolProvider": True, "workspaceSymbolProvider": True,
            "codeActionProvider": True, "documentFormattingProvider": True,
            "positionEncoding": "utf-16",
            **({"diagnosticProvider": {"identifier": "fake", "interFileDependencies": True, "workspaceDiagnostics": False}} if PULL else {})},
            "serverInfo": {"name": "fake-lsp"}}})
    elif method == "initialized":
        send({"id": "cfg-1", "method": "workspace/configuration", "params": {"items": [{"section": "fake"}]}})
        send({"method": "$/progress", "params": {"token": "t", "value": {"kind": "begin", "title": "indexing", "percentage": 0}}})
        send({"method": "$/progress", "params": {"token": "t", "value": {"kind": "end"}}})
    elif method == "textDocument/didOpen":
        td = p["textDocument"]
        docs[td["uri"]] = td["text"]
        publish(td["uri"])
    elif method == "textDocument/didChange":
        uri = p["textDocument"]["uri"]
        text = docs.get(uri, "")
        for c in p["contentChanges"]:
            if "range" in c:
                a, b = offset(text, c["range"]["start"]), offset(text, c["range"]["end"])
                text = text[:a] + c["text"] + text[b:]
            else:
                text = c["text"]
        docs[uri] = text
        if "CRASH" in text:
            os._exit(3)
        publish(uri)
    elif method == "textDocument/didClose":
        docs.pop(p["textDocument"]["uri"], None)
    elif method == "textDocument/completion":
        uri = p["textDocument"]["uri"]
        text = docs.get(uri, "")
        off = offset(text, p["position"])
        s, _, _ = word_at(text, off)
        r = rng(text, s, off)
        send({"id": mid, "result": {"isIncomplete": False, "items": [
            {"label": "alpha", "kind": 3, "detail": "fn alpha()", "sortText": "1",
             "textEdit": {"range": r, "newText": "alpha"}},
            {"label": "beta", "kind": 6, "sortText": "2", "insertText": "beta(${1:x})$0", "insertTextFormat": 2},
            {"label": "gamma", "kind": 7, "sortText": "3", "data": {"id": 1}},
        ] + [{"label": f"item{i}_{chr(97 + i % 26)}{chr(97 + (i * 7) % 26)}value", "kind": 6, "sortText": f"9{i:06d}",
               "detail": "number"} for i in range(int(os.environ.get("FAKE_LSP_ITEMS", "0")))]}})
    elif method == "completionItem/resolve":
        item = dict(p)
        if item.get("label") == "gamma":
            item["additionalTextEdits"] = [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                                            "newText": "import gamma;\n"}]
            item["documentation"] = "Gamma docs"
        send({"id": mid, "result": item})
    elif method == "textDocument/hover":
        text = docs.get(p["textDocument"]["uri"], "")
        _, _, w = word_at(text, offset(text, p["position"]))
        # Like TypeScript 7: a code block, then the text right after it.
        send({"id": mid, "result": {"contents": {"kind": "markdown",
              "value": f"```ts\nlet {w}:\tnumber\n```\n**hover** for `{w}`\r\n- item"}}})
    elif method == "textDocument/signatureHelp":
        text = docs.get(p["textDocument"]["uri"], "")
        off = offset(text, p["position"])
        line = text[:off].split("\n")[-1]
        active = line[line.rfind("(") + 1:].count(",") if "(" in line else 0
        send({"id": mid, "result": {"signatures": [{"label": "f(a: int, b: str)",
              "parameters": [{"label": "a: int"}, {"label": "b: str"}]}], "activeSignature": 0, "activeParameter": active}})
    elif method == "textDocument/definition":
        if DEF:
            path, line, col = DEF.rsplit(":", 2)
            pos = {"line": int(line), "character": int(col)}
            send({"id": mid, "result": {"uri": path_uri(path), "range": {"start": pos, "end": pos}}})
        else:
            send({"id": mid, "result": None})
    elif method == "textDocument/references":
        text = docs.get(p["textDocument"]["uri"], "")
        _, _, w = word_at(text, offset(text, p["position"]))
        send({"id": mid, "result": [{"uri": u, "range": rng(t, a, b)} for u, t, a, b in occurrences(w)]})
    elif method == "textDocument/rename":
        text = docs.get(p["textDocument"]["uri"], "")
        _, _, w = word_at(text, offset(text, p["position"]))
        changes = {}
        for u, t, a, b in occurrences(w):
            changes.setdefault(u, []).append({"range": rng(t, a, b), "newText": p["newName"]})
        send({"id": mid, "result": {"changes": changes}})
    elif method == "textDocument/documentSymbol":
        send({"id": mid, "result": symbols(docs.get(p["textDocument"]["uri"], ""))})
    elif method == "workspace/symbol":
        q = p.get("query", "").lower()
        out = []
        for f in files_under_root():
            u = path_uri(f)
            for s in symbols(text_of(u)):
                if q in s["name"].lower():
                    out.append({"name": s["name"], "kind": s["kind"], "location": {"uri": u, "range": s["selectionRange"]}})
        send({"id": mid, "result": out})
    elif method == "textDocument/codeAction":
        uri = p["textDocument"]["uri"]
        acts = []
        for d in p.get("context", {}).get("diagnostics", []):
            if d["message"] == "TODO found":
                acts.append({"title": "Replace TODO with DONE", "kind": "quickfix", "isPreferred": True,
                             "edit": {"changes": {uri: [{"range": d["range"], "newText": "DONE"}]}}})
        send({"id": mid, "result": acts})
    elif method == "textDocument/formatting":
        text = docs.get(p["textDocument"]["uri"], "")
        edits = []
        for i, line in enumerate(text.split("\n")):
            stripped = line.rstrip()
            if stripped != line:
                edits.append({"range": {"start": {"line": i, "character": len(stripped)}, "end": {"line": i, "character": len(line)}},
                              "newText": ""})
        send({"id": mid, "result": edits})
    elif method == "textDocument/diagnostic":
        send({"id": mid, "result": {"kind": "full", "items": diagnostics_for(p["textDocument"]["uri"])}})
    elif method == "shutdown":
        send({"id": mid, "result": None})
    elif method == "exit":
        sys.exit(0)
    elif mid is not None and method is not None:
        send({"id": mid, "error": {"code": -32601, "message": f"unhandled {method}"}})


while True:
    m = read()
    if m is None:
        break
    log(m)
    handle(m)
