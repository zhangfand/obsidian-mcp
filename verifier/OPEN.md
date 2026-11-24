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
2. **Bare links and lookups to a name two notes share.** Decided by the
   owner: a shared name is never settled by a default. A `note` argument
   gives `ambiguous` with every candidate path (rule 4: `Meeting`,
   `Meeting.md`, `meeting`). A link in text resolves to nothing, carries
   `ambiguous` in `get_links` (rule 2, `Inbox/Shared Names.md`), and is
   reported by `check_vault` with reason `ambiguous` (rule 3, 4 seeded
   links next to the 10 broken ones). An exact path is not ambiguous.
   Still open: a move or delete that *ends* an ambiguity makes a seeded
   ambiguous link start resolving. Is that "pointing to the same note as
   before"? Not seeded: rule 1 moves `Work/Meeting.md` to
   `Work/Team/Meeting.md`, so the name stays shared.
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
17. **Creates and moves onto a shared name.** Decided by the owner: only an
    exact taken path is refused. A create or move onto a name other notes
    have succeeds and lists them in `same_name` (rule 8: three creates, one
    of them on a name held only by a note in `.trash/`). Links the new
    note would capture or make ambiguous must be rewritten to keep their
    note (rule 1: two creates and two moves onto shared names). The vault
    has no reference-style links (`[t][ref]`), so the `move_note` refusal for
    them is never triggered.
18. **Rule 6 under rule set v3.** By-text edits are scored on bytes: outside
    the one match nothing may change, and the match must become `new_text`,
    with line endings compared as equal. A refusal is scored on its error
    kind (`ambiguous_text` or `text_not_found`) and an untouched file. For
    text that exists only in the properties block, the verifier expects
    `text_not_found` and does not score `in_properties`. Section edits may
    change only the body: the heading line, the blank lines directly under
    it and the blank lines at its end must stay. Section `""` is the lead.
