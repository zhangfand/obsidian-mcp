"""Loads the hand-written expected data and expands the Logbook pattern."""

import json
import os

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXP = os.path.join(HERE, "expected")


def load(name):
    with open(os.path.join(EXP, name), encoding="utf-8") as f:
        return json.load(f)


def logbook_names():
    pat = load("links.json")["logbook_pattern"]
    return [pat["name"].format(n=n) for n in range(1, pat["count"] + 1)]


def link_table():
    """{note path: [[line, kind, target, heading, block, display, to], ...]}"""
    data = load("links.json")
    notes = dict(data["notes"])
    names = logbook_names()
    for name in names:
        notes[f"Logbook/{name}.md"] = [[4, "wikilink", "Ground Station", None, None, None, "Logbook/Ground Station.md"]]
    idx = [[3, "wikilink", "Ground Station", None, None, None, "Logbook/Ground Station.md"]]
    for k, name in enumerate(names):
        idx.append([5 + k, "wikilink", name, None, None, None, f"Logbook/{name}.md"])
    notes["Logbook/Logbook Index.md"] = idx
    for path, rows in data["huge"]["notes"].items():
        notes[expand(path)] = [[expand(x) if isinstance(x, str) else x for x in r] for r in rows]
    return notes


LONGNAME = ("Long target name " * 16)[:240].rstrip()
_WORDS = "alpha bravo charlie delta echo foxtrot golf hotel india juliett kilo lima mike november oscar papa".split()


def filler(n):
    out, i = [], 0
    while len(" ".join(out)) < n:
        out.append(_WORDS[i % len(_WORDS)] + str(i % 97))
        i += 1
    return " ".join(out)


def expand(s):
    """Placeholders of the 'huge' section of links.json (see its _doc)."""
    if s == "FILLER(20480)":
        return filler(20480)
    return s.replace("LONGNAME_URL", LONGNAME.replace(" ", "%20")).replace("LONGNAME", LONGNAME)


def all_notes():
    return sorted(link_table())


def incoming(table, path):
    """Incoming link rows of `path` as (from, line, kind, target, heading, block, display), sorted by from, line."""
    rows = []
    for src, links in table.items():
        if src == path:
            continue
        for l in links:
            if l[6] == path:
                rows.append((src, l[0], l[1], l[2], l[3], l[4], l[5]))
    return sorted(rows, key=lambda r: (r[0], r[1]))
