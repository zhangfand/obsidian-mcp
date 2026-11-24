#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml==6.0.2"]
# ///
"""A stand-in obsidian-mcp server, written from server/TOOLS.md, used only to
prove the verifier. `--fault N` seeds one fault that must fail rule N alone.

standin.py --vault <dir> --listen 127.0.0.1:<port> [--fault N]
"""

import argparse
import difflib
import hashlib
import json
import os
import posixpath
import re
import shutil
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import quote

import yaml

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "lib"))
import model as M  # noqa: E402

LIMIT = 8192
FAULT = 0
VAULT = ""


class ToolError(Exception):
    def __init__(self, kind, message, **extra):
        super().__init__(message)
        self.obj = {"error": kind, "message": message, **extra}


class _Loader(yaml.SafeLoader):
    pass


_Loader.yaml_implicit_resolvers = {
    k: [(tag, rx) for tag, rx in v if tag != "tag:yaml.org,2002:timestamp"]
    for k, v in yaml.SafeLoader.yaml_implicit_resolvers.items()
}


# ---------- vault access ----------

def notes():
    return [f for f in M.list_files(VAULT) if f.lower().endswith(".md")]


def raw(path):
    with open(os.path.join(VAULT, path), "rb") as f:
        return f.read()


def version(data):
    return hashlib.sha256(data).hexdigest()[:16]


def properties(data):
    fm = M.frontmatter_span(data)
    if not fm:
        return {}
    lines = M.split_lines(data)
    block = data[lines[fm[0]][0]:lines[fm[1]][0]] if fm[1] > fm[0] else b""
    try:
        v = yaml.load(block.decode("utf-8"), Loader=_Loader)
    except yaml.YAMLError:
        return {}
    return v if isinstance(v, dict) else {}


def aliases(props):
    a = props.get("aliases", props.get("alias"))
    if a is None:
        return []
    return [str(x) for x in (a if isinstance(a, list) else [a])]


TAG_RE = re.compile(r"(?<![^\s(\[{,;])#([^\s#!@$%^&*(),.?\":{}|<>\[\]`'+=;~]+)")


def tags(data):
    out, seen = [], set()

    def add(t):
        t = t.lstrip("#").strip()
        if t and not t.isdigit() and t.lower() not in seen:
            seen.add(t.lower())
            out.append(t)
    lines, kinds = M.body_line_kinds(data)
    for (s, e, _), k in zip(lines, kinds):
        if k != "text":
            continue
        txt = M.mask_inline_code(data[s:e].decode("utf-8"))
        if re.match(r"^#{1,6}\s", txt):
            txt = re.sub(r"^#{1,6}\s", "", txt)
        for m in TAG_RE.finditer(txt):
            if not re.search(r"[^0-9/]", m.group(1)):
                continue
            add(m.group(1))
    pt = properties(data).get("tags")
    if isinstance(pt, str):
        for t in pt.split(","):
            add(t)
    elif isinstance(pt, list):
        for t in pt:
            add(str(t))
    return out


def check_path(arg):
    if not isinstance(arg, str) or not arg.strip():
        raise ToolError("invalid_argument", "a note or path is required")
    if arg.startswith("/") or re.match(r"^[A-Za-z]:", arg) or ".." in arg.split("/"):
        raise ToolError("outside_vault", "use a vault-relative path without '..'")
    if FAULT != 9 and any(p.startswith(".") for p in arg.split("/")):
        raise ToolError("outside_vault", "dot-folders are not part of the vault")
    real = os.path.realpath(os.path.join(VAULT, arg))
    parent = os.path.realpath(os.path.join(VAULT, posixpath.dirname(arg)))
    root = os.path.realpath(VAULT)
    for p in (real, parent):
        if p != root and not p.startswith(root + os.sep):
            raise ToolError("outside_vault", "that path leaves the vault through a symlink")


def resolve_note(arg):
    check_path(arg)
    if FAULT == 9 and any(p.startswith(".") for p in arg.split("/")):
        if os.path.isfile(os.path.join(VAULT, arg)):
            return arg
    al = arg.lower()
    al_md = al if al.endswith(".md") else al + ".md"
    ns = notes()
    hits = [n for n in ns if n.lower() == al_md]
    if not hits:
        hits = [n for n in ns if n.lower().endswith("/" + al_md) or n.lower() == al_md]
    if not hits and FAULT != 4:
        hits = [n for n in ns if al in (a.lower() for a in aliases(properties(raw(n))))]
    if len(hits) == 1:
        return hits[0]
    if len(hits) > 1:
        raise ToolError("ambiguous", "several notes match; pass a path", candidates=sorted(hits))
    names = sorted({posixpath.basename(n)[:-3] for n in ns})
    cands = difflib.get_close_matches(arg, names, n=5, cutoff=0.4)
    raise ToolError("not_found", "no note matches; try one of the candidates", candidates=cands)


# ---------- paging ----------

def page_items(head, lists, page, extra_tail=None):
    """head: dict; lists: [(field, items)]. Splits so each reply fits."""
    flat = [(f, it) for f, items in lists for it in items]
    start = 0
    if page is not None:
        try:
            start = int(page)
            assert 0 <= start <= len(flat)
        except Exception:
            raise ToolError("bad_page", "pass a next_page token from this tool")
    if FAULT == 10 and head.get("_tool") == "find_notes":
        start, end = 0, len(flat)
    else:
        end = start
        while end < len(flat):
            if len(render(head, lists, flat, start, end + 1, end + 1 < len(flat)).encode()) > LIMIT:
                break
            end += 1
        if end == start and start < len(flat):
            end = start + 1
    return json.loads(render(head, lists, flat, start, end, end < len(flat)))


def render(head, lists, flat, start, end, more):
    obj = {k: v for k, v in head.items() if not k.startswith("_")}
    for f, _ in lists:
        obj[f] = []
    for f, it in flat[start:end]:
        obj[f].append(it)
    if more:
        obj["next_page"] = str(end)
    return json.dumps(obj, ensure_ascii=False)


# ---------- links ----------

def link_obj(src, l):
    return {"from": src, "to": l.to, "target": l.target, "kind": l.kind, "heading": l.heading,
            "block": l.block, "display": l.display, "line": l.line}


# ---------- tools ----------

def t_find_notes(a):
    text, tag, prop, folder = a.get("text"), a.get("tag"), a.get("property"), a.get("folder")
    if folder is not None:
        check_path(folder)
        if not os.path.isdir(os.path.join(VAULT, folder)):
            raise ToolError("not_found", "no such folder", candidates=[])
    out = []
    for n in notes():
        data = raw(n)
        if folder and not n.lower().startswith(folder.strip("/").lower() + "/"):
            continue
        ntags = tags(data)
        if tag is not None:
            t = tag.lstrip("#").lower()
            if FAULT == 5:
                ok = any(x.lower() == t for x in ntags)
            else:
                ok = any(x.lower() == t or x.lower().startswith(t + "/") for x in ntags)
            if not ok:
                continue
        if prop is not None:
            props = {k.lower(): v for k, v in properties(data).items()}
            if prop.get("name", "").lower() not in props:
                continue
            if "value" in prop:
                v = props[prop["name"].lower()]
                vals = v if isinstance(v, list) else [v]
                want = prop["value"]
                if not any(_eq(x, want) for x in vals):
                    continue
        item = {"path": n, "name": posixpath.basename(n)[:-3], "tags": ntags}
        if text is not None:
            s = data.decode("utf-8")
            if text.lower() not in s.lower() and text.lower() not in item["name"].lower():
                continue
            snips = []
            for i, line in enumerate(s.splitlines(), 1):
                if text.lower() in line.lower():
                    snips.append({"line": i, "text": line.strip()[:200]})
                    if len(snips) == 3:
                        break
            item["snippets"] = snips
        out.append(item)
    out.sort(key=lambda x: x["path"])
    return page_items({"_tool": "find_notes", "total": len(out)}, [("notes", out)], a.get("page"))


def _eq(x, want):
    if isinstance(x, str) and isinstance(want, str):
        return x.lower() == want.lower()
    if isinstance(x, bool) or isinstance(want, bool):
        return x == want
    return x == want or str(x) == str(want)


def t_read_note(a):
    path = resolve_note(a.get("note"))
    data = raw(path)
    props = properties(data)
    head = {"path": path, "name": posixpath.basename(path)[:-3], "version": version(data),
            "aliases": aliases(props), "tags": tags(data), "properties": props}
    body = M.body_after_properties(data).decode("utf-8")
    sec = a.get("section")
    if sec is not None:
        content = _section_text(data, sec)
        head.update(mode="section", section=sec)
        return _page_text(head, content, a.get("page"))
    whole = dict(head, mode="full", content=body)
    if not a.get("full") and len(json.dumps(whole, ensure_ascii=False).encode()) <= LIMIT:
        return whole
    if a.get("full"):
        head["mode"] = "full"
        return _page_text(head, body, a.get("page"))
    lines = M.split_lines(data)
    hs = M.headings(data)
    outline = []
    for li, lvl, text in hs:
        rng = M.section_range(data, text)
        outline.append({"section": text, "level": lvl, "line": li + 1, "bytes": rng[1] - rng[0] if rng else 0})
    head["mode"] = "outline"
    return page_items(head, [("outline", outline)], a.get("page"))


def _section_text(data, sec):
    hs = M.headings(data)
    if "#" not in sec:
        matching = [h for h in hs if h[2].lower() == sec.strip().lower()]
        if len(matching) > 1:
            raise ToolError("ambiguous_section", "add parent headings", headings=[h[2] for h in matching])
    rng = M.section_range(data, sec)
    if rng is None:
        raise ToolError("section_not_found", "use one of the headings", headings=[h[2] for h in hs])
    return data[rng[0]:rng[1]].decode("utf-8")


def _page_text(head, text, page):
    start = int(page) if page else 0
    rest = text[start:]
    budget = LIMIT - 600 - len(json.dumps(head, ensure_ascii=False).encode())
    chunk = ""
    for line in rest.splitlines(keepends=True):
        if len(json.dumps(chunk + line, ensure_ascii=False).encode()) > budget:
            break
        chunk += line
    if not chunk and rest:
        chunk = rest[:1000]
    obj = dict(head, content=chunk) if start == 0 else {"path": head["path"], "version": head["version"],
                                                       "mode": head["mode"], "content": chunk}
    if start + len(chunk) < len(text):
        obj["next_page"] = str(start + len(chunk))
    return obj


def _all_links():
    return M.vault_links(VAULT)


def t_get_links(a):
    path = resolve_note(a.get("note"))
    d = a.get("direction", "both")
    allk = _all_links()
    out = [link_obj(path, l) for l in allk.get(path, [])] if d in ("out", "both") else []
    if FAULT == 2:
        out = [o for o in out if o["kind"] != "embed"]
    inc = []
    if d in ("in", "both"):
        for src, ls in allk.items():
            if src != path:
                inc += [link_obj(src, l) for l in ls if l.to == path]
        inc.sort(key=lambda o: (o["from"], o["line"]))
    return page_items({"path": path}, [("outgoing", out), ("incoming", inc)], a.get("page"))


def t_check_vault(a):
    allk = _all_links()
    broken = []
    for src, ls in allk.items():
        for l in ls:
            if l.to is None:
                o = link_obj(src, l)
                o["reason"] = "missing_note"
                broken.append(o)
    broken.sort(key=lambda o: (o["from"], o["line"]))
    if FAULT == 3:
        broken = broken[:-1]
    linked = set()
    for src, ls in allk.items():
        for l in ls:
            if l.to and l.to != src and l.to.lower().endswith(".md"):
                linked.add(src)
                linked.add(l.to)
    un = [{"path": n, "name": posixpath.basename(n)[:-3]} for n in sorted(allk) if n not in linked]
    return page_items({"total_broken_links": len(broken), "total_unlinked_notes": len(un)},
                      [("broken_links", broken), ("unlinked_notes", un)], a.get("page"))


def _write(path, data):
    full = os.path.join(VAULT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "wb") as f:
        f.write(data)


def _eol(data):
    return b"\r\n" if b"\r\n" in data else b"\n"


def _check_version(data, v):
    if FAULT != 7 and v != version(data):
        raise ToolError("stale", "the note changed since you read it; read it again and retry", version=version(data))


def t_create_note(a):
    p = a.get("path")
    check_path(p)
    if not p.lower().endswith(".md"):
        p += ".md"
    if any(n.lower() == p.lower() for n in notes()) or os.path.exists(os.path.join(VAULT, p)):
        raise ToolError("already_exists", "pick another path", path=p)
    text = ""
    if a.get("properties"):
        text = "---\n" + yaml.safe_dump(a["properties"], allow_unicode=True, sort_keys=False) + "---\n"
    text += a.get("content") or ""
    _write(p, text.encode("utf-8"))
    return {"path": p, "name": posixpath.basename(p)[:-3], "version": version(text.encode())}


def t_edit_section(a):
    path = resolve_note(a.get("note"))
    data = raw(path)
    _check_version(data, a.get("version"))
    sec = a.get("section", "")
    _section_text(data, sec)
    s, e = M.section_range(data, sec)
    eol = _eol(data)
    content = (a.get("content") or "").encode("utf-8").replace(b"\n", eol)
    if not content.endswith(eol):
        content += eol
    head_end = data.find(b"\n", s)
    head_end = e if head_end == -1 or head_end >= e else head_end + 1
    mode = a.get("mode", "replace")
    sect = data[s:e]
    if mode == "replace":
        new = data[s:head_end] + (eol if head_end == e and not sect.endswith(b"\n") else b"") + eol + content
        if e < len(data):
            new += eol
    elif mode == "prepend":
        new = data[s:head_end] + content + data[head_end:e]
    else:
        body = sect
        trail = b""
        while body.endswith(eol + eol):
            body = body[:-len(eol)]
            trail += eol
        if not body.endswith(b"\n"):
            body += eol
        new = body + content + trail
    out = data[:s] + new + data[e:]
    _write(path, out)
    return {"path": path, "version": version(out)}


def _yaml_scalar(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if v is None:
        return "null"
    if isinstance(v, (int, float)):
        return json.dumps(v)
    s = str(v)
    if re.fullmatch(r"[A-Za-z][A-Za-z0-9 _-]*", s) and s.lower() not in ("true", "false", "null", "yes", "no", "on", "off"):
        return s
    return json.dumps(s, ensure_ascii=False)


def _prop_lines(name, v, eol):
    if isinstance(v, list):
        out = f"{name}:" + eol
        for x in v:
            out += f"  - {_yaml_scalar(x)}" + eol
        return out.encode("utf-8")
    return f"{name}: {_yaml_scalar(v)}{eol}".encode("utf-8")


def t_set_property(a):
    path = resolve_note(a.get("note"))
    data = raw(path)
    _check_version(data, a.get("version"))
    name = a.get("name")
    if not name:
        raise ToolError("invalid_argument", "name is required")
    value = a.get("value")
    eol = _eol(data).decode()
    rng = M.property_range(data, name)
    new = b"" if value is None else _prop_lines(name, value, eol)
    if rng:
        out = data[:rng[0]] + new + data[rng[1]:]
    elif value is None:
        out = data
    elif M.frontmatter_span(data):
        bounds = M.frontmatter_insert_bounds(data)
        at = bounds[-1]
        out = data[:at] + new + data[at:]
    else:
        out = ("---" + eol).encode() + new + ("---" + eol).encode() + data
    if FAULT == 6:
        out = out.replace(b"\r\n", b"\n")
    _write(path, out)
    return {"path": path, "version": version(out), "properties": properties(out)}


def _target_forms(link, new_path, src):
    np = new_path[:-3] if new_path.lower().endswith(".md") else new_path
    name = posixpath.basename(np)
    if link.kind == "markdown":
        rel = posixpath.relpath(new_path, posixpath.dirname(src) or ".")
        forms = []
        for p in ([rel] if link.target.startswith(".") else []) + [new_path, rel]:
            forms.append("<" + p + ">" if link.target.startswith("<") else quote(p, safe="/"))
        return forms
    return [name, np]


def t_move_note(a):
    src = resolve_note(a.get("note"))
    to = a.get("to")
    check_path(to)
    if not to.lower().endswith(".md"):
        to += ".md"
    if os.path.exists(os.path.join(VAULT, to)):
        raise ToolError("already_exists", "pick another path", path=to)
    before = _all_links()
    os.makedirs(os.path.dirname(os.path.join(VAULT, to)), exist_ok=True)
    os.rename(os.path.join(VAULT, src), os.path.join(VAULT, to))
    mp = lambda p: to if p == src else p  # noqa: E731
    files = M.list_files(VAULT)
    res = M.Resolver(files)
    updated = []
    for old_f, ls in before.items():
        f = mp(old_f)
        data = raw(f)
        links = M.parse_links(data)
        edits = []
        for lb, la in zip(ls, links):
            if lb.to is None:
                continue
            want = mp(lb.to)
            la.to = res.resolve(la, f)
            if la.to == want:
                continue
            if FAULT == 1 and la.kind == "markdown":
                continue
            for form in _target_forms(la, want, f):
                probe = M.Link(la.line, la.kind, form, None, None, None, 0, 0, 0, 0)
                if res.resolve(probe, f) == want:
                    edits.append((la.tstart, la.tend, form.encode("utf-8")))
                    break
        if edits:
            for s, e, b in sorted(edits, reverse=True):
                data = data[:s] + b + data[e:]
            _write(f, data)
            updated.append(f)
    return {"from": src, "path": to, "version": version(raw(to)), "updated_notes": sorted(updated)}


def t_delete_note(a):
    path = resolve_note(a.get("note"))
    allk = _all_links()
    broken = []
    for s, ls in allk.items():
        if s != path:
            broken += [link_obj(s, l) for l in ls if l.to == path]
    broken.sort(key=lambda o: (o["from"], o["line"]))
    if FAULT == 8:
        os.remove(os.path.join(VAULT, path))
    else:
        dest = os.path.join(VAULT, ".trash", path)
        while os.path.exists(dest):
            dest += "~"
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        shutil.move(os.path.join(VAULT, path), dest)
    for o in broken:
        o["to"] = None
    return {"path": path, "deleted": True, "now_broken": broken}


TOOLS = {"find_notes": t_find_notes, "read_note": t_read_note, "get_links": t_get_links,
         "check_vault": t_check_vault, "create_note": t_create_note, "edit_section": t_edit_section,
         "set_property": t_set_property, "move_note": t_move_note, "delete_note": t_delete_note}


# ---------- transport ----------

class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        if self.path != "/mcp":
            self.send_error(404)
            return
        n = int(self.headers.get("Content-Length", 0))
        msg = json.loads(self.rfile.read(n))
        if "id" not in msg:
            self.send_response(202)
            self.end_headers()
            return
        m = msg.get("method")
        if m == "initialize":
            result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                      "serverInfo": {"name": "standin", "version": "0"}}
        elif m == "tools/list":
            result = {"tools": [{"name": k, "inputSchema": {"type": "object"}} for k in TOOLS]}
        elif m == "tools/call":
            name = msg["params"]["name"]
            args = msg["params"].get("arguments") or {}
            try:
                if name not in TOOLS:
                    raise ToolError("invalid_argument", f"no tool {name}")
                obj, err = TOOLS[name](args), False
            except ToolError as e:
                obj, err = e.obj, True
            except (OSError, ValueError, TypeError, KeyError, AttributeError) as e:
                obj, err = {"error": "invalid_argument", "message": str(e)}, True
            result = {"content": [{"type": "text", "text": json.dumps(obj, ensure_ascii=False)}], "isError": err}
        else:
            result = {}
        body = json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    global FAULT, VAULT
    ap = argparse.ArgumentParser()
    ap.add_argument("--vault", required=True)
    ap.add_argument("--listen", required=True)
    ap.add_argument("--fault", type=int, default=0)
    a = ap.parse_args()
    FAULT, VAULT = a.fault, os.path.abspath(a.vault)
    host, port = a.listen.rsplit(":", 1)
    ThreadingHTTPServer((host, int(port)), H).serve_forever()


if __name__ == "__main__":
    main()
