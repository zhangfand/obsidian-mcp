//! Path arguments that leave the vault, and items too large for one reply.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tools::{REPLY_LIMIT, Tools};
use vault::Vault;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn call(t: &Tools, name: &str, args: Value) -> (Value, bool) {
    let r = t.call(name, Some(args.clone()));
    assert!(r.text.len() <= REPLY_LIMIT, "{name} {args} reply is {} bytes", r.text.len());
    (serde_json::from_str(&r.text).unwrap(), r.is_error)
}

fn ok(t: &Tools, name: &str, args: Value) -> Value {
    let (v, err) = call(t, name, args);
    assert!(!err, "unexpected error: {v}");
    v
}

/// Calls every page; returns the pages.
fn all_pages(t: &Tools, name: &str, mut args: Value) -> Vec<Value> {
    let mut pages = Vec::new();
    loop {
        let v = ok(t, name, args.clone());
        let next = v["next_page"].as_str().map(String::from);
        pages.push(v);
        match next {
            Some(p) => args["page"] = json!(p),
            None => return pages,
        }
    }
}

fn joined(pages: &[Value]) -> String {
    pages.iter().map(|p| p["content"].as_str().unwrap()).collect()
}

// ---- Path arguments ----

/// A vault with a folder and a file symlinked to the outside, plus hidden
/// folders holding notes.
fn escape_fixture() -> (tempfile::TempDir, tempfile::TempDir, Tools) {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(dir.path(), "Inside.md", "[[Inside]]\n");
    write(dir.path(), ".obsidian/Config.md", "secret\n");
    write(dir.path(), ".git/Head.md", "secret\n");
    write(outside.path(), "Secret.md", "[[Missing behind the symlink]]\n");
    write(outside.path(), "sub/Deep.md", "secret\n");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("outside")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("Secret.md"), dir.path().join("Leak.md")).unwrap();
    let t = Tools::new(Vault::open(dir.path()).unwrap());
    (dir, outside, t)
}

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

fn assert_refused(t: &Tools, tool: &str, arg: &str) {
    for q in ESCAPES {
        let (v, err) = call(t, tool, json!({ arg: q }));
        assert!(err, "{tool}.{arg}={q} was answered: {v}");
        assert_eq!(v["error"], "outside_vault", "{tool}.{arg}={q}: {v}");
    }
}

#[test]
fn find_notes_folder_refuses_outside() {
    let (_d, _o, t) = escape_fixture();
    assert_refused(&t, "find_notes", "folder");
    // A folder inside the vault is still fine.
    assert_eq!(ok(&t, "find_notes", json!({"folder": ""}))["total"], 1);
}

#[test]
fn read_note_note_refuses_outside() {
    let (_d, _o, t) = escape_fixture();
    assert_refused(&t, "read_note", "note");
}

#[test]
fn get_links_note_refuses_outside() {
    let (_d, _o, t) = escape_fixture();
    assert_refused(&t, "get_links", "note");
}

#[test]
fn check_vault_never_shows_outside() {
    let (_d, _o, t) = escape_fixture();
    // check_vault takes no path; it must not see notes behind the symlinks,
    // whose broken link would otherwise be reported.
    let v = ok(&t, "check_vault", json!({}));
    assert_eq!(v["total_broken_links"], 0, "{v}");
}

// ---- Items over 8 KB ----

const BIG: usize = 20 * 1024;

fn big_word(c: char) -> String {
    std::iter::repeat_n(c, BIG).collect()
}

#[test]
fn huge_link_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = big_word('t');
    write(dir.path(), "A.md", &format!("[[{target}|{}]] and [[B]]\n", big_word('d')));
    write(dir.path(), "B.md", "[[A]]\n");
    let t = Tools::new(Vault::open(dir.path()).unwrap());

    let v = ok(&t, "get_links", json!({"note": "A"}));
    let big = &v["outgoing"][0];
    assert_eq!(big["cut"], json!(["target", "display"]));
    assert!(big["target"].as_str().unwrap().ends_with('…'));
    assert_eq!(big["line"], 1);
    assert_eq!(v["outgoing"][1]["to"], "B.md");
    assert!(v["outgoing"][1].get("cut").is_none());

    let v = ok(&t, "check_vault", json!({}));
    assert_eq!(v["total_broken_links"], 1);
    assert_eq!(v["broken_links"][0]["cut"], json!(["target", "display"]));

    // The whole target is still readable from the note itself.
    let text = joined(&all_pages(&t, "read_note", json!({"note": "A", "full": true})));
    assert!(text.contains(&target));
}

#[test]
fn huge_properties_block() {
    let dir = tempfile::tempdir().unwrap();
    let block = format!("status: ok\nsummary: {}\ntags: [small]\n", big_word('p'));
    write(dir.path(), "P.md", &format!("---\n{block}---\n# Body\ntext\n"));
    let t = Tools::new(Vault::open(dir.path()).unwrap());

    let v = ok(&t, "read_note", json!({"note": "P"}));
    assert_eq!(v["mode"], "full");
    assert_eq!(v["properties"], json!({"status": "ok", "tags": ["small"]}));
    assert_eq!(v["cut"], json!(["properties"]));
    assert_eq!(v["content"], "# Body\ntext\n");

    let pages = all_pages(&t, "read_note", json!({"note": "P", "raw_properties": true}));
    assert!(pages.len() > 2);
    assert!(pages.iter().all(|p| p["mode"] == "properties"));
    assert_eq!(joined(&pages), block);
}

#[test]
fn huge_single_line() {
    let dir = tempfile::tempdir().unwrap();
    let line = format!("start {} needle {} #{}\n", big_word('x'), big_word('"'), big_word('g'));
    write(dir.path(), "L.md", &format!("# Top\n{line}## Next\nend\n"));
    let t = Tools::new(Vault::open(dir.path()).unwrap());

    let v = ok(&t, "find_notes", json!({"text": "needle"}));
    let note = &v["notes"][0];
    assert!(note["snippets"][0]["text"].as_str().unwrap().contains("needle"));
    // The 20 KB tag is left out rather than shortened into a wrong tag.
    assert_eq!(note["tags"], json!([]));
    assert_eq!(note["cut"], json!(["tags"]));

    let v = ok(&t, "read_note", json!({"note": "L"}));
    assert_eq!(v["mode"], "outline");
    assert_eq!(v["cut"], json!(["tags"]));

    let pages = all_pages(&t, "read_note", json!({"note": "L", "section": "Top"}));
    assert!(pages.len() > 3);
    // `## Next` is inside `# Top`.
    assert_eq!(joined(&pages), format!("# Top\n{line}## Next\nend\n"));
    let pages = all_pages(&t, "read_note", json!({"note": "L", "full": true}));
    assert_eq!(joined(&pages), format!("# Top\n{line}## Next\nend\n"));
}

#[test]
fn huge_heading() {
    let dir = tempfile::tempdir().unwrap();
    let heading = big_word('h');
    let body = format!("# {heading}\nunder it\n## Small\nx\n");
    write(dir.path(), "H.md", &body);
    write(dir.path(), "Linker.md", &format!("[[H#{heading}]]\n"));
    let t = Tools::new(Vault::open(dir.path()).unwrap());

    let v = ok(&t, "read_note", json!({"note": "H"}));
    assert_eq!(v["mode"], "outline", "{v}");
    let first = &v["outline"][0];
    assert_eq!(first["cut"], json!(["section"]));
    assert_eq!(first["line"], 1);
    assert_eq!(v["outline"][1]["section"], "Small");

    let v = ok(&t, "read_note", json!({"note": "H", "heading_line": 1}));
    assert_eq!(v["mode"], "section");
    assert_eq!(v["cut"], json!(["section"]));
    let pages = all_pages(&t, "read_note", json!({"note": "H", "heading_line": 1}));
    assert_eq!(joined(&pages), body);

    // Asking by the full heading text works too, and its error lists stay small.
    let pages = all_pages(&t, "read_note", json!({"note": "H", "section": heading}));
    assert_eq!(joined(&pages), body);
    let (e, err) = call(&t, "read_note", json!({"note": "H", "section": "missing"}));
    assert!(err);
    assert_eq!(e["error"], "section_not_found");
    assert_eq!(e["headings"], json!(["Small"]));
    assert_eq!(e["cut"], json!(["headings"]));
    let (e, _) = call(&t, "read_note", json!({"note": "H", "heading_line": 99}));
    assert_eq!(e["error"], "section_not_found");

    let v = ok(&t, "get_links", json!({"note": "Linker"}));
    assert_eq!(v["outgoing"][0]["to"], "H.md");
    assert_eq!(v["outgoing"][0]["cut"], json!(["heading"]));
}

#[test]
fn huge_arguments_stay_small() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "A.md", "a\n");
    let t = Tools::new(Vault::open(dir.path()).unwrap());
    let (e, _) = call(&t, "read_note", json!({"note": big_word('q')}));
    assert_eq!(e["error"], "not_found");
    let (e, _) = call(&t, "read_note", json!({"note": "A", big_word('k'): 1}));
    assert_eq!(e["error"], "invalid_argument");
    let (e, _) = call(&t, &big_word('n'), json!({}));
    assert_eq!(e["error"], "invalid_argument");
    let (e, _) = call(&t, "find_notes", json!({"folder": format!("../{}", big_word('f'))}));
    assert_eq!(e["error"], "outside_vault");
}
