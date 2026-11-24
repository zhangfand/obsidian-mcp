//! The change tools, by the owner's rules: links keep their file across
//! moves, edits change only their target, stale edits are refused, deletes
//! go to the trash, every path argument stays inside the vault, and no reply
//! is over 8 KB.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tools::{REPLY_LIMIT, Tools};
use vault::{Note, Vault, VaultPath};

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn call(t: &Tools, name: &str, args: Value) -> (Value, bool) {
    let r = t.call(name, Some(args.clone()));
    assert!(
        r.text.len() <= REPLY_LIMIT,
        "{name} {args} reply is {} bytes",
        r.text.len()
    );
    (serde_json::from_str(&r.text).unwrap(), r.is_error)
}

fn ok(t: &Tools, name: &str, args: Value) -> Value {
    let (v, err) = call(t, name, args.clone());
    assert!(!err, "{name} {args}: unexpected error: {v}");
    v
}

fn err(t: &Tools, name: &str, args: Value) -> Value {
    let (v, err) = call(t, name, args.clone());
    assert!(err, "{name} {args}: expected an error: {v}");
    v
}

fn tools_for(dir: &Path) -> Tools {
    let tmp = dir.parent().unwrap().join(format!(
        "{}-tmp",
        dir.file_name().unwrap().to_string_lossy()
    ));
    Tools::new(Vault::open(dir).unwrap().with_tmp_dir(tmp).unwrap())
}

/// Every file under `root`, hidden ones included, with its bytes.
fn all_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn version(t: &Tools, note: &str) -> String {
    ok(t, "read_note", json!({"note": note}))["version"]
        .as_str()
        .unwrap()
        .to_string()
}

// ---- Rule 1: moves keep every link on its file ----

fn link_vault(dir: &Path) {
    write(
        dir,
        "Hub.md",
        "---\nrelated: \"[[Alpha]]\"\nlist:\n  - \"[[Deep/Gamma Note#Top]]\"\n---\n# Hub\r\n\
         [[Alpha]] [[Alpha|the alpha]] [[Alpha#Plan]] [[Alpha#^blk]] ![[Alpha]] [[Alpha.md]]\n\
         [t](Projects/Alpha.md) [u](<Projects/Alpha.md#Plan>) [w](Projects/Alpha.md \"title\") [[Projects/Alpha#Plan|p]]\n\
         [[Beta]] [p](Deep/Gamma%20Note.md) ![[pic.png]] ![i](assets/pic.png) [[Leaf]]\n\
         `[[Alpha]]` and `[t](Projects/Alpha.md)` in code\n\
         ```\n[[Alpha]] [[Beta]]\n```\n\
         | a | b |\n|---|---|\n| [[Alpha\\|tab]] | [[Beta]] |\n\
         [[Nowhere]] stays broken. No final newline: [[Beta]]",
    );
    write(
        dir,
        "Projects/Alpha.md",
        "# Top\n## Plan\nsee [[Beta]] and [r](../Hub.md) and [s](Sub/Leaf.md) ^blk\n",
    );
    write(dir, "Beta.md", "[[Alpha]] [[Hub]] [back](Hub.md)\n");
    write(
        dir,
        "Deep/Gamma Note.md",
        "# Top\n[[Alpha#Plan|x]] [m](../Beta.md) [[Projects/Sub/Leaf]]\n",
    );
    write(dir, "Projects/Sub/Leaf.md", "[[Alpha]] [up](../Alpha.md)\n");
    write(dir, "assets/pic.png", "png");
    write(dir, ".obsidian/workspace.json", "{\"last\": \"[[Alpha]]\"}");
    write(dir, ".trash/Old.md", "[[Alpha]] [[Beta]]\n");
}

/// Where each link of each note resolves, by note path.
fn resolutions(root: &Path) -> BTreeMap<VaultPath, Vec<Option<VaultPath>>> {
    let snap = Vault::open(root).unwrap().snapshot().unwrap();
    snap.notes()
        .iter()
        .map(|n| {
            (
                n.path.clone(),
                n.links
                    .iter()
                    .map(|l| snap.resolve(l, &n.path).cloned())
                    .collect(),
            )
        })
        .collect()
}

/// True when `new` is `old` with only link target spans replaced.
fn only_targets_changed(path: &VaultPath, old: &str, new: &str) -> Result<(), String> {
    let a = Note::parse(path.clone(), old.to_string());
    let b = Note::parse(path.clone(), new.to_string());
    if a.links.len() != b.links.len() {
        return Err(format!(
            "{path}: link count {} -> {}",
            a.links.len(),
            b.links.len()
        ));
    }
    let (mut pa, mut pb) = (0, 0);
    for (la, lb) in a.links.iter().zip(&b.links) {
        let (Some(sa), Some(sb)) = (la.span.clone(), lb.span.clone()) else {
            continue;
        };
        if old[pa..sa.start] != new[pb..sb.start] {
            return Err(format!("{path}: text before line {} changed", la.line));
        }
        if (
            la.heading.clone(),
            la.block.clone(),
            la.display.clone(),
            la.kind,
        ) != (
            lb.heading.clone(),
            lb.block.clone(),
            lb.display.clone(),
            lb.kind,
        ) {
            return Err(format!(
                "{path}: more than the target of line {} changed",
                la.line
            ));
        }
        pa = sa.end;
        pb = sb.end;
    }
    if old[pa..] != new[pb..] {
        return Err(format!("{path}: text after the last link changed"));
    }
    Ok(())
}

#[test]
fn twenty_moves_keep_every_link() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("vault");
    fs::create_dir(&dir).unwrap();
    link_vault(&dir);
    let t = tools_for(&dir);

    // Each note by its first path, and where it is now.
    let mut at: BTreeMap<&str, String> = [
        ("alpha", "Projects/Alpha.md"),
        ("beta", "Beta.md"),
        ("hub", "Hub.md"),
        ("gamma", "Deep/Gamma Note.md"),
        ("leaf", "Projects/Sub/Leaf.md"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect();
    let moves = [
        ("alpha", "Archive/Alpha"),
        ("beta", "Other/Alpha"), // shares Alpha's name: bare [[Alpha]] would be ambiguous
        ("hub", "Hub Renamed"),
        ("gamma", "Gamma Note"),
        ("leaf", "Leaf"),
        ("alpha", "Projects/Alpha"),
        ("beta", "Beta"),
        ("hub", "Deep/Hub"),
        ("gamma", "Deep/Gamma Note"),
        ("leaf", "Projects/Sub/Leaf"),
        ("alpha", "Alpha Prime"),
        ("beta", "Projects/Beta"),
        ("hub", "Hub"),
        ("alpha", "Projects/Sub/Alpha"),
        ("gamma", "Projects/Gamma Note"),
        ("leaf", "Deep/Leaf"),
        ("beta", "Deep/Alpha"), // shares the name again
        ("hub", "Projects/Hub"),
        ("alpha", "Archive/"), // into an existing folder, keeping the file name
        ("gamma", "Gamma"),
    ];
    let hidden_before: Vec<(String, Vec<u8>)> = all_files(&dir)
        .into_iter()
        .filter(|(p, _)| p.starts_with('.'))
        .collect();

    for (i, (who, to)) in moves.iter().enumerate() {
        let from = at[who].clone();
        let files_before = all_files(&dir);
        let before = resolutions(&dir);
        let v = ok(&t, "move_note", json!({"note": from, "to": to}));
        let new_path = v["path"].as_str().unwrap().to_string();
        at.insert(who, new_path.clone());
        let map = |p: &VaultPath| {
            if p.as_str() == from {
                VaultPath::parse(&new_path).unwrap()
            } else {
                p.clone()
            }
        };

        let after = resolutions(&dir);
        for (path, targets) in &before {
            let now = &after[&map(path)];
            for (k, want) in targets.iter().enumerate() {
                if let Some(want) = want {
                    assert_eq!(
                        now[k].as_ref(),
                        Some(&map(want)),
                        "move {i} ({from} -> {to}): {path} link {k}"
                    );
                }
            }
        }
        let files_after = all_files(&dir);
        let mut updated: Vec<String> = Vec::new();
        for (p, old) in &files_before {
            if !p.ends_with(".md") || p.starts_with('.') {
                assert_eq!(files_after.get(p), Some(old), "move {i}: {p} changed");
                continue;
            }
            let vp = VaultPath::parse(p).unwrap();
            let new = files_after
                .get(map(&vp).as_str())
                .unwrap_or_else(|| panic!("move {i}: {p} vanished"));
            if new != old {
                only_targets_changed(
                    &vp,
                    std::str::from_utf8(old).unwrap(),
                    std::str::from_utf8(new).unwrap(),
                )
                .unwrap_or_else(|e| panic!("move {i}: {e}"));
                if p != &from {
                    updated.push(p.clone());
                }
            }
        }
        let mut reported: Vec<String> = v["updated_notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        reported.sort();
        assert_eq!(reported, updated, "move {i}: updated_notes");
        assert_eq!(v["total_updated_notes"], updated.len());
        assert!(!files_after.contains_key(&from) || from.to_lowercase() == new_path.to_lowercase());
    }

    let hidden_after: Vec<(String, Vec<u8>)> = all_files(&dir)
        .into_iter()
        .filter(|(p, _)| p.starts_with('.'))
        .collect();
    assert_eq!(
        hidden_before, hidden_after,
        ".obsidian and .trash are untouched"
    );
    // Code still holds the words as written.
    let hub = fs::read_to_string(dir.join(&at["hub"])).unwrap();
    assert!(hub.contains("`[[Alpha]]` and `[t](Projects/Alpha.md)` in code"));
    assert!(hub.contains("```\n[[Alpha]] [[Beta]]\n```"));
    assert!(hub.contains("[[Nowhere]] stays broken"));
    assert!(!hub.ends_with('\n'));
    assert!(hub.contains("# Hub\r\n"));
}

#[test]
fn move_rewrites_only_what_needs_it() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("v");
    fs::create_dir(&dir).unwrap();
    link_vault(&dir);
    let t = tools_for(&dir);
    let hub_before = fs::read_to_string(dir.join("Hub.md")).unwrap();
    let v = ok(
        &t,
        "move_note",
        json!({"note": "Projects/Alpha", "to": "Archive/Alpha"}),
    );
    let hub = fs::read_to_string(dir.join("Hub.md")).unwrap();
    // Bare-name links still reach the moved note and are left as written.
    assert!(hub.contains(
        "[[Alpha]] [[Alpha|the alpha]] [[Alpha#Plan]] [[Alpha#^blk]] ![[Alpha]] [[Alpha.md]]"
    ));
    // Path links no longer reach it: rewritten to the shortest form, style kept.
    assert!(
        hub.contains("[t](Alpha.md) [u](<Alpha.md#Plan>) [w](Alpha.md \"title\") [[Alpha#Plan|p]]"),
        "{hub}"
    );
    assert_ne!(hub, hub_before);
    assert!(v.get("same_name").is_none());

    // A second note named Alpha: bare links would be ambiguous, so they get a path.
    let v = ok(
        &t,
        "move_note",
        json!({"note": "Beta", "to": "Other/Alpha"}),
    );
    assert_eq!(v["same_name"], json!(["Archive/Alpha.md"]));
    let hub = fs::read_to_string(dir.join("Hub.md")).unwrap();
    assert!(
        hub.contains("[[Archive/Alpha]] [[Archive/Alpha|the alpha]] [[Archive/Alpha#Plan]]"),
        "{hub}"
    );
    assert!(
        hub.contains("[[Other/Alpha]] [p](Deep/Gamma%20Note.md)"),
        "{hub}"
    );
    assert!(
        hub.contains("| [[Archive/Alpha\\|tab]] | [[Other/Alpha]] |"),
        "{hub}"
    );
    assert!(
        hub.starts_with("---\nrelated: \"[[Archive/Alpha]]\"\n"),
        "{hub}"
    );
    // The bare name is now ambiguous for agents too.
    assert_eq!(
        err(&t, "read_note", json!({"note": "Alpha"}))["error"],
        "ambiguous"
    );
}

#[test]
fn move_keeps_relative_links_of_the_moved_note() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "A/Note.md",
        "[x](../B/Target.md) [y](Here.md) [z](<../B/Other Target.md>)\n",
    );
    write(dir.path(), "A/Here.md", "here\n");
    write(dir.path(), "C/Here.md", "another here\n");
    write(dir.path(), "B/Target.md", "t\n");
    write(dir.path(), "B/Other Target.md", "t\n");
    let t = tools_for(dir.path());
    ok(&t, "move_note", json!({"note": "A/Note", "to": "C/D/Note"}));
    let text = fs::read_to_string(dir.path().join("C/D/Note.md")).unwrap();
    assert_eq!(
        text,
        "[x](Target.md) [y](A/Here.md) [z](<Other Target.md>)\n"
    );
}

#[test]
fn move_refusals() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "A.md", "[[B]]\n");
    write(dir.path(), "B.md", "b\n");
    write(dir.path(), "R.md", "[ref][1]\n\n[1]: B.md\n");
    let t = tools_for(dir.path());
    let e = err(&t, "move_note", json!({"note": "A", "to": "b"}));
    assert_eq!(e["error"], "already_exists");
    assert_eq!(e["path"], "B.md");
    let e = err(
        &t,
        "move_note",
        json!({"note": "B", "to": "Elsewhere/B", "version": "0000"}),
    );
    assert_eq!(e["error"], "stale");
    // A reference-style link cannot be rewritten in place.
    let e = err(
        &t,
        "move_note",
        json!({"note": "B", "to": "Elsewhere/Sub/B2"}),
    );
    assert_eq!(e["error"], "invalid_argument", "{e}");
    assert!(dir.path().join("B.md").exists());
    // Changing only the letter case is allowed.
    ok(&t, "move_note", json!({"note": "A", "to": "a"}));
    assert!(dir.path().join("a.md").exists());
}

// ---- Rule 6: edits change only their target ----

/// Asserts `new` equals `old` outside `old[range]`.
fn only_inside(old: &str, new: &str, range: std::ops::Range<usize>) {
    assert_eq!(
        &new[..range.start],
        &old[..range.start],
        "text before the target changed"
    );
    let tail = &old[range.end..];
    assert!(
        new.ends_with(tail),
        "text after the target changed:\nold tail {tail:?}\nnew {new:?}"
    );
    assert!(new.len() >= range.start + tail.len());
}

#[test]
fn thirty_edits_change_only_their_target() {
    let dir = tempfile::tempdir().unwrap();
    let lf = "---\n# a comment\ntitle: 'Quoted Title'\ntags:\n  - one\n  - two\nstatus: draft   # inline comment\nempty:\n---\n# Week 1\n## Plan\nplan one\n## Notes\nnotes one\n# Week 2\n## Plan\nplan two\nlast line without newline";
    write(dir.path(), "LF.md", lf);
    write(dir.path(), "CRLF.md", &lf.replace('\n', "\r\n"));
    write(dir.path(), "Bare.md", "# Only\nbody\n");
    let t = tools_for(dir.path());

    let section_edits = [
        ("Week 1#Plan", "replace", "new plan\n"),
        ("Week 1#Plan", "append", "appended"),
        ("Week 1#Plan", "prepend", "first\n"),
        ("Notes", "replace", "fresh notes"),
        ("Week 2#Plan", "append", "after the last line"),
        ("Week 2#Plan", "replace", "two\nlines\n"),
        ("Week 2", "prepend", "intro of week 2\n"),
        ("Week 1", "replace", "## Plan\nall of week 1 replaced\n"),
        ("Week 1#Plan", "replace", ""),
        ("Week 2#Plan", "prepend", "top"),
    ];
    let property_edits: [(&str, Value); 10] = [
        ("status", json!("done")),
        ("tags", json!(["one", "two", "three"])),
        ("title", json!("Plain Title")),
        ("new prop", json!(42)),
        ("empty", json!(true)),
        ("Status", json!("back: with colon")),
        ("tags", json!([])),
        ("nested", json!({"a": 1, "b": ["x", "y"]})),
        ("new prop", Value::Null),
        ("date", json!("2026-02-01")),
    ];
    let mut count = 0;
    for file in ["LF", "CRLF"] {
        for (section, mode, content) in section_edits.iter().take(if file == "LF" { 10 } else { 5 })
        {
            let old = fs::read_to_string(dir.path().join(format!("{file}.md"))).unwrap();
            let note = Note::parse(
                VaultPath::parse(&format!("{file}.md")).unwrap(),
                old.clone(),
            );
            let sec = match note.find_section(section) {
                vault::SectionLookup::Found(s) => s,
                other => panic!("{section}: {other:?}"),
            };
            let target = match *mode {
                "replace" => note.headings[sec.heading].end..sec.range.end,
                "append" => sec.range.end..sec.range.end,
                _ => note.headings[sec.heading].end..note.headings[sec.heading].end,
            };
            let v = ok(
                &t,
                "edit_section",
                json!({"note": file, "section": section, "mode": mode, "content": content, "version": version(&t, file)}),
            );
            let new = fs::read_to_string(dir.path().join(format!("{file}.md"))).unwrap();
            assert_eq!(v["version"], version(&t, file));
            only_inside(&old, &new, target);
            if file == "CRLF" {
                assert!(
                    !new.replace("\r\n", "").contains('\n'),
                    "a bare LF crept into a CRLF file: {new:?}"
                );
            }
            count += 1;
        }
        for (name, value) in property_edits
            .iter()
            .take(if file == "LF" { 10 } else { 5 })
        {
            let old = fs::read_to_string(dir.path().join(format!("{file}.md"))).unwrap();
            let note = Note::parse(
                VaultPath::parse(&format!("{file}.md")).unwrap(),
                old.clone(),
            );
            let target = match note.property_span(name) {
                Some((r, _)) => r,
                None => {
                    let b = note.properties_block.clone().unwrap();
                    b.end..b.end
                }
            };
            let v = ok(
                &t,
                "set_property",
                json!({"note": file, "name": name, "value": value, "version": version(&t, file)}),
            );
            let new = fs::read_to_string(dir.path().join(format!("{file}.md"))).unwrap();
            only_inside(&old, &new, target);
            let props = ok(&t, "read_note", json!({"note": file}))["properties"].clone();
            let key = props
                .as_object()
                .unwrap()
                .keys()
                .find(|k| k.to_lowercase() == name.to_lowercase())
                .cloned();
            match value {
                Value::Null => assert!(key.is_none()),
                v => assert_eq!(&props[key.unwrap()], v, "{name}"),
            }
            assert_eq!(v["properties"], props);
            if file == "CRLF" {
                assert!(
                    !new.replace("\r\n", "").contains('\n'),
                    "a bare LF crept into a CRLF file: {new:?}"
                );
            }
            count += 1;
        }
    }
    assert_eq!(count, 30);
    let lf_now = fs::read_to_string(dir.path().join("LF.md")).unwrap();
    assert!(
        lf_now.starts_with("---\n# a comment\ntitle: Plain Title\n"),
        "{lf_now}"
    );
    // The key keeps the file's spelling even when named in another case.
    assert!(
        lf_now.contains("\nstatus: \"back: with colon\"\n"),
        "key spelling and quoting: {lf_now}"
    );

    // A note without properties gets a block at the top, and nothing else changes.
    let v = ok(
        &t,
        "set_property",
        json!({"note": "Bare", "name": "status", "value": "new", "version": version(&t, "Bare")}),
    );
    assert_eq!(v["properties"], json!({"status": "new"}));
    assert_eq!(
        fs::read_to_string(dir.path().join("Bare.md")).unwrap(),
        "---\nstatus: new\n---\n# Only\nbody\n"
    );
}

#[test]
fn invalid_yaml_is_refused_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let text = "---\ntitle: [unclosed\n---\nbody\n";
    write(dir.path(), "Bad.md", text);
    let t = tools_for(dir.path());
    let e = err(
        &t,
        "set_property",
        json!({"note": "Bad", "name": "x", "value": 1, "version": version(&t, "Bad")}),
    );
    assert_eq!(e["error"], "invalid_properties");
    assert_eq!(fs::read_to_string(dir.path().join("Bad.md")).unwrap(), text);
}

#[test]
fn section_edit_arguments() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "S.md", "# A\nx\n# A\ny\n");
    let t = tools_for(dir.path());
    let v = version(&t, "S");
    assert_eq!(
        err(
            &t,
            "edit_section",
            json!({"note": "S", "section": "A", "content": "z", "version": v})
        )["error"],
        "ambiguous_section"
    );
    assert_eq!(
        err(
            &t,
            "edit_section",
            json!({"note": "S", "content": "z", "version": v})
        )["error"],
        "invalid_argument"
    );
    ok(
        &t,
        "edit_section",
        json!({"note": "S", "heading_line": 3, "content": "z", "version": v}),
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("S.md")).unwrap(),
        "# A\nx\n# A\nz\n"
    );
}

#[cfg(unix)]
#[test]
fn edits_keep_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "P.md", "# H\nbody\n");
    fs::set_permissions(dir.path().join("P.md"), fs::Permissions::from_mode(0o640)).unwrap();
    let t = tools_for(dir.path());
    ok(
        &t,
        "edit_section",
        json!({"note": "P", "section": "H", "content": "new\n", "version": version(&t, "P")}),
    );
    let mode = fs::metadata(dir.path().join("P.md"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640);
}

// ---- Rule 7: stale edits are refused ----

#[test]
fn stale_edits_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "N.md", "---\na: 1\n---\n# H\nbody\n");
    let t = tools_for(dir.path());
    let v = version(&t, "N");
    // Someone else changes the note after the agent read it.
    write(
        dir.path(),
        "N.md",
        "---\na: 1\n---\n# H\nbody changed elsewhere\n",
    );
    for (tool, args) in [
        (
            "edit_section",
            json!({"note": "N", "section": "H", "content": "mine", "version": v}),
        ),
        (
            "set_property",
            json!({"note": "N", "name": "a", "value": 2, "version": v}),
        ),
        ("move_note", json!({"note": "N", "to": "M", "version": v})),
        ("delete_note", json!({"note": "N", "version": v})),
    ] {
        let e = err(&t, tool, args);
        assert_eq!(e["error"], "stale", "{tool}");
        assert!(
            e["message"].as_str().unwrap().contains("Read it again"),
            "{tool}: {e}"
        );
        assert_eq!(e["version"], version(&t, "N"));
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("N.md")).unwrap(),
        "---\na: 1\n---\n# H\nbody changed elsewhere\n"
    );
}

// ---- Rule 8: delete to the trash, create refuses a taken path ----

#[test]
fn delete_goes_to_trash_and_lists_links() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Gone.md", "bye\n");
    write(dir.path(), "Sub/Gone.md", "the other\n");
    write(dir.path(), "A.md", "[[Gone]] and ![[Gone]]\n");
    write(dir.path(), "B.md", "[g](Gone.md)\n");
    write(dir.path(), ".trash/Gone.md", "already in the trash\n");
    write(dir.path(), ".trash/Gone 1.md", "also there\n");
    let t = tools_for(dir.path());
    let v = ok(&t, "delete_note", json!({"note": "Gone.md"}));
    assert_eq!(v["deleted"], true);
    assert_eq!(v["trash_path"], ".trash/Gone 2.md");
    assert_eq!(v["total_now_broken"], 3);
    let from: Vec<(&str, usize)> = v["now_broken"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["from"].as_str().unwrap(),
                l["line"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(from, vec![("A.md", 1), ("A.md", 1), ("B.md", 1)]);
    assert!(!dir.path().join("Gone.md").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join(".trash/Gone 2.md")).unwrap(),
        "bye\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".trash/Gone.md")).unwrap(),
        "already in the trash\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".trash/Gone 1.md")).unwrap(),
        "also there\n"
    );
    // Deleting the second one of that name.
    let v = ok(&t, "delete_note", json!({"note": "Sub/Gone"}));
    assert_eq!(v["trash_path"], ".trash/Gone 3.md");
}

#[test]
fn create_note_rules() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "root index\n");
    write(dir.path(), "a/plan.md", "the plan\n");
    write(dir.path(), "a/Uses.md", "[[plan]] and [p](plan.md)\n");
    write(dir.path(), "Root.md", "[[plan]]\n");
    let t = tools_for(dir.path());

    let e = err(&t, "create_note", json!({"path": "INDEX"}));
    assert_eq!(e["error"], "already_exists");
    assert_eq!(e["path"], "index.md");

    // The same name in another folder is allowed and reported.
    let v = ok(
        &t,
        "create_note",
        json!({"path": "goals/index", "content": "# Goals\n", "properties": {"status": "new", "tags": ["g"]}}),
    );
    assert_eq!(v["path"], "goals/index.md");
    assert_eq!(v["same_name"], json!(["index.md"]));
    assert_eq!(v["updated_notes"], json!([]));
    assert_eq!(
        fs::read_to_string(dir.path().join("goals/index.md")).unwrap(),
        "---\nstatus: new\ntags:\n  - g\n---\n# Goals\n"
    );
    assert_eq!(v["version"], version(&t, "goals/index"));
    let e = err(&t, "read_note", json!({"note": "index"}));
    assert_eq!(e["error"], "ambiguous");
    assert_eq!(e["candidates"], json!(["goals/index.md", "index.md"]));
    assert_eq!(
        ok(&t, "read_note", json!({"note": "index.md"}))["path"],
        "index.md"
    );

    // A new b/plan.md would make the bare [[plan]] links ambiguous: they get a path.
    let v = ok(
        &t,
        "create_note",
        json!({"path": "b/plan.md", "content": "other plan\n"}),
    );
    assert_eq!(v["updated_notes"], json!(["Root.md", "a/Uses.md"]));
    assert_eq!(
        fs::read_to_string(dir.path().join("Root.md")).unwrap(),
        "[[a/plan]]\n"
    );
    // From a/Uses.md the Markdown link is an exact relative path, so it stays.
    assert_eq!(
        fs::read_to_string(dir.path().join("a/Uses.md")).unwrap(),
        "[[a/plan]] and [p](plan.md)\n"
    );

    let e = err(
        &t,
        "create_note",
        json!({"path": "c.md", "content": "---\nx: 1\n---\n", "properties": {"y": 2}}),
    );
    assert_eq!(e["error"], "invalid_argument");
}

#[test]
fn ambiguous_links_are_flagged() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a/plan.md", "a\n");
    write(dir.path(), "b/plan.md", "b\n");
    write(dir.path(), "Uses.md", "[[plan]] [[a/plan]]\n");
    let t = tools_for(dir.path());
    let v = ok(&t, "get_links", json!({"note": "Uses"}));
    assert_eq!(v["outgoing"][0]["to"], Value::Null);
    assert_eq!(
        v["outgoing"][0]["ambiguous"],
        json!(["a/plan.md", "b/plan.md"])
    );
    assert_eq!(v["outgoing"][1]["to"], "a/plan.md");
    let v = ok(&t, "check_vault", json!({}));
    assert_eq!(v["total_broken_links"], 1);
    assert_eq!(v["broken_links"][0]["reason"], "ambiguous");
    let v = ok(&t, "get_links", json!({"note": "b/plan"}));
    assert_eq!(v["incoming"], json!([]));
    // Moving a note leaves a link that was already ambiguous as written.
    ok(&t, "move_note", json!({"note": "b/plan", "to": "c/plan"}));
    assert_eq!(
        fs::read_to_string(dir.path().join("Uses.md")).unwrap(),
        "[[plan]] [[a/plan]]\n"
    );
}

// ---- Rule 9: every new path argument stays inside the vault ----

const ESCAPES: &[&str] = &[
    "outside",
    "outside/sub",
    "outside/Secret.md",
    "outside/sub/Deep",
    "outside/nothing/here",
    "Leak",
    "Leak.md",
    "..",
    "../x",
    "Folder/../../x",
    "/etc",
    "/etc/passwd",
    "C:\\Windows",
    "~/notes",
    ".obsidian",
    ".obsidian/Config",
    ".git",
    ".git/Head.md",
    "a/.trash/x",
];

/// Builds `{tool, args}` for one escape form.
type ArgsFor = Box<dyn Fn(&str) -> Value>;

#[cfg(unix)]
#[test]
fn change_tools_refuse_paths_outside() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("v");
    let outside = base.path().join("out");
    write(&dir, "Inside.md", "# H\nbody\n");
    write(&dir, ".obsidian/Config.md", "c\n");
    write(&dir, ".git/Head.md", "h\n");
    write(&outside, "Secret.md", "secret\n");
    write(&outside, "sub/Deep.md", "deep\n");
    std::os::unix::fs::symlink(&outside, dir.join("outside")).unwrap();
    std::os::unix::fs::symlink(outside.join("Secret.md"), dir.join("Leak.md")).unwrap();
    let t = tools_for(&dir);
    let before_out = all_files(&outside);
    let v = version(&t, "Inside");
    let cases: Vec<(&str, ArgsFor)> = vec![
        (
            "create_note.path",
            Box::new(|q| json!({"tool": "create_note", "args": {"path": q, "content": "x"}})),
        ),
        (
            "move_note.to",
            Box::new(|q| json!({"tool": "move_note", "args": {"note": "Inside", "to": q}})),
        ),
        (
            "move_note.note",
            Box::new(|q| json!({"tool": "move_note", "args": {"note": q, "to": "Elsewhere"}})),
        ),
        (
            "edit_section.note",
            Box::new(
                move |q| json!({"tool": "edit_section", "args": {"note": q, "section": "H", "content": "x", "version": v}}),
            ),
        ),
        (
            "set_property.note",
            Box::new(
                |q| json!({"tool": "set_property", "args": {"note": q, "name": "a", "value": 1, "version": "x"}}),
            ),
        ),
        (
            "delete_note.note",
            Box::new(|q| json!({"tool": "delete_note", "args": {"note": q}})),
        ),
    ];
    for (label, make) in &cases {
        for q in ESCAPES {
            let c = make(q);
            let (e, is_err) = call(&t, c["tool"].as_str().unwrap(), c["args"].clone());
            assert!(is_err, "{label}={q} was accepted: {e}");
            assert_eq!(e["error"], "outside_vault", "{label}={q}: {e}");
        }
    }
    assert_eq!(all_files(&outside), before_out, "nothing outside changed");
    assert!(dir.join("Inside.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join(".obsidian/Config.md")).unwrap(),
        "c\n"
    );
}

// ---- Rule 10: change replies stay small ----

#[test]
fn change_replies_stay_small() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Target.md", "t\n");
    for i in 0..300 {
        write(
            dir.path(),
            &format!("Linkers/A long folder name for size/Linker number {i:03}.md"),
            "[[Target]] [[Target]]\n",
        );
    }
    let t = tools_for(dir.path());
    // Only call() checks sizes; every reply below is checked.
    let v = ok(
        &t,
        "move_note",
        json!({"note": "Target", "to": "Moved/Elsewhere/Target"}),
    );
    assert_eq!(v["total_updated_notes"], 0, "a unique name still resolves");
    ok(
        &t,
        "create_note",
        json!({"path": "Other/Target", "content": "capture"}),
    );
    let v = ok(
        &t,
        "move_note",
        json!({"note": "Moved/Elsewhere/Target", "to": "Third/Target"}),
    );
    assert_eq!(v["total_updated_notes"], 300);
    assert_eq!(v["cut"], json!(["updated_notes"]));
    let v = ok(&t, "delete_note", json!({"note": "Third/Target"}));
    assert_eq!(v["total_now_broken"], 600);
    assert_eq!(v["cut"], json!(["now_broken"]));
    let big = "x".repeat(20 * 1024);
    let v = ok(
        &t,
        "create_note",
        json!({"path": "Big", "properties": {"huge": big}}),
    );
    let v2 = ok(
        &t,
        "set_property",
        json!({"note": "Big", "name": "small", "value": 1, "version": v["version"]}),
    );
    assert_eq!(v2["cut"], json!(["properties"]));
}
