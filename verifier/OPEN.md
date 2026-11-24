# Open cases

Cases where help.obsidian.md is silent, where TOOLS.md and the rule set differ,
or where the verifier had to choose. None of the cases in "Not scored" is in
the seeded vault or the expected data. Each waits for the planner.

## Not scored until decided

1. **Bare alias links.** TOOLS.md: `[[Some Alias]]` is unresolved unless a file
   has that name. help.obsidian.md (Aliases) only says Obsidian writes alias
   links as `[[Real Name|Alias]]`; it does not say what a bare `[[Alias]]`
   does. I agree with TOOLS.md, but the docs do not state it. The vault
   holds only the `[[Real Name|Alias]]` form, which is what rule 2 scores as
   "links by alias".
2. **Bare links and lookups to a name two notes share.** TOOLS.md picks the
   note in the linking note's folder, then the shortest path, then the
   alphabetically first, and answers `ambiguous` to the `note` argument
   `"Meeting"`. The docs do not say how Obsidian picks. The vault reaches
   `Archive/Meeting.md` and `Work/Meeting.md` only by path-qualified links
   and lookups.
3. **Case-insensitive lookups and links** (`"apollo"`, `[[apollo]]`). The docs
   do not say. Not scored.
4. **Links in indented code blocks.** TOOLS.md says they are not links. The
   docs do not say. Not seeded (only fenced and inline code).
5. **Links to a missing heading or block** (`[[Apollo#Nope]]`). TOOLS.md does
   not report them in phase 1. The docs do not say. Not seeded.
6. **Text search beyond note bodies.** TOOLS.md matches names and the
   properties block too, and code. Obsidian's search has its own operators.
   Scored text tokens appear only in note bodies, outside code and outside
   names, so both readings give the same answer.
7. **Numbers against their text form** (`"3"` matches `3`), and YAML dates
   as strings. TOOLS.md behaviour, not in the docs. Not scored.
8. **Unicode normalisation of file names** (NFC against NFD, as macOS
   writes). Not scored; all names in the vault are NFC.

## The rule set and TOOLS.md differ

9. **What an unlinked note is.** Decided in rule set v2: `check_vault`
   reports broken links only. Rule 3 no longer scores unlinked notes, and it
   fails when the result still carries `unlinked_notes`. The five notes that
   nothing links to stay in the vault, because rules 2 and 10 use them.
17. **`create_note` refuses a name used in another folder.** TOOLS.md (since
    08f5d49) refuses `create_note` with `already_exists` when any note in
    the vault already has that name, even in another folder. Obsidian allows
    two notes with the same name in different folders (the vault itself
    holds `Archive/Meeting.md` and `Work/Meeting.md`). Rule 8 says only "a
    create on a taken name is refused". The verifier does not score either
    reading: its refused creates are all on the same path, and its successful
    create (`Inbox/New Idea`) uses a name no note has. No current rule 1, 6,
    7 or 8 case conflicts with the 08f5d49 changes. The vault has no
    reference-style links (`[t][ref]`), so the new `move_note` refusal for
    them is never triggered.

## Choices the verifier made (tell me if any is wrong)

10. **`target` of markdown links.** TOOLS.md says "as written". The verifier
    compares markdown targets after dropping `<...>` and decoding `%20`, so
    `Areas/Reading%20List.md` and `Areas/Reading List.md` both pass.
11. **Rule 1 strictness.** Only the target part of a link may change: a
    rename may not add display text (`[[Old]]` to `[[New|Old]]` fails), and
    files in `.obsidian/` and `.trash/` must stay byte-identical. Relative
    markdown links inside a moved note must still reach the same note.
12. **Rule 4 misses.** The expected name must be among `candidates`; it need
    not be first.
13. **Rule 7 wording.** "The reply says to read again" is checked as the word
    `read` in the error's `message`.
14. **Rule 9 in phase 1.** Four of the 19 cases call `create_note` and
    `move_note`. A phase-1 server without those tools refuses them as unknown
    tools, which counts as refused. The 15 read-tool cases alone meet the
    count of 10.
15. **`.git` in the vault.** Git cannot commit a `.git` folder inside
    `verifier/vault/`, so `verifier/run` creates `.git/HEAD`, `.git/config`
    and `.git/secret.md` in the `--vault` copy before any check. It refuses
    to run unless the copy is otherwise byte-identical to `verifier/vault/`
    with both symlinks intact.
16. **Handshake.** TOOLS.md points to `server/AGENTS.md` for the exact
    handshake, which the verifier builder may not read. The verifier sends
    `initialize`, then `notifications/initialized`, then stateless
    `tools/call` POSTs, and keeps an `Mcp-Session-Id` if one is returned.
    Not yet tried against the real server.
