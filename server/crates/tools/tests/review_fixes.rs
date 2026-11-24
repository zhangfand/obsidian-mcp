//! A failed change leaves the vault as it was; notes that are not valid
//! UTF-8, paths that differ only in letter case, and symlinks into hidden
//! folders are handled safely.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tools::Tools;
use vault::Vault;

fn write(root: &Path, rel: &str, text: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    (base, root)
}

fn tools_for(root: &Path) -> Tools {
    let tmp = root.parent().unwrap().join("tmp");
    Tools::new(Vault::open(root).unwrap().with_tmp_dir(tmp).unwrap())
}

fn call(t: &Tools, name: &str, args: Value) -> (Value, bool) {
    let r = t.call(name, Some(args));
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

fn read(root: &Path, rel: &str) -> Vec<u8> {
    fs::read(root.join(rel)).unwrap()
}

// ---- A failed move or create changes nothing ----

#[test]
fn a_move_that_cannot_finish_rewrites_no_links() {
    let (_base, root) = setup();
    write(&root, "old/a.md", b"A\n");
    write(&root, "other/a.md", b"other A\n");
    write(&root, "b.md", b"see [[old/a]]\n");
    write(&root, "blocker", b"a file, not a folder");
    let t = tools_for(&root);
    let e = err(
        &t,
        "move_note",
        json!({"note": "old/a", "to": "blocker/a.md"}),
    );
    assert_eq!(e["error"], "io_error");
    let msg = e["message"].as_str().unwrap();
    assert!(msg.contains("could not be written"), "{msg}");
    assert!(msg.contains("blocker is a file"), "{msg}");
    assert_eq!(read(&root, "b.md"), b"see [[old/a]]\n");
    assert_eq!(read(&root, "old/a.md"), b"A\n");
    let check = ok(&t, "check_vault", json!({}));
    assert_eq!(check["total_broken_links"], 0);
}

#[test]
fn a_create_that_cannot_finish_rewrites_no_links() {
    let (_base, root) = setup();
    write(&root, "other/x.md", b"X\n");
    write(&root, "b.md", b"see [[x]]\n");
    write(&root, "blocker", b"a file, not a folder");
    let t = tools_for(&root);
    let e = err(
        &t,
        "create_note",
        json!({"path": "blocker/x.md", "content": "new"}),
    );
    assert_eq!(e["error"], "io_error");
    assert_eq!(read(&root, "b.md"), b"see [[x]]\n");
    assert!(!root.join("blocker/x.md").exists());
}

#[test]
fn a_move_of_a_note_changed_meanwhile_rewrites_no_links() {
    let (_base, root) = setup();
    write(&root, "old/a.md", b"A\n");
    write(&root, "other/a.md", b"other A\n");
    write(&root, "b.md", b"see [[old/a]]\n");
    let t = tools_for(&root);
    let v = ok(&t, "read_note", json!({"note": "old/a"}))["version"].clone();
    write(&root, "old/a.md", b"A, edited in Obsidian\n");
    let e = err(
        &t,
        "move_note",
        json!({"note": "old/a", "to": "new/a.md", "version": v}),
    );
    assert_eq!(e["error"], "stale");
    assert_eq!(read(&root, "b.md"), b"see [[old/a]]\n");
    assert!(!root.join("new/a.md").exists());
}

// ---- Notes that are not valid UTF-8 ----

#[test]
fn a_note_that_is_not_utf8_is_refused_clearly_and_kept_intact() {
    let (_base, root) = setup();
    let bytes = b"hello \xff world\n";
    write(&root, "bad.md", bytes);
    write(&root, "links.md", b"[[bad]]\n");
    let t = tools_for(&root);
    let v = ok(&t, "read_note", json!({"note": "bad"}))["version"].clone();

    for (tool, args) in [
        (
            "edit_note",
            json!({"note": "bad", "version": v, "old_text": "hello", "new_text": "hi"}),
        ),
        (
            "edit_note",
            json!({"note": "bad", "version": v, "section": "", "content": "x"}),
        ),
        (
            "set_property",
            json!({"note": "bad", "version": v, "name": "a", "value": 1}),
        ),
    ] {
        let e = err(&t, tool, args);
        assert_eq!(e["error"], "invalid_argument", "{tool}: {e}");
        assert!(
            e["message"].as_str().unwrap().contains("not valid UTF-8"),
            "{e}"
        );
        assert_eq!(read(&root, "bad.md"), bytes);
    }

    // It can still be moved and deleted, bytes intact.
    ok(&t, "move_note", json!({"note": "bad", "to": "kept/bad.md"}));
    assert_eq!(read(&root, "kept/bad.md"), bytes);
    let d = ok(&t, "delete_note", json!({"note": "bad"}));
    assert_eq!(read(&root, d["trash_path"].as_str().unwrap()), bytes);
}

#[test]
fn a_move_that_would_rewrite_a_note_that_is_not_utf8_is_refused() {
    let (_base, root) = setup();
    write(&root, "old/a.md", b"A\n");
    write(&root, "other/a.md", b"other A\n");
    let bytes = b"\xfe see [[old/a]]\n";
    write(&root, "bad.md", bytes);
    let t = tools_for(&root);
    let e = err(&t, "move_note", json!({"note": "old/a", "to": "new/a.md"}));
    assert_eq!(e["error"], "invalid_argument");
    assert!(
        e["message"].as_str().unwrap().contains("not valid UTF-8"),
        "{e}"
    );
    assert_eq!(read(&root, "bad.md"), bytes);
    assert!(root.join("old/a.md").exists());
}

// ---- Paths that differ only in letter case ----

#[test]
fn paths_that_differ_only_in_case_are_told_apart() {
    let (_base, root) = setup();
    write(&root, "Note.md", b"upper\n");
    write(&root, "note.md", b"lower\n");
    if fs::read(root.join("Note.md")).unwrap() == b"lower\n" {
        return; // a case-insensitive file system holds only one of them
    }
    let t = tools_for(&root);
    assert_eq!(
        ok(&t, "read_note", json!({"note": "Note.md"}))["path"],
        "Note.md"
    );
    assert_eq!(
        ok(&t, "read_note", json!({"note": "note.md"}))["path"],
        "note.md"
    );
    // A bare name matches regardless of case, so it stays ambiguous, as before.
    for bare in ["note", "Note", "NOTE"] {
        let e = err(&t, "read_note", json!({"note": bare}));
        assert_eq!(e["error"], "ambiguous", "{bare}");
    }

    // Renaming one onto the other is refused, and nothing is overwritten.
    let e = err(&t, "move_note", json!({"note": "Note.md", "to": "note.md"}));
    assert_eq!(e["error"], "already_exists");
    let e = err(&t, "move_note", json!({"note": "Note.md", "to": "NOTE.md"}));
    assert_eq!(e["error"], "already_exists");
    assert_eq!(read(&root, "Note.md"), b"upper\n");
    assert_eq!(read(&root, "note.md"), b"lower\n");
}

#[test]
fn a_case_only_rename_still_works() {
    let (_base, root) = setup();
    write(&root, "Note.md", b"upper\n");
    write(&root, "b.md", b"[[Note]]\n");
    let t = tools_for(&root);
    ok(&t, "move_note", json!({"note": "Note.md", "to": "NOTE.md"}));
    assert_eq!(read(&root, "NOTE.md"), b"upper\n");
}

// ---- Symlinks into hidden folders ----

#[cfg(unix)]
#[test]
fn symlinks_into_hidden_folders_are_not_part_of_the_vault() {
    use std::os::unix::fs::symlink;
    let (_base, root) = setup();
    write(&root, ".obsidian/secret.md", b"plugin token xyz\n");
    write(&root, "Visible.md", b"visible\n");
    symlink(root.join(".obsidian/secret.md"), root.join("peek.md")).unwrap();
    symlink(root.join(".obsidian"), root.join("cfg")).unwrap();
    symlink(root.join("Visible.md"), root.join("Alias of visible.md")).unwrap();
    let t = tools_for(&root);

    for name in ["peek", "peek.md"] {
        let e = err(&t, "read_note", json!({"note": name}));
        assert_eq!(e["error"], "outside_vault", "{name}");
    }
    let e = err(&t, "read_note", json!({"note": "cfg/secret.md"}));
    assert_eq!(e["error"], "outside_vault");
    let e = err(
        &t,
        "create_note",
        json!({"path": "cfg/new.md", "content": "x"}),
    );
    assert_eq!(e["error"], "outside_vault");
    let found = ok(&t, "find_notes", json!({"text": "token"}));
    assert_eq!(found["total"], 0);
    assert!(!root.join(".obsidian/new.md").exists());

    // A symlink to a visible note is still a note, as before.
    assert_eq!(
        ok(&t, "read_note", json!({"note": "Alias of visible"}))["content"],
        "visible\n"
    );
}
