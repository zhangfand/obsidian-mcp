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
| `outside_vault` | a path argument (`note` of every tool, `folder` of `find_notes`, `path`/`to` of the change tools) contains `..`, is absolute, goes through a dot-folder (`.obsidian`, `.git`, `.trash`, any `.x`), or goes through a symlink (file or folder) that leads outside the vault | none |
| `section_not_found` | `section` matches no heading in the note, or no heading is on `heading_line` | `"headings": [string]` the note's headings as section paths (see below) |
| `ambiguous_section` | `section` matches more than one heading | `"headings": [string]` the section paths of every match, in file order |
| `too_large` | last resort: a reply would still be over 8 KB after the rules under "Large items" | none |
| `invalid_argument` | an argument is missing, of the wrong type, or empty where a value is needed | none |
| `bad_page` | `page` is not a token this tool gave for these same arguments | none |
| `stale` | (change tools) the note changed since the `version` passed | `"version": string` the note's current version |
| `already_exists` | (change tools) the target path, or for `create_note` the note name, is taken | `"path": string` the existing file |
| `invalid_properties` | (`set_property`) the note's properties block is not valid YAML | none |
| `text_not_found` | (`edit_note`) `old_text` does not occur in the note's text after the properties block | `"in_properties": true` when it occurs only in the properties block |
| `ambiguous_text` | (`edit_note`) `old_text` occurs more than once | `"count": n`, `"lines": [line numbers of the first 20 matches]` |
| `io_error` | the file system refused (permissions, disk) | none |

Any list in an error (`candidates`, `headings`) is kept within the 8 KB
limit: entries over 512 bytes and entries past about 4 KB are left out, and
the error then carries `"cut": ["candidates"]` or `"cut": ["headings"]`.
Argument values quoted in `message` are cut to 200 bytes.

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
give `ambiguous`, except that among paths differing only in letter case the
one written exactly as given is used; none at all give `not_found` with the closest names. A
bare name (no folder, no `.md`) that several notes share is `ambiguous`
even when one of them sits at the vault root: with `index.md` and
`goals/index.md`, `"index"` is refused, `"index.md"` and `"goals/index"`
are not.
`..`, absolute paths, dot-folders and symlinks leading outside the vault
give `outside_vault`.

### Paging

No result text is larger than 8192 bytes (UTF-8). A tool whose result would
be larger returns the first part with `"next_page": string` (opaque). Call the
same tool again with **the same arguments** plus `"page": <that string>` to
get the next part. The last page has no `next_page` field. Reading every page
returns each list item exactly once, in a stable order. Tokens carry no
server state; if the vault changes between pages, items may shift.

Where a long text is paged (`read_note` content), each page carries a
`content` chunk; joining the chunks of all pages in order gives the whole text
exactly. Chunks break at line ends where possible; a line longer than one
reply is split inside the line.

### Large items

No reply is ever over 8192 bytes, even when a single item is larger. An item
or result that had to be shortened carries `"cut": [field names]`; the field
is absent when nothing was cut.

- **Text fields of list items** (`target`, `heading`, `block`, `display` of a
  link; `section` of an outline entry or of a `read_note` section result)
  over 512 bytes are cut to at most 512 bytes ending in `…`. The whole text
  is in the note: read the link's `from` note (its `line`), or read the
  section with `heading_line`.
- **Name lists** (`tags`, `aliases`) leave out entries over 200 bytes and
  entries past about 800 bytes (`find_notes`: 1,000 bytes), rather than
  shorten a name into a wrong one. `read_note` with `raw_properties: true`
  gives the properties block as written; inline tags are in the text.
- **`properties`** on `read_note` leaves out any property whose value is over
  500 bytes as JSON, whose name is over 200 bytes, or that comes after about
  2,500 bytes; the result then has `"cut": ["properties"]`. Read the whole
  block, paged, with `raw_properties: true`.

### Versions

`version` is an opaque string that changes whenever the note's file bytes
change (a hash of the whole file, properties included). `read_note` returns
it. `edit_note` and `set_property` take it and refuse with `stale` when the
file changed since.

### Sections

A section is a heading and everything under it up to the next heading of the
same or a higher level (fewer `#`). Headings inside code blocks are not
headings. A **section path** names a heading by its text, without `#`
marks: `"Plan"`. When the same text appears more than once, add parent
headings joined by `#`: `"Week 1#Plan"`. Matching is case-insensitive and
ignores surrounding spaces. A section path that matches several headings is
refused with `ambiguous_section` (by `read_note` and `edit_note` alike),
listing the full section paths of every match; pass one of them.

The **lead** is the text after the properties block and before the first
heading. Its section path is `""` (the empty string): `read_note` and
`edit_note` accept `"section": ""` for it. A note without headings is all
lead.

The **body** of a section (or of the lead) is its text below the heading
line, without the blank lines directly under the heading line and without
the blank lines at its end. Edits by section work on the body, so those
blank lines stay as they were.

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
- `ambiguous`: present only when the target matches several files by name
  or path suffix (see below): their paths, sorted. `to` is then `null`.

Resolution follows Obsidian:

1. `[[#heading]]` / `[[#^block]]` (empty target) point at the note itself.
2. A target with an extension other than `.md` (`diagram.png`) names any
   file; without an extension it names a note (`.md` is added).
3. Match the target, case-insensitively, as a vault-relative path; then
   (markdown links only) relative to the linking note's folder. Either match
   wins outright.
4. Otherwise match it as a file name or path suffix anywhere in the vault.
   One match is the link's file. **Several matches are never settled by a
   default**: the link resolves to nothing (`to: null`) and carries
   `"ambiguous": [candidate paths]`. With `index.md` and `goals/index.md`,
   `[[index]]` is the root note (an exact path), `[[goals/index]]` the other,
   and `[[plan]]` with `a/plan.md` and `b/plan.md` is ambiguous.
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
  `value`: the property's value equals it, or (for a list) contains it; a
  `value` that is itself a list must have every element contained. A
  property text `"[[Alpha]]"` equals the value `"Alpha"`.
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
  "heading_line": "integer, optional (1-based line of a heading, from the outline)",
  "full": "boolean, optional, default false",
  "raw_properties": "boolean, optional, default false",
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
  "cut": ["properties"],
  "next_page": "string, only when more pages follow"
}
```

Give at most one of `section`, `heading_line`, `full` and `raw_properties`
(`invalid_argument` otherwise).

`mode` is one of:

- `"full"`: `content` is the note's text after the properties block. Used
  when `section` is absent and the whole result fits in one 8 KB reply, when
  `full: true`, or when a long note has no headings (then paged). No
  `outline` field.
- `"outline"`: the note is too long to return whole and no `section` was
  asked. No `content` field. `outline` lists every heading in file order:
  `section` is its section path (shortest one that is unique), `level` is
  1-6, `line` is the 1-based line of the heading, `bytes` is the size of the
  section's text. When the lead holds anything but blank lines, it is the
  first entry: `{ "section": "", "level": 0, "line": <first line after the
  properties block>, "bytes": n }`, read with `"section": ""`. Ask again with `section` to read a part. A very long
  outline is paged.
- `"section"`: `section` or `heading_line` was given. `content` is that
  section's text, heading line included (for `""`, the lead as written); a field
  `"section": "<the section path>"` is added. A section larger than one
  reply is paged. `heading_line` names the section by the line of its
  heading, for a heading too long to name or a section path that repeats.
- `"properties"`: `raw_properties: true`. `content` is the properties block
  as written, without its `---` fences (`""` when there is none), paged.

`aliases` comes from the `aliases` property (list or single string; `alias`
is accepted too). `tags` are the note's tags without `#`, from body and
properties, deduplicated case-insensitively, spelled as first seen.
`properties` is the properties block as JSON (`{}` when absent); YAML dates
are strings. `cut` follows "Large items". On pages after the first, only
`path`, `version`, `mode`, `section`, `content` or `outline`, and
`next_page` are present.

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
  this note (ambiguous links resolve to no note, so they are not listed), sorted by `from` then `line`.
- Both lists are always present (empty when `direction` excludes them).
- Paging: `outgoing` items come first, then `incoming`; a page holds a
  slice of each list, and each occurrence appears on exactly one page.

### `check_vault`

Broken links in the whole vault, and nothing else.

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
  "total_broken_links": 1,
  "next_page": "string, only when more pages follow"
}
```

- A **broken link** is a link (any kind) with `to: null`; `reason` is
  `"ambiguous"` (it matches several files; the link carries `ambiguous` with
  their paths), else `"missing_note"` (a note-style target) or
  `"missing_file"` (a target with a non-`.md` extension). Links whose file exists but whose `#heading` or
  `#^block` does not are **not** reported in phase 1.
- Ambiguous links are inside `broken_links`, with `reason: "ambiguous"`
  and the `ambiguous` candidate list; there is no separate key for them.
- `broken_links` is sorted by `from` then `line`. `total_broken_links` is
  always present (`0` when there are none) and counts them across all
  pages.

## Phase 2 tools (change)

All change tools refuse `outside_vault` for every path argument (`note`,
`create_note.path`, `move_note.to`) and resolve `note` as above.

**What a write touches.** A change replaces only the bytes of its target:
the matched text, the section body, the lines of one property, or the
target part of a link.
Everything else in the file stays byte-identical, including line endings
(a CRLF file stays CRLF: new text is written with the file's own line
ending) and a missing final newline. Adding a properties block to a note
that has none is the one change that adds lines at the top. Files are
replaced whole through a temporary file and a rename, so a reader never
sees half a file; file permissions are kept. Files in dot-folders
(`.obsidian/`, `.trash/`, ...) are never changed, except that `delete_note`
adds a file to `.trash/`.

**Staleness.** `edit_note` and `set_property` require `version` (from
`read_note`) and refuse with `stale` when the file changed since; the
message says to read the note again, and the error carries the current
`version`. `move_note` and `delete_note` accept an optional `version` with
the same check. Without it they are still safe: neither rewrites the
note's own text from an earlier read. `delete_note` moves the file's bytes
as they are at that moment into `.trash/`, so nothing is lost; `move_note`
carries the note's bytes as they are, and each linking note is rewritten
from its bytes read during the same call and is checked unchanged just
before it is replaced.

### `create_note`

```json
{ "path": "Projects/Gamma.md", "content": "string, optional (body text)", "properties": { "any": "JSON, optional" } }
```

`path` is vault-relative; `.md` is added when missing; missing folders are
created. Refused with `already_exists` (with `"path"` of the existing file)
only when a file has exactly that path (case-insensitively). A name that
other notes have in other folders is allowed; the result then lists them in
`"same_name"`, so later calls should name the note by path (a bare name is
refused as `ambiguous`). Files in `.trash/` do not count.

Existing links keep their file: a link that resolved to one file before
and would, with the new note present, resolve to it instead or become
ambiguous is rewritten exactly as `move_note` rewrites links, and its note
is listed in `updated_notes`. With
`properties`, the file starts with a properties block holding them, then
`content`; `content` that itself starts with a `---` block is then refused
with `invalid_argument`. Result:
`{ "path": "Projects/Gamma.md", "name": "Gamma", "version": "...", "updated_notes": [paths], "total_updated_notes": 0, "same_name": [paths] }`
(`same_name` only when non-empty; `updated_notes` may carry `cut` as in
`move_note`).
Errors: `already_exists`, `outside_vault`, `invalid_argument`, `io_error`.

### `edit_note`

Changes one part of a note: a piece of text named by its exact words, or a
section.

```json
{
  "note": "string",
  "version": "string (from read_note)",

  "old_text": "string: the exact text to replace",
  "new_text": "string: what to put instead (may be empty)",

  "section": "string (a section path; \"\" for the lead), optional",
  "heading_line": "integer, optional instead of section",
  "content": "string",
  "mode": "\"replace\" | \"append\" | \"prepend\", optional, default \"replace\""
}
```

Give exactly one way, else `invalid_argument`:

- **By text:** `old_text` and `new_text`, no `content` or `mode`.
  `old_text` (not empty) must occur exactly once in the note's text after
  the properties block; heading lines and the lead are part of that text.
  Matching is exact, except that line endings compare equal (`\r\n` in
  the file matches `\n` sent by the agent). Only the matched bytes are
  replaced; `new_text` is written in the file's line ending. With
  `section` or `heading_line` as well, only that section (heading line
  included, or the lead for `""`) is searched, for text that repeats
  elsewhere. Refused with:
  - `text_not_found` when it does not occur. When it occurs only inside
    the properties block, the error adds `"in_properties": true` and the
    message points to `set_property`; the properties block is never edited
    by text.
  - `ambiguous_text` when it occurs more than once, with `"count": n` and
    `"lines": [line numbers of the first 20 matches]`; add more
    surrounding text, or a `section`, to make it unique.
- **By section:** `section` or `heading_line` with `content` (and
  optionally `mode`), no `old_text` or `new_text`. The section is its
  heading and everything up to the next heading of the same or a higher
  level, so subsections are part of it. `replace` replaces the section's
  body (see "Sections": the heading line, the blank lines under it and
  the blank lines at the end stay); `append` adds `content` at the end of
  the body (after subsections, before the trailing blank lines);
  `prepend` adds it at the start of the body (after the blank lines under
  the heading). `""` names the lead, with no heading line. `content`'s
  line breaks are written in the file's line ending, and a line break is
  added at its end when it has none and text follows it. A body that is
  only blank lines: `content` goes after them.

A note whose file is not valid UTF-8 is refused with `invalid_argument`
and nothing is written.
Result: `{ "path": "...", "version": "..." }`.
Errors: `stale` (with current `version`), `text_not_found`,
`ambiguous_text`, `section_not_found`, `ambiguous_section`, `not_found`,
`ambiguous`, `invalid_argument`.

### `set_property`

```json
{ "note": "string", "name": "string", "value": "any JSON; null removes the property", "version": "string" }
```

`name` matches an existing property case-insensitively, and the key keeps
its spelling in the file. Only that property's lines (its key line and
the indented or `- ` lines under it) are rewritten; other properties, their
order, comments and quoting stay byte-identical. A new property is added as
the last line of the block; the block is created at the top of the file
when absent. Values are written as YAML: text plain when YAML allows it,
else double-quoted; lists one item per line as `  - item`; objects as YAML
mappings. Refused with `invalid_properties` when the note's properties
block is not valid YAML (nothing is rewritten; fix the block by hand), and
with `invalid_argument` when the file is not valid UTF-8.
Result: `{ "path": "...", "version": "...", "properties": { ... } }`
(`properties` follows "Large items" as in `read_note`, with `cut`).
Errors: `stale`, `invalid_properties`, `not_found`, `ambiguous`,
`invalid_argument`.

### `move_note`

```json
{ "note": "string", "to": "string (new vault-relative path)", "version": "string, optional" }
```

Moves or renames the note. `to` is a note path (`.md` added when missing);
when `to` names an existing folder, the note keeps its file name inside it.
Missing folders are created. Refused with `already_exists` when another file
has the path `to` (case-insensitively; changing only the case of the note's
own path is allowed).

Every link in the vault that resolved to one file before keeps pointing to
that file:
links to the moved note, links from it (relative Markdown links included),
and links to other files whose name the moved note could now capture. A
link whose text still resolves to the same file is left untouched. Any
other is rewritten in its target part only — `#heading`, `#^block`,
`|display`, `<...>`, the percent-encoding style and a written `.md` stay —
to the shortest target that resolves to that file only: the bare name,
then the path with as few parent folders as needed. A link that the change
would make ambiguous is rewritten the same way; since a shared name is
ambiguous, links to a note whose name is shared get a path. Links in code
and files in dot-folders are never touched. Links that were broken or
ambiguous before stay as written. Before anything is written, every rewritten file is parsed again
and each of its links checked; if any would not point where it did, the move
is refused with `invalid_argument` and nothing is written. Reference-style
Markdown links (`[t][ref]`) cannot be rewritten in place; a move that would
need to rewrite one is refused the same way.

Result: `{ "from": "old/path.md", "path": "new/path.md", "version": "...", "updated_notes": ["paths, after the move, of other notes whose links were rewritten"], "total_updated_notes": 1, "same_name": [paths] }`.
`same_name` lists the other notes with the note's new name, only when there
are any.
`"total_updated_notes": n` counts them; when the list does not fit in one
reply it holds the first paths that fit and the result carries
`"cut": ["updated_notes"]`. The moved note itself is not listed; its `version`
changes only if its own links were rewritten. Linking notes are replaced
one after another, then the note is moved; this is not atomic across files.
If a step fails part way, the linking notes already rewritten are put back,
and the error message says so or names any that could not be.
Errors: `already_exists`, `outside_vault`, `not_found`, `ambiguous`,
`stale`, `invalid_argument`, `io_error`.

### `delete_note`

```json
{ "note": "string", "version": "string, optional" }
```

Moves the note into the vault's `.trash/` folder (Obsidian's "move to
Obsidian trash"), so it can be restored by hand. The file keeps its name
there; when `.trash/` already holds that name, a number is added
(`Name 1.md`, `Name 2.md`, ...), so no file is ever overwritten. Result:
`{ "path": "deleted/path.md", "deleted": true, "trash_path": ".trash/Name.md", "now_broken": [link objects in other notes that resolved to it] }`.
`"total_now_broken": n` counts them all. A change is not paged: when the
list does not fit in one reply, it holds the first links that fit and the
result carries `"cut": ["now_broken"]`; `check_vault` lists them all, since
they are now broken links.
Errors: `not_found`, `ambiguous`, `stale`, `outside_vault`, `io_error`.
