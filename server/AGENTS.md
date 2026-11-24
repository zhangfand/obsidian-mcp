# obsidian-mcp server

A Rust MCP server that lets agents work with one Obsidian vault in
Obsidian's terms. The tool contract is `TOOLS.md`; this file says what each
crate promises and records the choices made where Obsidian's docs are silent.

## Crates and what each promises

One cargo workspace, three crates. Dependencies point one way:
`transport` → `tools` → `vault`.

### `vault` — what Obsidian means

- Reads the vault from disk on every call (`Vault::snapshot`). There is no
  stored index and no cache; a 5,000-note vault reads in about 80 ms (see
  "Measured").
- Inside one file: the properties block, headings and sections, wikilinks,
  embeds, Markdown links, inline and property tags, aliases and block ids,
  and that text in code (fenced, indented, inline) and math is none of these.
- Across files: how a path, name or alias reaches a note
  (`Snapshot::lookup`), where a link leads when names collide
  (`Snapshot::resolve`), what links into a note, and which links are
  broken. Names, paths, tags, aliases and headings compare
  case-insensitively.
- Never exposes a dot-folder (`.obsidian`, `.git`, `.trash`, ...), never
  follows a symlinked folder, never reads a symlinked file that leaves the
  vault, and refuses `..` and absolute paths (`VaultPath`, `PathError`).
  `Snapshot::safe_path` is the one gate for every path argument: it also
  refuses a path when any existing part of it is a symlink (file or folder)
  leading outside the vault.
- Each vocabulary term is one type: `Note`, `Link`, `LinkKind`, `Heading`,
  `Section`, `Property`, `PropertyValue`, `Properties`, `Tag`, `Alias`,
  `BlockId`, `VaultPath`.
- Knows which bytes are a link target (`Link::span`), a section
  (`Heading::end`, `Section::range`) and a property (`Note::property_span`),
  and changes a note only by splicing those bytes: `Note::edit_section`,
  `Note::set_property` (refused on invalid YAML, and parsed again to check
  that exactly the intended properties come back), `new_note_text`.
- Plans moves and creates so that every link that resolved to one file
  still does (`Snapshot::plan_move`, `Snapshot::plan_create`): only links
  that would point elsewhere or become ambiguous are rewritten, in their
  target only, to the shortest form that reaches the same file; the
  rewritten files are parsed again and every link checked before anything
  is written.
- Writes files whole through a temporary file outside the vault and a
  rename, keeping permissions; never overwrites on create, move or trash
  (hard link, then unlink); checks a file still holds the bytes a change was
  planned from (`Vault::replace_file`, `create_file`, `move_file`,
  `move_to_trash`).
- Knows nothing about agents, MCP, JSON, paging or byte limits.
- Wikilinks come from its own scanner, not pulldown-cmark: pulldown-cmark
  0.13 reads `[[n\|t]]` in a table as the target `n\`, and links a
  `[[...]]` that spans lines, which Obsidian does not. pulldown-cmark handles
  `[[n#h]]`, `[[n#^b]]`, `[[n|t]]` and `![[n]]` correctly and is still used
  for code, math, headings and Markdown links.

### `tools` — how an agent talks to the vault

- Each tool is a plain function with its own argument and result types:
  `find_notes`, `read_note`, `get_links`, `check_vault`. `Tools::call` runs
  one by name on JSON arguments and gives back one JSON text and an error
  flag. `specs()` lists the tools with their input schemas.
- No reply text is over 8192 bytes, with no exception. Longer results are
  cut into pages with an opaque `next_page` token that carries the tool, a
  hash of the other arguments and an offset; no state is kept between
  calls. A single item too large for a reply has its long fields cut and
  marked with `cut` (TOOLS.md, "Large items"); `Tools::call` replaces any
  reply that would still be over the limit with a `too_large` error.
- Owns the note `version` (a hash of the file bytes) and the `stale`
  check: `edit_section` and `set_property` require it, `move_note` and
  `delete_note` take it optionally.
- Owns the trash decision: the note keeps its file name in `.trash/`, with
  ` 1`, ` 2`, ... added when the name is taken.
- Runs one change at a time per process (a lock), each planned on a fresh
  read of the vault.
- Owns the wording of errors: every error says what to try next.
- Decides nothing about Obsidian.
- The change tools are `create_note`, `edit_section`, `set_property`,
  `move_note`, `delete_note`.

### `transport` — the binary `obsidian-mcp`

- Carries MCP Streamable HTTP at `/mcp` to `tools`. Only this crate depends
  on `rmcp` (pinned `=3.11.0`).
- Stateless: each POST is answered alone with `application/json`; no
  session id is issued or needed.
- Decides nothing about tools or the vault.

## Running

```sh
cargo build --release
target/release/obsidian-mcp --vault <dir> --listen 127.0.0.1:<port> [--allowed-host <host>]... [--tmp-dir <dir>]
```

`--tmp-dir` is where new file contents are written before being renamed
into place; the default is a hidden sibling of the vault folder
(`<parent>/.<vault name>.obsidian-mcp-tmp`), and it must be outside the
vault. At start the server tries a rename from it into the vault: if the
two are on different filesystems it says so on stderr and writes temporary
files as hidden files next to each changed note instead (Obsidian ignores
dot-files).

rmcp checks the `Host` header against an allow list to stop DNS
rebinding. Loopback (`localhost`, `127.0.0.1`, `::1`) is always allowed;
behind a tunnel, pass the public hostname with `--allowed-host`, or every
request gets `403 Forbidden: Host header is not allowed`.

## Talking to the server

Verified with curl against the built binary:

```sh
curl -s http://127.0.0.1:<port>/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_note","arguments":{"note":"Alpha"}}}'
```

- No `initialize` call is needed. `tools/list` and `tools/call` work as the
  first and only request. `initialize` is still answered for clients that
  send it, and `notifications/initialized` gets `202`.
- Both headers above are required. An `Accept` without
  `text/event-stream` gets `406`.
- `MCP-Protocol-Version` may be left out (treated as `2025-03-26`) or set
  to any version up to `2025-11-25` with nothing else needed.
- With `MCP-Protocol-Version: 2026-07-28` rmcp also requires the
  `Mcp-Method` header (and `Mcp-Name` for `tools/call`) and, in
  `params._meta`, `io.modelcontextprotocol/protocolVersion`,
  `io.modelcontextprotocol/clientInfo` and
  `io.modelcontextprotocol/clientCapabilities`; without them the reply is
  `400`. The simplest client sends no version header.
- The reply is one JSON-RPC response. The tool result is
  `result.content[0].text` (a JSON object as a string) and
  `result.isError`.

## Commands

```sh
cargo test                                          # all unit and HTTP tests
cargo clippy --all-targets
cargo run --release -p tools --example full_read    # time a 5,000-note vault
```

## Measured

`cargo run --release -p tools --example full_read` on the x86_64 build
machine (4 cores, shared with other builds), 5,000 generated notes of about
2.5 KB each in 50 folders, warm page cache, best of 5:

| what | time |
|---|---|
| full read: list, read and parse every note | 80 ms |
| `check_vault` (full read + resolve every link) | 161 ms |
| `get_links` (full read + incoming links) | 126 ms |
| `find_notes` by text | 115 ms |
| `move_note` of a note with links, plan and write (measured later, same machine) | 254 ms |

Under the 300 ms bar, so there is no cache.

## Open questions

Choices made where Obsidian's help is silent or unclear. Each is
implemented as stated.

1. **Links by alias.** `[[My Alias]]` alone does not resolve, as in
   Obsidian; Obsidian writes alias links as `[[Real Name|My Alias]]`.
   Such a link is reported broken by `check_vault`. `note` arguments *do*
   accept aliases.
2. **Name collisions (decided by the owner).** A link whose name or path
   suffix matches several files resolves to nothing and is reported as
   ambiguous with the candidates; an exact vault path (or, for Markdown
   links, an exact path relative to the linking note) wins outright.
   Obsidian itself would pick one; we never pick silently.
3. **Missing heading or block.** `[[Note#No such heading]]` where `Note`
   exists is not reported as broken.
4. **Links in properties.** Wikilinks in the properties block count as
   links (Obsidian 1.4+ does so for property values). The block is scanned
   line by line, so a `[[...]]` in a YAML comment would also count.
   Markdown links in properties do not count.
5. **Comments and HTML.** Links and tags inside `%% ... %%` comments and
   HTML blocks are counted; only code and math hide them.
6. **Inline tags** start at a line start or after whitespace (`(#tag)` is
   not a tag), use letters, digits, `_`, `-`, `/`, need one non-digit, and
   lose a trailing `/`.
7. **Property strings.** `tags: "a, b c"` is split on commas and spaces into
   three tags; `aliases: "a, b"` is one alias. `tag` and `alias` are read
   too.
8. **Headings inside quotes, callouts and lists** are not headings: they
   neither appear in the outline nor end a section.
9. **Section names** match heading text case-insensitively with runs of
    spaces collapsed; parents in `A#B` may skip levels. Obsidian strips
    characters such as `:` `|` `^` from heading links; we compare the text
    as written.
10. **Duplicate full section paths** (two `## Plan` under the same `# Week`)
    cannot be told apart by `section`, which answers `ambiguous_section`;
    `read_note` with `heading_line` reads either one.
11. **Markdown link targets** are percent-decoded; a target with no
    extension is a note; relative to the linking note's folder is tried
    after the vault root.
12. **Invalid YAML** in the properties block gives `{}` properties, no
    property tags or aliases; wikilinks in it still count.
13. **Symlinked files** that point inside the vault are notes in their own
    right (the target may then appear twice); symlinked folders are never
    followed.
14. **Non-UTF-8 files** are read with invalid bytes replaced.
15. **`find_notes` property `value`** given as a list means the property
    must contain every element. A property text `"[[Alpha]]"` matches the
    value `"Alpha"`.

16. **Broken links and moves.** A link that was broken before a move or
    create stays as written, even when the new name would now make it
    resolve.
17. **Comments on a property's own lines** (`status: draft # note`) are
    part of that property and are dropped when it is rewritten; comments
    on other lines stay.
18. **Folders** emptied by a move are left in place, as Obsidian does.
19. **Reference-style Markdown links** (`[t][ref]` with `[ref]: Note.md`)
    resolve, but a move or create that would need to rewrite one is
    refused, since the target is written in the definition.

## Known limits

- Paths are never shortened. A note path near the system limit of 4,096
  bytes could leave a reply with no room; the reply is then `too_large`
  rather than over 8 KB.
- A move or create that rewrites links in several notes is not atomic
  across files (accepted by the owner): linking notes are replaced one by
  one, then the note is moved. Each file is checked unchanged first; a
  failure part way says how many notes were already rewritten.
- Changes from outside this process (Obsidian itself) can race with a
  write between the final byte check and the rename; the window is one
  read-compare-rename.
- Editing a note that is a symlink replaces the symlink with a regular
  file.
