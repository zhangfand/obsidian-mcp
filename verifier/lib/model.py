"""The verifier's own reading of an Obsidian vault on disk.

Link forms and resolution follow help.obsidian.md (Internal links, Embed
files, Properties) as pinned down in server/TOOLS.md. The verifier uses this
to resolve links itself (rule 1), to find byte ranges of sections and
properties (rule 6), and checks it against the hand-written expected/links.json
before every run, so a bug here stops the run instead of scoring the server.
"""

import os
import posixpath
import re
from dataclasses import dataclass, field
from urllib.parse import unquote

WIKI_RE = re.compile(r"(!?)\[\[([^\[\]\n]*?)\]\]")
MD_RE = re.compile(r"(!?)\[([^\[\]\n]*)\]\((<[^>\n]*>|[^()\s]*)\)")
SCHEME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
FENCE_RE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
HEADING_RE = re.compile(rb"^(#{1,6})[ \t]+(.*?)[ \t#]*$")


@dataclass
class Link:
    line: int
    kind: str          # wikilink | embed | markdown
    target: str        # as written, without #heading/#^block/|display
    heading: str | None
    block: str | None
    display: str | None
    start: int         # byte offsets of the whole link in the file
    end: int
    tstart: int        # byte offsets of the target part (the only part a move may change)
    tend: int
    to: str | None = None
    ambiguous: list | None = None  # candidate paths when a name matches several files

    def row(self):
        r = [self.line, self.kind, self.target, self.heading, self.block, self.display, self.to]
        return r + [self.ambiguous] if self.ambiguous else r


def split_lines(data: bytes):
    """[(start, end_without_eol, end_with_eol)] for each line."""
    out, pos = [], 0
    n = len(data)
    while pos < n:
        nl = data.find(b"\n", pos)
        if nl == -1:
            out.append((pos, n, n))
            break
        e = nl
        if e > pos and data[e - 1:e] == b"\r":
            e -= 1
        out.append((pos, e, nl + 1))
        pos = nl + 1
    return out


def frontmatter_span(data: bytes):
    """(first_body_line_index, closing_line_index) of the properties block, or None."""
    lines = split_lines(data)
    if not lines or data[lines[0][0]:lines[0][1]] != b"---":
        return None
    for i in range(1, len(lines)):
        if data[lines[i][0]:lines[i][1]] == b"---":
            return (1, i)
    return None


def body_line_kinds(data: bytes):
    """Per line: 'fm' (properties block incl. fences), 'code' (fenced code), or 'text'."""
    lines = split_lines(data)
    kinds = ["text"] * len(lines)
    fm = frontmatter_span(data)
    first = 0
    if fm:
        for i in range(0, fm[1] + 1):
            kinds[i] = "fm"
        first = fm[1] + 1
    fence = None
    for i in range(first, len(lines)):
        s = data[lines[i][0]:lines[i][1]].decode("utf-8", "replace")
        m = FENCE_RE.match(s)
        if fence is None:
            if m:
                fence = m.group(1)
                kinds[i] = "code"
        else:
            kinds[i] = "code"
            if m and m.group(1)[0] == fence[0] and len(m.group(1)) >= len(fence) and s.strip() == m.group(1):
                fence = None
    return lines, kinds


def mask_inline_code(s: str) -> str:
    out = list(s)
    i = 0
    while i < len(s):
        if s[i] == "`":
            j = i
            while j < len(s) and s[j] == "`":
                j += 1
            tick = s[i:j]
            k = s.find(tick, j)
            if k == -1:
                i = j
                continue
            for x in range(i, k + len(tick)):
                out[x] = " "
            i = k + len(tick)
        else:
            i += 1
    return "".join(out)


def _bpos(s: str, i: int) -> int:
    return len(s[:i].encode("utf-8"))


def parse_wiki_inner(inner: str):
    display = None
    if "|" in inner:
        inner, display = inner.split("|", 1)
    heading = block = None
    target = inner
    if "#" in inner:
        target, rest = inner.split("#", 1)
        if rest.startswith("^"):
            block = rest[1:]
        else:
            heading = rest
    return target, heading, block, display


def parse_links(data: bytes):
    lines, kinds = body_line_kinds(data)
    links = []
    for i, (s0, e0, _) in enumerate(lines):
        if kinds[i] == "code":
            continue
        raw = data[s0:e0].decode("utf-8", "replace")
        text = raw if kinds[i] == "fm" else mask_inline_code(raw)
        found = []
        for m in WIKI_RE.finditer(text):
            if kinds[i] == "fm" and m.group(1):
                continue
            target, heading, block, display = parse_wiki_inner(m.group(2))
            kind = "embed" if m.group(1) else "wikilink"
            ts = m.start(2)
            found.append((m.start(), Link(i + 1, kind, target, heading, block, display,
                                          s0 + _bpos(raw, m.start()), s0 + _bpos(raw, m.end()),
                                          s0 + _bpos(raw, ts), s0 + _bpos(raw, ts + len(target)))))
        if kinds[i] == "text":
            for m in MD_RE.finditer(text):
                dest = m.group(3)
                if not dest or SCHEME_RE.match(dest.strip("<>")):
                    continue
                inside = any(w.start <= s0 + _bpos(raw, m.start()) < w.end for _, w in found)
                if inside:
                    continue
                heading = block = None
                target = dest
                if not dest.startswith("<") and "#" in dest:
                    target, frag = dest.split("#", 1)
                    if frag.startswith("^"):
                        block = frag[1:]
                    else:
                        heading = unquote(frag)
                kind = "embed" if m.group(1) else "markdown"
                ts = m.start(3)
                found.append((m.start(), Link(i + 1, kind, target, heading, block, m.group(2),
                                              s0 + _bpos(raw, m.start()), s0 + _bpos(raw, m.end()),
                                              s0 + _bpos(raw, ts), s0 + _bpos(raw, ts + len(target)))))
        found.sort(key=lambda x: x[0])
        links.extend(l for _, l in found)
    return links


def norm_md_target(t: str) -> str:
    """A markdown destination as a path: angle brackets dropped, %-escapes decoded."""
    if t.startswith("<") and t.endswith(">"):
        t = t[1:-1]
    return unquote(t)


def is_dot_path(rel: str) -> bool:
    return any(p.startswith(".") for p in rel.split("/"))


def list_files(vault: str):
    """Vault-relative paths of every regular file outside dot-folders, not via symlinks."""
    out = []
    for root, dirs, files in os.walk(vault, followlinks=False):
        rel_root = os.path.relpath(root, vault)
        rel_root = "" if rel_root == "." else rel_root.replace(os.sep, "/")
        dirs[:] = sorted(d for d in dirs if not d.startswith(".") and not os.path.islink(os.path.join(root, d)))
        for f in sorted(files):
            if f.startswith("."):
                continue
            full = os.path.join(root, f)
            if os.path.islink(full):
                continue
            out.append(f"{rel_root}/{f}" if rel_root else f)
    return sorted(out)


def list_all_files(vault: str):
    """Every regular file, dot-folders included (for byte-for-byte diffs)."""
    out = []
    for root, dirs, files in os.walk(vault, followlinks=False):
        dirs[:] = [d for d in dirs if not os.path.islink(os.path.join(root, d)) and d != ".git"]
        for f in files:
            full = os.path.join(root, f)
            if os.path.islink(full):
                continue
            out.append(os.path.relpath(full, vault).replace(os.sep, "/"))
    return sorted(out)


class Resolver:
    def __init__(self, files):
        self.files = list(files)
        self.by_lower = {}
        for f in self.files:
            self.by_lower.setdefault(f.lower(), []).append(f)

    def resolve(self, link: Link, src: str):
        return self.resolve_full(link, src)[0]

    def resolve_full(self, link: Link, src: str):
        """(path or None, candidate paths when ambiguous or None)."""
        t = norm_md_target(link.target) if link.kind == "markdown" else link.target
        if t == "":
            return src, None
        last = t.rsplit("/", 1)[-1]
        ext = os.path.splitext(last)[1]
        if not ext or ext.lower() == ".md" or not re.fullmatch(r"\.[A-Za-z0-9]+", ext):
            if not t.lower().endswith(".md"):
                t = t + ".md"
        tl = t.lower()
        # 1. vault-relative path
        if ".." not in tl.split("/") and not tl.startswith("/"):
            hit = self.by_lower.get(posixpath.normpath(tl))
            if hit:
                return sorted(hit)[0], None
        # 2. markdown links: relative to the linking note
        if link.kind == "markdown":
            rel = posixpath.normpath(posixpath.join(posixpath.dirname(src), tl))
            if not rel.startswith(".."):
                hit = self.by_lower.get(rel)
                if hit:
                    return sorted(hit)[0], None
        # 3. name or path suffix anywhere
        cands = [f for f in self.files if f.lower() == tl or f.lower().endswith("/" + tl)]
        if not cands:
            return None, None
        if len(cands) > 1:
            # Owner's decision (rule set v2): a shared name is never settled by a default.
            return None, sorted(cands)
        return cands[0], None


def vault_links(vault: str):
    """{note path: [Link]} for every note, resolved."""
    files = list_files(vault)
    res = Resolver(files)
    out = {}
    for f in files:
        if not f.lower().endswith(".md"):
            continue
        with open(os.path.join(vault, f), "rb") as fh:
            data = fh.read()
        links = parse_links(data)
        for l in links:
            l.to, l.ambiguous = res.resolve_full(l, f)
        out[f] = links
    return out


# ---- sections and properties (byte ranges) ----

def headings(data: bytes):
    """[(line_index, level, text)] outside properties and code."""
    lines, kinds = body_line_kinds(data)
    out = []
    for i, (s, e, _) in enumerate(lines):
        if kinds[i] != "text":
            continue
        m = HEADING_RE.match(data[s:e])
        if m:
            out.append((i, len(m.group(1)), m.group(2).decode("utf-8").strip()))
    return out


def section_range(data: bytes, path: str):
    """Byte range [start, end) of the section named by a section path ('A#B'),
    heading line included. None when not found."""
    lines, _ = body_line_kinds(data)
    hs = headings(data)
    parts = [p.strip().lower() for p in path.split("#")]
    for k, (li, lvl, text) in enumerate(hs):
        if text.lower() != parts[-1]:
            continue
        # parents: walk back collecting enclosing headings
        chain, cur = [], lvl
        for j in range(k - 1, -1, -1):
            if hs[j][1] < cur:
                chain.append(hs[j][2].lower())
                cur = hs[j][1]
        want = parts[:-1][::-1]
        if want and chain[:len(want)] != want and not _subseq(want, chain):
            continue
        start = lines[li][0]
        end = len(data)
        for j in range(k + 1, len(hs)):
            if hs[j][1] <= lvl:
                end = lines[hs[j][0]][0]
                break
        return (start, end)
    return None


def lead_range(data: bytes):
    """[start, end) of the lead: after the properties block, before the first heading."""
    lines, _ = body_line_kinds(data)
    fm = frontmatter_span(data)
    start = lines[fm[1]][2] if fm else 0
    hs = headings(data)
    end = lines[hs[0][0]][0] if hs else len(data)
    return (start, end)


def _blank(b: bytes) -> bool:
    return b.strip() == b""


def section_body_range(data: bytes, path: str):
    """[start, end) of a section's body (TOOLS.md "Sections"): below the
    heading line, without the blank lines directly under it and at its end.
    path "" is the lead. When the body is only blank lines it is empty at the
    section's end, where TOOLS.md puts new content."""
    if path == "":
        s, e = lead_range(data)
        b = s
    else:
        rng = section_range(data, path)
        if rng is None:
            return None
        s, e = rng
        nl = data.find(b"\n", s)
        b = e if nl == -1 or nl >= e else nl + 1
    seg = [ln for ln in split_lines(data) if b <= ln[0] < e]
    k = 0
    while k < len(seg) and _blank(data[seg[k][0]:seg[k][1]]):
        k += 1
    if k == len(seg):
        return (e, e)
    m = len(seg)
    while m > k and _blank(data[seg[m - 1][0]:seg[m - 1][1]]):
        m -= 1
    return (seg[k][0], seg[m - 1][2])


def _subseq(want, chain):
    it = iter(chain)
    return all(w in it for w in want)


def property_range(data: bytes, name: str):
    """Byte range of the lines of property `name` in the properties block, or None."""
    fm = frontmatter_span(data)
    if not fm:
        return None
    lines = split_lines(data)
    i = fm[0]
    while i < fm[1]:
        s, e, full = lines[i]
        line = data[s:e].decode("utf-8")
        m = re.match(r"^([^\s#:-][^:]*):(\s|$)", line)
        j = i + 1
        while j < fm[1]:
            nxt = data[lines[j][0]:lines[j][1]].decode("utf-8")
            if nxt[:1] in (" ", "\t", "-") or nxt == "":
                j += 1
            else:
                break
        if m and m.group(1).strip().lower() == name.lower():
            return (lines[i][0], lines[j - 1][2])
        i = j
    return None


def frontmatter_insert_bounds(data: bytes):
    """Byte offsets where a new property's lines may go: every line start
    inside the properties block; [0] when there is no block."""
    fm = frontmatter_span(data)
    if not fm:
        return [0]
    lines = split_lines(data)
    return [lines[i][0] for i in range(fm[0], fm[1] + 1)]


def body_after_properties(data: bytes) -> bytes:
    fm = frontmatter_span(data)
    if not fm:
        return data
    return data[split_lines(data)[fm[1]][2]:]
