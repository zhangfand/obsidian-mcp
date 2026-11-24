# obsidian-mcp tools

The exact argument and result shapes of the nine tools. This file is the
contract between the server and anything that calls it.

## Transport

`obsidian-mcp --vault <dir> --listen 127.0.0.1:<port>` serves MCP Streamable
HTTP at `/mcp`. Each POST is answered alone (stateless, no session id), with
a JSON body (`Content-Type: application/json`). Clients send
`Accept: application/json, text/event-stream`. The exact handshake is in
`server/AGENTS.md` under "Talking to the server".

Every tool is called with `tools/call`, `params.name` = the tool name and
`params.arguments` = the argument object below.

## Common rules

### Results

A successful result is one `text` content item holding one JSON object, the
"result" shown for each tool. `isError` is `false`.

An error is one `text` content item holding one JSON object, with
`isError: true`:

```json
{ "error": "<kind>", "message": "<what went wrong and what to try next>", ...extra fields }
```

Error kinds (the `error` field), and their extra fields:

| kind | when | extra fields |
|---|---|---|
| `not_found` | `note` matches no note | `"candidates": [string]` up to 5 closest note names, closest first (may be empty) |
| `ambiguous` | `note` matches more than one note | `"candidates": [string]` the vault-relative paths of every match, sorted |
| `outside_vault` | a path argument contains `..`, is absolute, goes through a dot-folder (`.obsidian`, `.git`, `.trash`, any `.x`), or a symlink that leaves the vault | none |
| `section_not_found` | `section` matches no heading in the note | `"headings": [string]` the note's headings as section paths (see below) |
| `ambiguous_section` | `section` matches more than one heading | `"headings": [string]` the section paths of every match, in file order |
| `invalid_argument` | an argument is missing, of the wrong type, or empty where a value is needed | none |
| `bad_page` | `page` is not a token this tool gave for these same arguments | none |
| `stale` | (change tools) the note changed since the `version` passed | `"version": string` the note's current version |
| `already_exists` | (change tools) the target path is taken | `"path": string` |
| `io_error` | the file system refused (permissions, disk) | none |

### Notes

- A note is a `.md` file inside the vault, outside any dot-folder.
- `path` in results is always vault-relative, with `/` separators and the
  `.md` extension, e.g. `"Projects/Alpha.md"`.
- `name` is the file name without `.md`, e.g. `"Alpha"`.

### The `note` argument

Every tool that acts on one note takes `"note": string`. It is resolved in
this order, case-insensitively at every step:

1. a vault-relative path, with or without `.md` (`"Projects/Alpha"`,
   `"Projects/Alpha.md"`);
2. a note name (`"Alpha"`), or a path suffix (`"Projects/Alpha"` when the
   note is `Work/Projects/Alpha.md`);
3. an alias from the note's `aliases` property.

The first step with any match decides. One match is used; several matches
give `ambiguous`; none at all give `not_found` with the closest names.
`..`, absolute paths and dot-folders give `outside_vault`.

### Paging

No result text is larger than 8192 bytes (UTF-8). A tool whose result would
be larger returns the first part with `"next_page": string` (opaque). Call the
same tool again with **the same arguments** plus `"page": <that string>` to
get the next part. The last page has no `next_page` field. Reading every page
returns each list item exactly once, in a stable order. Tokens carry no
server state; if the vault changes between pages, items may shift.

Where a long text is paged (`read_note` content), each page carries a
`content` chunk; joining the chunks of all pages in order gives the whole text
exactly. Chunks break at line ends where possible.

### Versions

`version` is an opaque string that changes whenever the note's file bytes
change (a hash of the whole file, properties included). `read_note` returns
it. `edit_section` and `set_property` take it and refuse with `stale` when the
file changed since.

### Sections

A section is a heading and everything under it up to the next heading of the
same or a higher level (fewer `#`). Headings inside code blocks are not
headings. A **section path** names a heading by its text, without `#`
marks: `"Plan"`. When the same text appears more than once, add parent
headings joined by `#`: `"Week 1#Plan"`. Matching is case-insensitive and
ignores surrounding spaces. A section path that matches several headings is
refused with `ambiguous_section` (by `read_note` and `edit_section` alike),
listing the full section paths of every match; pass one of them.

### Links (shared by `get_links` and `check_vault`)

A link is one occurrence in a note's text, outside code (fenced, indented
and inline code) and outside the properties block:

| form | `kind` |
|---|---|
| `[[target]]`, `[[target#heading]]`, `[[target#^block]]`, `[[target\|text]]`, `[[#heading]]` | `"wikilink"` |
| `![[target]]` and the same variants | `"embed"` |
| `[text](target.md)`, `[text](target.md#heading)`, `[text](<target with spaces.md>)`, `[text](target%20name.md)` | `"markdown"` |
| `![alt](image.png)` | `"embed"` |

Markdown links whose target has a URL scheme (`https:`, `mailto:`, ...) are
not links in this sense and never appear. Wikilinks inside property values
(e.g. `related: "[[Alpha]]"`), as Obsidian counts them, are links of kind
`"wikilink"` with the line inside the properties block. Markdown links in
properties are not links.

A link object:

```json
{
  "from": "Daily/2026-01-02.md",
  "to": "Projects/Alpha.md",
  "target": "Alpha",
  "kind": "wikilink",
  "heading": "Plan",
  "block": null,
  "display": "the plan",
  "line": 12
}
```

- `from`: path of the note that contains the link.
- `to`: path of the file it resolves to (a note, or any other file such as
  `Assets/diagram.png`), or `null` when it resolves to nothing.
- `target`: the link target as written, without `#heading`, `#^block` and
  `|display` (`""` for `[[#heading]]`).
- `heading`: the section path after `#` (`"A#B"` for `[[n#A#B]]`), or `null`.
- `block`: the block id after `#^`, without `^`, or `null`.
- `display`: the text after `|` (wikilink) or inside `[...]` (markdown), or
  `null`.
- `line`: 1-based line number in the file (properties block lines counted).

Resolution follows Obsidian:

1. `[[#heading]]` / `[[#^block]]` (empty target) point at the note itself.
2. A target with an extension other than `.md` (`diagram.png`) names any
   file; without an extension it names a note (`.md` is added).
3. Match the target, case-insensitively, as a vault-relative path; then
   (markdown links only) relative to the linking note's folder; then as a
   file name or path suffix anywhere in the vault.
4. When several files share the name, prefer the one in the linking note's
   folder, then the one with the shortest path, then the alphabetically first.
5. **Aliases do not resolve links.** As in Obsidian, `[[My Alias]]` is
   unresolved unless a file is named `My Alias`; the alias form Obsidian
   writes is `[[Real Name|My Alias]]`, which resolves to `Real Name`.

## Phase 1 tools (read)

### `find_notes`

Find notes by text, tag, property and folder. Every given filter must match
(AND). With no filter, every note matches.

Arguments:

```json
{
  "text": "string, optional",
  "tag": "string, optional",
  "property": { "name": "string", "value": "any JSON, optional" },
  "folder": "string, optional",
  "page": "string, optional"
}
```

- `text`: case-insensitive substring of the note's name or of its whole file
  text (properties block included).
- `tag`: with or without a leading `#`, case-insensitive. A note matches when
  it has the tag or a nested tag below it: `project` matches `#project` and
  `#project/alpha`, not `#projects`. Tags count from the body (`#tag`) and
  from the `tags` property (`tags: [a, b]`, `tags: a`, `tags: "a, b"`).
- `property`: the note has the property `name` (case-insensitive). With
  `value`: the property's value equals it, or (for a list) contains it.
  Strings compare case-insensitively; numbers and booleans compare as
  values, also against their text form (`"3"` matches `3`).
- `folder`: vault-relative folder; only notes inside it or its subfolders.

Result:

```json
{
  "notes": [
    {
      "path": "Projects/Alpha.md",
      "name": "Alpha",
      "tags": ["project/alpha"],
      "snippets": [ { "line": 14, "text": "...the matching line, trimmed..." } ]
    }
  ],
  "total": 1,
  "next_page": "string, only when more pages follow"
}
```

`notes` is sorted by `path`. `total` is the number of matching notes across
all pages. `tags` is the note's full tag list (see `read_note`).

`snippets` is present only when `text` is given: up to 3 lines of the file
that contain the text (case-insensitive), in file order. `line` is 1-based
(properties block lines counted); `text` is the line trimmed to about 200
bytes around the match. A note that matches only by name has
`"snippets": []`.

### `read_note`

Read one note: its properties, and its text in full, as an outline, or one
section.

Arguments:

```json
{
  "note": "string",
  "section": "string, optional (a section path)",
  "full": "boolean, optional, default false",
  "page": "string, optional"
}
```

Result:

```json
{
  "path": "Projects/Alpha.md",
  "name": "Alpha",
  "version": "9f2c4e01a7b3d5e6",
  "aliases": ["A"],
  "tags": ["project/alpha", "active"],
  "properties": { "aliases": ["A"], "status": "active", "due": "2026-02-01" },
  "mode": "full",
  "content": "string (see mode)",
  "outline": [ { "section": "Plan", "level": 2, "line": 8, "bytes": 1200 } ],
  "next_page": "string, only when more pages follow"
}
```

`mode` is one of:

- `"full"`: `content` is the note's text after the properties block. Used
  when `section` is absent and the whole result fits in one 8 KB reply, or
  when `full: true` (then paged if needed). No `outline` field.
- `"outline"`: the note is too long to return whole and no `section` was
  asked. No `content` field. `outline` lists every heading in file order:
  `section` is its section path (shortest one that is unique), `level` is
  1-6, `line` is the 1-based line of the heading, `bytes` is the size of the
  section's text. Ask again with `section` to read a part. A very long
  outline is paged.
- `"section"`: `section` was given. `content` is that section's text,
  heading line included; a field `"section": "<the section path>"` is
  added. A section larger than one reply is paged.

`aliases` comes from the `aliases` property (list or single string; `alias`
is accepted too). `tags` are the note's tags without `#`, from body and
properties, deduplicated case-insensitively, spelled as first seen.
`properties` is the properties block as JSON (`{}` when absent); YAML dates
are strings. On pages after the first, only `path`, `version`, `mode`,
`content` or `outline`, and `next_page` are present.

### `get_links`

Links of one note in both directions.

Arguments:

```json
{
  "note": "string",
  "direction": "\"out\" | \"in\" | \"both\", optional, default \"both\"",
  "page": "string, optional"
}
```

Result:

```json
{
  "path": "Projects/Alpha.md",
  "outgoing": [ { "from": "Projects/Alpha.md", "to": "Beta.md", "target": "Beta", "kind": "wikilink", "heading": null, "block": null, "display": null, "line": 5 } ],
  "incoming": [ { "from": "Daily/2026-01-02.md", "to": "Projects/Alpha.md", "target": "A", "kind": "embed", "heading": null, "block": null, "display": null, "line": 3 } ]
}
```

- `outgoing`: every link occurrence in this note, in file order, resolved
  or not. Links to the note itself (`[[#Plan]]`) are included with
  `to` = this note.
- `incoming`: every link occurrence in **other** notes that resolves to
  this note, sorted by `from` then `line`.
- Both lists are always present (empty when `direction` excludes them).
- Paging: `outgoing` items come first, then `incoming`; a page holds a
  slice of each list, and each occurrence appears on exactly one page.

### `check_vault`

Broken links and unlinked notes in the whole vault.

Arguments:

```json
{ "page": "string, optional" }
```

Result:

```json
{
  "broken_links": [
    { "from": "Inbox.md", "to": null, "target": "Missing Note", "kind": "wikilink", "heading": null, "block": null, "display": null, "line": 4, "reason": "missing_note" }
  ],
  "unlinked_notes": [ { "path": "Old/Stray.md", "name": "Stray" } ],
  "total_broken_links": 1,
  "total_unlinked_notes": 1,
  "next_page": "string, only when more pages follow"
}
```

- A **broken link** is a link (any kind) with `to: null`; `reason` is
  `"missing_note"` (a note-style target) or `"missing_file"` (a target with a
  non-`.md` extension). Links whose file exists but whose `#heading` or
  `#^block` does not are **not** reported in phase 1.
- An **unlinked note** is a note with no resolved link to another note and
  no resolved link from another note (Obsidian's graph "orphan"). Links to
  itself and links to non-note files do not count.
- `broken_links` sorted by `from` then `line`; `unlinked_notes` by `path`.
  Paging: `broken_links` first, then `unlinked_notes`.

## Phase 2 tools (change) — shapes fixed now, built later

All change tools refuse `outside_vault` paths and resolve `note` as above.
Each returns the note's new `version`.

### `create_note`

```json
{ "path": "Projects/Gamma.md", "content": "string, optional (body text)", "properties": { "any": "JSON, optional" } }
```

`path` is vault-relative; `.md` is added when missing; missing folders are
created. Result: `{ "path": "Projects/Gamma.md", "name": "Gamma", "version": "..." }`.
Errors: `already_exists`, `outside_vault`, `invalid_argument`.

### `edit_section`

```json
{
  "note": "string",
  "section": "string (a section path)",
  "content": "string",
  "mode": "\"replace\" | \"append\" | \"prepend\", optional, default \"replace\"",
  "version": "string (from read_note)"
}
```

`replace` replaces the section's text below its heading line (the heading
line stays); `append` adds `content` at the end of the section; `prepend`
just below the heading line. Result: `{ "path": "...", "version": "..." }`.
Errors: `stale` (with current `version`), `section_not_found`,
`ambiguous_section`, `not_found`, `ambiguous`.

### `set_property`

```json
{ "note": "string", "name": "string", "value": "any JSON; null removes the property", "version": "string" }
```

Creates the properties block when absent. Result:
`{ "path": "...", "version": "...", "properties": { ... } }`.
Errors: `stale`, `not_found`, `ambiguous`, `invalid_argument`.

### `move_note`

```json
{ "note": "string", "to": "string (new vault-relative path)" }
```

Moves or renames the note. Every link that resolved to the note is always
rewritten to keep resolving to it (as Obsidian does on rename); no move
leaves a link broken.
Result: `{ "from": "old/path.md", "path": "new/path.md", "version": "...", "updated_notes": ["paths of notes whose links were rewritten"] }`.
Errors: `already_exists`, `outside_vault`, `not_found`, `ambiguous`.

### `delete_note`

```json
{ "note": "string" }
```

Moves the note to the vault's `.trash/` folder (Obsidian's "move to Obsidian
trash"), so it can be restored by hand. Result:
`{ "path": "deleted/path.md", "deleted": true, "now_broken": [link objects that resolved to it] }`.
Errors: `not_found`, `ambiguous`.
