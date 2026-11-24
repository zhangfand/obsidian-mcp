"""One function per rule. Each returns a list of cases: (name, ok, detail).

Only rules 1-10 are here; rule 11 is checked on the real machine.
"""

import json
import os
import re

import expected as E
import model as M

MIN_CASES = {1: 20, 2: 20, 3: 15, 4: 20, 5: 15, 6: 30, 7: 1, 8: 1, 9: 10, 10: 1}
PHASE = {2: 1, 3: 1, 4: 1, 5: 1, 9: 1, 10: 1, 1: 2, 6: 2, 7: 2, 8: 2}


def j(x):
    return json.dumps(x, ensure_ascii=False)


def norm_target(kind, target):
    return M.norm_md_target(target) if kind == "markdown" else target


def link_key(d):
    """A link object from the server as a comparable tuple."""
    return (d.get("from"), d.get("to"), norm_target(d.get("kind"), d.get("target") or ""),
            d.get("kind"), d.get("heading"), d.get("block"), d.get("display"), d.get("line"))


def row_key(src, row):
    line, kind, target, heading, block, display, to = row
    return (src, to, norm_target(kind, target), kind, heading, block, display, line)


def read_disk(vault, rel):
    with open(os.path.join(vault, rel), "rb") as f:
        return f.read()


def collect(client, tool, args, *fields):
    reps, err = client.pages(tool, args)
    out = {f: [] for f in fields}
    for r in reps:
        if isinstance(r.data, dict):
            for f in fields:
                out[f].extend(r.data.get(f) or [])
    return reps, err, out


# ---------- rule 2 ----------

def rule2(ctx):
    c = ctx.client
    table = E.link_table()
    cases = []
    for path in E.load("get_links.json")["notes"]:
        reps, err, got = collect(c, "get_links", {"note": path, "direction": "both"}, "outgoing", "incoming")
        if err:
            cases.append((f"get_links {path}", False, err))
            continue
        exp_out = [row_key(path, r) for r in table[path]]
        exp_in = [(r[0], r[6] and path, norm_target(r[2], r[3]), r[2], r[4], r[5], r[6], r[1])
                  for r in E.incoming(table, path)]
        exp_in = [(f, path, t, k, h, b, d, ln) for (f, _, t, k, h, b, d, ln) in exp_in]
        got_out = [link_key(x) for x in got["outgoing"]]
        got_in = sorted((link_key(x) for x in got["incoming"]), key=lambda k: (k[0], k[7]))
        exp_in = sorted(exp_in, key=lambda k: (k[0], k[7]))
        problems = []
        if got_out != exp_out:
            problems.append(_diff("outgoing", exp_out, got_out))
        if got_in != exp_in:
            problems.append(_diff("incoming", exp_in, got_in))
        cases.append((f"get_links {path}", not problems, "; ".join(problems)))
    return cases


def _diff(label, exp, got):
    missing = [x for x in exp if x not in got]
    extra = [x for x in got if x not in exp]
    if not missing and not extra:
        return f"{label}: same items, wrong order or duplicates ({len(got)} vs {len(exp)})"
    return f"{label}: missing {j(missing[:2])} extra {j(extra[:2])} ({len(missing)} missing, {len(extra)} extra)"


# ---------- rule 3 ----------

def rule3(ctx):
    exp = E.load("check_vault.json")
    reps, err, got = collect(ctx.client, "check_vault", {}, "broken_links", "unlinked_notes")
    if err:
        return [("check_vault", False, err)]
    cases = []
    got_b = [(d.get("from"), d.get("line"), norm_target(d.get("kind"), d.get("target") or "")) for d in got["broken_links"]]
    for src, line, target in exp["broken_links"]:
        kind = "markdown" if target.endswith(".md") else "wikilink"
        key = (src, line, norm_target(kind, target))
        cases.append((f"broken link {src}:{line} {target}", key in got_b, "" if key in got_b else "not reported"))
    exp_b = {(s, l, norm_target("markdown" if t.endswith(".md") else "wikilink", t)) for s, l, t in exp["broken_links"]}
    for k in got_b:
        if k not in exp_b:
            cases.append((f"extra broken link {j(k)}", False, "reported but not seeded as broken"))
    if len(got_b) != len(set(got_b)):
        cases.append(("broken links reported once", False, "duplicates"))
    got_u = [d.get("path") for d in got["unlinked_notes"]]
    for p in exp["unlinked_notes"]:
        cases.append((f"unlinked note {p}", p in got_u, "" if p in got_u else "not reported"))
    for p in got_u:
        if p not in exp["unlinked_notes"]:
            cases.append((f"extra unlinked note {p}", False, "reported but has links"))
    return cases


# ---------- rule 4 ----------

def rule4(ctx):
    exp = E.load("lookups.json")
    cases = []
    for arg, path in exp["hits"]:
        r = ctx.client.call("read_note", {"note": arg})
        ok = not r.is_error and isinstance(r.data, dict) and r.data.get("path") == path
        cases.append((f"lookup {arg!r}", ok, "" if ok else f"want {path}, got {r.short()}"))
    for arg, name in exp["misses"]:
        r = ctx.client.call("read_note", {"note": arg})
        cands = (r.data or {}).get("candidates") if isinstance(r.data, dict) else None
        ok = (r.is_error and r.kind == "not_found" and isinstance(cands, list)
              and any(cand == name or (isinstance(cand, str) and cand.endswith("/" + name + ".md")) for cand in cands))
        cases.append((f"miss {arg!r}", ok, "" if ok else f"want not_found with {name!r} in candidates, got {r.short()}"))
    return cases


# ---------- rule 5 ----------

def rule5(ctx):
    exp = E.load("find_notes.json")
    logbook = [f"Logbook/{n}.md" for n in E.logbook_names()]
    cases = []
    for args, want in exp["queries"]:
        want = logbook if want == "LOGBOOK" else want
        name = f"find_notes {j(args)}"
        reps, err, got = collect(ctx.client, "find_notes", args, "notes")
        if err:
            cases.append((name, False, err))
            continue
        paths = [n.get("path") for n in got["notes"]]
        problems = []
        if set(paths) != set(want):
            problems.append(_diff("notes", sorted(want), sorted(set(paths))))
        if "text" in args:
            q = args["text"].lower()
            nosnip = [n.get("path") for n in got["notes"]
                      if not any(q in str(s.get("text", "")).lower() for s in (n.get("snippets") or []) if isinstance(s, dict))]
            if nosnip:
                problems.append(f"no snippet containing the text for {j(nosnip[:3])} ({len(nosnip)} notes)")
        cases.append((name, not problems, "; ".join(problems)))
    return cases


# ---------- rule 9 ----------

def rule9(ctx):
    exp = E.load("outside.json")
    vault = ctx.vault
    parent = os.path.dirname(os.path.abspath(vault))
    before_parent = set(os.listdir(parent))
    lonely = os.path.join(vault, "Orphans/Lonely.md")
    cases = []
    for tool, args in exp["cases"]:
        args = json.loads(j(args).replace("VAULT", os.path.abspath(vault)))
        r = ctx.client.call(tool, args)
        problems = []
        if not r.is_error:
            problems.append(f"not refused: {r.short()}")
        leaked = [m for m in exp["leak_markers"] if r.text and m in r.text]
        if leaked:
            problems.append(f"reply leaks {leaked}")
        if tool == "move_note" and not os.path.exists(lonely):
            problems.append("the note was moved")
        cases.append((f"{tool} {j(args)}", not problems, "; ".join(problems)))
    new = set(os.listdir(parent)) - before_parent
    cases.append(("nothing created outside the vault", not new, f"new entries beside the vault: {sorted(new)}" if new else ""))
    for p in [".obsidian/plugins/evil.md", ".git/Lonely.md"]:
        if os.path.exists(os.path.join(vault, p)):
            cases.append((f"nothing written in {p}", False, "file created"))
    return cases


# ---------- rule 10 ----------

def rule10_paging(ctx):
    """The paging cases; the size of every reply is judged in finish_rule10."""
    c = ctx.client
    logbook = [f"Logbook/{n}.md" for n in E.logbook_names()]
    table = E.link_table()
    cases = []

    def paged(name, tool, args, field, key, want, min_pages=2):
        reps, err = c.pages(tool, args)
        if err:
            cases.append((name, False, err))
            return
        items = []
        for r in reps:
            items.extend(r.data.get(field) or [])
        keys = [key(x) for x in items]
        problems = []
        if len(reps) < min_pages:
            problems.append(f"{len(reps)} page(s); result is larger than 8 KB so expected a next_page")
        dup = len(keys) - len(set(keys))
        if dup:
            problems.append(f"{dup} items repeated across pages")
        if set(keys) != set(want):
            problems.append(f"union of pages differs: {len(set(want) - set(keys))} missing, {len(set(keys) - set(want))} extra")
        cases.append((name, not problems, "; ".join(problems)))

    paged("find_notes text telemetry paged", "find_notes", {"text": "telemetry"}, "notes", lambda n: n.get("path"), logbook)
    paged("find_notes tag log paged", "find_notes", {"tag": "log"}, "notes", lambda n: n.get("path"), logbook)
    paged("find_notes all notes paged", "find_notes", {}, "notes", lambda n: n.get("path"), E.all_notes())
    idx = "Logbook/Logbook Index.md"
    paged("get_links outgoing paged", "get_links", {"note": idx, "direction": "out"}, "outgoing",
          link_key, [row_key(idx, r) for r in table[idx]])
    gs = "Logbook/Ground Station.md"
    want_in = [(f, gs, norm_target(k, t), k, h, b, d, ln) for (f, ln, k, t, h, b, d) in E.incoming(table, gs)]
    paged("get_links incoming paged", "get_links", {"note": gs, "direction": "in"}, "incoming", link_key, want_in)

    # A long note read whole: pages join to the exact text after the properties block.
    big = "Reference/Big Manual.md"
    body = M.body_after_properties(read_disk(ctx.vault, big)).decode("utf-8")
    reps, err = c.pages("read_note", {"note": big, "full": True})
    if err:
        cases.append(("read_note full paged", False, err))
    else:
        joined = "".join(r.data.get("content") or "" for r in reps)
        problems = []
        if len(reps) < 2:
            problems.append("one page for a note over 8 KB")
        if joined != body:
            problems.append(f"joined content differs from the file ({len(joined)} vs {len(body)} chars)")
        cases.append(("read_note full paged", not problems, "; ".join(problems)))
    r = c.call("read_note", {"note": big})
    ok = not r.is_error and isinstance(r.data, dict) and r.data.get("mode") == "outline"
    cases.append(("read_note long note gives an outline", ok, "" if ok else r.short()))
    return cases


def finish_rule10(ctx, cases):
    measured = len(ctx.client.replies)
    for tool, args, size in ctx.client.oversize:
        cases.append((f"reply size {tool} {j(args)[:120]}", False, f"{size} bytes > 8192"))
    cases.append((f"all {measured} replies of this run are at most 8192 bytes", not ctx.client.oversize,
                  f"{len(ctx.client.oversize)} oversize" if ctx.client.oversize else ""))
    return cases


# ---------- rule 6 ----------

def rule6(ctx):
    c, vault = ctx.client, ctx.vault
    cases = []
    for e in E.load("edits.json")["edits"]:
        kind, path = e[0], e[1]
        r = c.call("read_note", {"note": path})
        if r.is_error:
            cases.append((f"{kind} {path} {e[2]}", False, f"read_note failed: {r.short()}"))
            continue
        before = read_disk(vault, path)
        if kind == "section":
            _, _, sec, mode, text, gone = e
            name = f"edit_section {path} {sec!r} {mode}"
            rng = M.section_range(before, sec)
            if rng is None:
                raise RuntimeError(f"verifier bug: no section {sec!r} in {path}")
            rep = c.call("edit_section", {"note": path, "section": sec, "content": text, "mode": mode,
                                          "version": r.data.get("version")})
            after = read_disk(vault, path)
            problems = []
            if rep.is_error:
                problems.append(f"refused: {rep.short()}")
            else:
                s, end = rng
                tail = len(before) - end
                if after[:s] != before[:s] or (tail and after[-tail:] != before[end:]) or len(after) < s + tail:
                    problems.append("bytes outside the section changed")
                else:
                    inside = after[s:len(after) - tail].decode("utf-8", "replace")
                    if text not in inside:
                        problems.append("new text not inside the section")
                    for g in gone:
                        if g in inside:
                            problems.append(f"replace kept old text {g}")
                    head_end = before.find(b"\n", s) + 1 or end
                    if not after[s:].startswith(before[s:head_end]):
                        problems.append("heading line changed")
            cases.append((name, not problems, "; ".join(problems)))
        else:
            _, _, prop, value = e
            name = f"set_property {path} {prop}={j(value)}"
            rng = M.property_range(before, prop)
            rep = c.call("set_property", {"note": path, "name": prop, "value": value, "version": r.data.get("version")})
            after = read_disk(vault, path)
            problems = []
            if rep.is_error:
                problems.append(f"refused: {rep.short()}")
            else:
                if rng:
                    allowed = [rng]
                else:
                    allowed = [(b, b) for b in M.frontmatter_insert_bounds(before)]
                if not any(_only_inside(before, after, s, en) for s, en in allowed):
                    problems.append("bytes outside the property's lines changed")
                back = c.call("read_note", {"note": path})
                props = (back.data or {}).get("properties", {}) if isinstance(back.data, dict) else {}
                got = {k.lower(): v for k, v in props.items()}.get(prop.lower(), None)
                if value is None:
                    if prop.lower() in {k.lower() for k in props}:
                        problems.append("property still present")
                elif got != value:
                    problems.append(f"read back {j(got)}")
            cases.append((name, not problems, "; ".join(problems)))
    return cases


def _only_inside(before, after, s, e):
    tail = len(before) - e
    if len(after) < s + tail:
        return False
    return after[:s] == before[:s] and after[len(after) - tail:] == before[e:]


# ---------- rule 7 ----------

def rule7(ctx):
    c, vault = ctx.client, ctx.vault
    cases = []
    for kind, path, where, args in E.load("stale.json")["cases"]:
        name = f"stale {kind} {path} after a {where} change"
        r = c.call("read_note", {"note": path})
        if r.is_error:
            cases.append((name, False, f"read_note failed: {r.short()}"))
            continue
        old = r.data.get("version")
        data = read_disk(vault, path)
        if where == "body":
            changed = data + b"Changed on disk by someone else.\n"
        else:
            changed = data.replace(b"status: ", b"status: changed-", 1)
        with open(os.path.join(vault, path), "wb") as f:
            f.write(changed)
        tool = "edit_section" if kind == "section" else "set_property"
        rep = c.call(tool, dict(args, note=path, version=old))
        problems = []
        if not (rep.is_error and rep.kind == "stale"):
            problems.append(f"not refused as stale: {rep.short()}")
        elif not re.search(r"\bread", (rep.data or {}).get("message", ""), re.I):
            problems.append("message does not say to read again")
        if read_disk(vault, path) != changed:
            problems.append("file changed by the refused edit")
        # control: with a fresh version the same edit goes through
        r2 = c.call("read_note", {"note": path})
        rep2 = c.call(tool, dict(args, note=path, version=(r2.data or {}).get("version")))
        if rep2.is_error:
            problems.append(f"control edit with a fresh version refused: {rep2.short()}")
        cases.append((name, not problems, "; ".join(problems)))
    return cases


# ---------- rule 8 ----------

def rule8(ctx):
    c, vault = ctx.client, ctx.vault
    exp = E.load("deletes.json")
    table = E.link_table()
    cases = []
    for path, linkers in exp["deletes"]:
        name = f"delete {path}"
        data = read_disk(vault, path)
        rep = c.call("delete_note", {"note": path})
        problems = []
        if rep.is_error:
            problems.append(f"refused: {rep.short()}")
        else:
            if os.path.exists(os.path.join(vault, path)):
                problems.append("file still in place")
            if not _in_trash(vault, data, os.path.basename(path)):
                problems.append("no file with the same bytes in .trash/")
            got = sorted({(d.get("from"), d.get("line")) for d in rep.data.get("now_broken") or []})
            want = sorted(tuple(x) for x in linkers)
            if got != want:
                problems.append(f"now_broken lists {j(got)}, want {j(want)}")
            r = c.call("read_note", {"note": path})
            if not r.is_error:
                problems.append("deleted note still readable")
        cases.append((name, not problems, "; ".join(problems)))
    for arg, existing in exp["creates_taken"]:
        name = f"create on taken name {arg}"
        data = read_disk(vault, existing)
        rep = c.call("create_note", {"path": arg, "content": "Overwrite attempt."})
        problems = []
        if not (rep.is_error and rep.kind == "already_exists"):
            problems.append(f"not refused as already_exists: {rep.short()}")
        if read_disk(vault, existing) != data:
            problems.append("existing file changed")
        cases.append((name, not problems, "; ".join(problems)))
    arg, path, text = exp["create_new"]
    rep = c.call("create_note", {"path": arg, "content": text})
    ok = not rep.is_error and os.path.exists(os.path.join(vault, path)) and text.encode() in read_disk(vault, path)
    cases.append((f"create new {arg}", ok, "" if ok else rep.short()))
    data = read_disk(vault, path) if ok else b""
    rep = c.call("create_note", {"path": path, "content": "Second create."})
    ok2 = rep.is_error and rep.kind == "already_exists" and (not ok or read_disk(vault, path) == data)
    cases.append((f"create again {path}", ok2, "" if ok2 else rep.short()))
    return cases


def _in_trash(vault, data, base):
    root = os.path.join(vault, ".trash")
    for dp, _, files in os.walk(root):
        for f in files:
            with open(os.path.join(dp, f), "rb") as fh:
                if fh.read() == data:
                    return True
    return False


# ---------- rule 1 ----------

def snapshot(vault):
    files = M.list_all_files(vault)
    return {f: read_disk(vault, f) for f in files}


def rule1(ctx):
    c, vault = ctx.client, ctx.vault
    base_files = snapshot(vault)
    base_links = M.vault_links(vault)
    ident = {}  # current path -> original path
    for p in base_files:
        ident[p] = p
    cases = []
    for src, dst in E.load("moves.json")["moves"]:
        name = f"move {src} -> {dst}"
        rep = c.call("move_note", {"note": src, "to": dst})
        problems = []
        if rep.is_error:
            problems.append(f"refused: {rep.short()}")
        elif rep.data.get("path") != dst:
            problems.append(f"result path {rep.data.get('path')}")
        if not rep.is_error:
            ident[dst] = ident.pop(src)
        problems += _compare_vault(vault, base_files, base_links, ident)
        cases.append((name, not problems, "; ".join(problems[:3])))
    return cases


def _compare_vault(vault, base_files, base_links, ident):
    problems = []
    now = snapshot(vault)
    orig_to_now = {o: n for n, o in ident.items()}
    want_paths = {orig_to_now.get(p, p) for p in base_files}
    missing = sorted(want_paths - set(now))
    extra = sorted(set(now) - want_paths)
    if missing:
        problems.append(f"files missing: {missing[:3]}")
    if extra:
        problems.append(f"unexpected files: {extra[:3]}")
    links_now = M.vault_links(vault)
    for orig, data in base_files.items():
        cur = orig_to_now.get(orig, orig)
        if cur not in now:
            continue
        after = now[cur]
        if M.is_dot_path(orig) or not orig.lower().endswith(".md"):
            if after != data:
                problems.append(f"{cur}: changed but is not a note")
            continue
        bl = base_links.get(orig, [])
        al = links_now.get(cur, [])
        if len(bl) != len(al):
            problems.append(f"{cur}: {len(al)} links, was {len(bl)}")
            continue
        # text outside link targets must be identical
        pos_b = pos_a = 0
        for lb, la in zip(bl, al):
            if data[pos_b:lb.tstart] != after[pos_a:la.tstart]:
                problems.append(f"{cur}:{la.line}: text outside link targets changed")
                break
            pos_b, pos_a = lb.tend, la.tend
            want = orig_to_now.get(lb.to, lb.to) if lb.to else None
            if la.to != want:
                problems.append(f"{cur}:{la.line}: link {after[la.start:la.end].decode()!r} resolves to {la.to}, want {want}")
                break
        else:
            if data[pos_b:] != after[pos_a:]:
                problems.append(f"{cur}: text outside link targets changed")
    return problems
