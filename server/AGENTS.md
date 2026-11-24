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
- Owns the note `version` (a hash of the file bytes) that the phase 2
  change tools will check for `stale`.
- Owns the wording of errors: every error says what to try next.
- Decides nothing about Obsidian.
- The five change tools are not built (phase 2). Calling one gives
  `invalid_argument` saying it is not available yet; they are not listed.

### `transport` — the binary `obsidian-mcp`

- Carries MCP Streamable HTTP at `/mcp` to `tools`. Only this crate depends
  on `rmcp` (pinned `=3.11.0`).
- Stateless: each POST is answered alone with `application/json`; no
  session id is issued or needed.
- Decides nothing about tools or the vault.

## Running

```sh
cargo build --release
target/release/obsidian-mcp --vault <dir> --listen 127.0.0.1:<port> [--allowed-host <host>]...
```

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

Under the 300 ms bar, so there is no cache.

## Open questions

Choices made where Obsidian's help is silent or unclear. Each is
implemented as stated.

1. **Links by alias.** `[[My Alias]]` alone does not resolve, as in
   Obsidian; Obsidian writes alias links as `[[Real Name|My Alias]]`.
   Such a link is reported broken by `check_vault`. `note` arguments *do*
   accept aliases.
2. **Name collisions.** When several files share a link's name: the one
   in the linking note's folder, then the shortest path, then the
   alphabetically first.
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

## Known limits

- Paths are never shortened. A note path near the system limit of 4,096
  bytes could leave a reply with no room; the reply is then `too_large`
  rather than over 8 KB.
- The five change tools (phase 2) are specified in `TOOLS.md` but not built.
