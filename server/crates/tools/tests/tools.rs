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

fn fixture() -> (tempfile::TempDir, Tools) {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        "Projects/Alpha.md",
        "---\naliases: [The First]\ntags: [project/alpha]\nstatus: Active\npriority: 2\n---\n# Alpha\nIntro with the word Zebra.\n## Plan\nStep one, see [[Beta#Goals|goals]].\n## Notes\n#active thing\n",
    );
    write(
        r,
        "Beta.md",
        "---\nstatus: done\ntags: project\n---\n# Goals\n![[Alpha]] and [[The First]] and [[Projects/Alpha#Plan]].\n",
    );
    write(r, "Gamma.md", "Nothing links here. #projects\n");
    write(r, "Long.md", &long_note());
    write(r, "Flat.md", &"a line without headings\n".repeat(800));
    let tools = Tools::new(Vault::open(r).unwrap());
    (dir, tools)
}

fn long_note() -> String {
    let mut s = String::from("# Week 1\n## Plan\n");
    s.push_str(&"monday tasks\n".repeat(50));
    s.push_str("# Week 2\n## Plan\n");
    s.push_str(&"tuesday tasks are long\n".repeat(600));
    s.push_str("## Review\ndone\n");
    s
}

fn call(t: &Tools, name: &str, args: Value) -> (Value, bool) {
    let r = t.call(name, Some(args));
    assert!(
        r.text.len() <= REPLY_LIMIT,
        "{name} reply is {} bytes",
        r.text.len()
    );
    (serde_json::from_str(&r.text).unwrap(), r.is_error)
}

fn ok(t: &Tools, name: &str, args: Value) -> Value {
    let (v, err) = call(t, name, args);
    assert!(!err, "unexpected error: {v}");
    v
}

fn err(t: &Tools, name: &str, args: Value) -> Value {
    let (v, err) = call(t, name, args);
    assert!(err, "expected an error: {v}");
    v
}

fn paths(v: &Value) -> Vec<&str> {
    v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["path"].as_str().unwrap())
        .collect()
}

#[test]
fn find_by_tag_text_property() {
    let (_d, t) = fixture();
    assert_eq!(
        paths(&ok(&t, "find_notes", json!({"tag": "#project"}))),
        vec!["Beta.md", "Projects/Alpha.md"]
    );
    assert_eq!(
        paths(&ok(&t, "find_notes", json!({"tag": "Project/Alpha"}))),
        vec!["Projects/Alpha.md"]
    );
    assert_eq!(
        paths(&ok(&t, "find_notes", json!({"tag": "active"}))),
        vec!["Projects/Alpha.md"]
    );
    assert_eq!(
        paths(&ok(
            &t,
            "find_notes",
            json!({"property": {"name": "Status", "value": "active"}})
        )),
        vec!["Projects/Alpha.md"]
    );
    assert_eq!(
        paths(&ok(
            &t,
            "find_notes",
            json!({"property": {"name": "priority", "value": 2}})
        )),
        vec!["Projects/Alpha.md"]
    );
    assert_eq!(
        paths(&ok(
            &t,
            "find_notes",
            json!({"property": {"name": "status"}})
        )),
        vec!["Beta.md", "Projects/Alpha.md"]
    );
    assert_eq!(
        paths(&ok(&t, "find_notes", json!({"folder": "Projects"}))),
        vec!["Projects/Alpha.md"]
    );
    let v = ok(&t, "find_notes", json!({"text": "zebra"}));
    assert_eq!(paths(&v), vec!["Projects/Alpha.md"]);
    assert_eq!(
        v["notes"][0]["snippets"],
        json!([{"line": 8, "text": "Intro with the word Zebra."}])
    );
    assert!(
        ok(&t, "find_notes", json!({"tag": "project"}))["notes"][0]
            .get("snippets")
            .is_none()
    );
    err(&t, "find_notes", json!({"folder": "../x"}));
}

#[test]
fn find_pages_cover_each_note_once() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..400 {
        write(
            dir.path(),
            &format!("Many/Note number {i:03}.md"),
            "#bulk with some words\n",
        );
    }
    let t = Tools::new(Vault::open(dir.path()).unwrap());
    let mut seen = Vec::new();
    let mut page: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut args = json!({"tag": "bulk", "text": "words"});
        if let Some(p) = &page {
            args["page"] = json!(p);
        }
        let v = ok(&t, "find_notes", args);
        assert_eq!(v["total"], 400);
        seen.extend(paths(&v).into_iter().map(String::from));
        pages += 1;
        match v["next_page"].as_str() {
            Some(p) => page = Some(p.to_string()),
            None => break,
        }
    }
    assert!(pages > 1);
    let mut want: Vec<String> = (0..400)
        .map(|i| format!("Many/Note number {i:03}.md"))
        .collect();
    want.sort();
    assert_eq!(seen, want);
    // A token is bound to its arguments.
    let first = ok(&t, "find_notes", json!({"tag": "bulk"}));
    let e = err(
        &t,
        "find_notes",
        json!({"tag": "other", "page": first["next_page"]}),
    );
    assert_eq!(e["error"], "bad_page");
}

#[test]
fn read_modes() {
    let (_d, t) = fixture();
    let v = ok(&t, "read_note", json!({"note": "the first"}));
    assert_eq!(v["mode"], "full");
    assert_eq!(v["path"], "Projects/Alpha.md");
    assert_eq!(v["aliases"], json!(["The First"]));
    assert_eq!(v["tags"], json!(["project/alpha", "active"]));
    assert_eq!(v["properties"]["priority"], 2);
    assert!(v["content"].as_str().unwrap().starts_with("# Alpha\n"));
    assert_eq!(v["version"].as_str().unwrap().len(), 16);

    let v = ok(&t, "read_note", json!({"note": "Long"}));
    assert_eq!(v["mode"], "outline");
    assert!(v.get("content").is_none());
    let sections: Vec<&str> = v["outline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["section"].as_str().unwrap())
        .collect();
    assert_eq!(
        sections,
        vec!["Week 1", "Week 1#Plan", "Week 2", "Week 2#Plan", "Review"]
    );

    let e = err(&t, "read_note", json!({"note": "Long", "section": "plan"}));
    assert_eq!(e["error"], "ambiguous_section");
    assert_eq!(e["headings"], json!(["Week 1#Plan", "Week 2#Plan"]));
    let e = err(&t, "read_note", json!({"note": "Long", "section": "Nope"}));
    assert_eq!(e["error"], "section_not_found");

    let v = ok(
        &t,
        "read_note",
        json!({"note": "Long", "section": "Week 1#Plan"}),
    );
    assert_eq!(v["mode"], "section");
    assert_eq!(
        v["content"].as_str().unwrap(),
        format!("## Plan\n{}", "monday tasks\n".repeat(50))
    );
    assert!(v.get("next_page").is_none());
}

fn read_all(t: &Tools, mut args: Value) -> String {
    let mut out = String::new();
    loop {
        let v = ok(t, "read_note", args.clone());
        out.push_str(v["content"].as_str().unwrap());
        match v["next_page"].as_str() {
            Some(p) => args["page"] = json!(p),
            None => return out,
        }
    }
}

#[test]
fn long_text_pages_rejoin() {
    let (d, t) = fixture();
    let long = long_note();
    let start = long.find("## Plan\ntuesday").unwrap();
    let end = long.find("## Review").unwrap();
    assert_eq!(
        read_all(&t, json!({"note": "Long", "section": "Week 2#Plan"})),
        long[start..end]
    );
    assert_eq!(read_all(&t, json!({"note": "Long", "full": true})), long);
    // No headings: no outline to give, so the text is paged.
    assert_eq!(
        read_all(&t, json!({"note": "Flat"})),
        fs::read_to_string(d.path().join("Flat.md")).unwrap()
    );
}

#[test]
fn links_both_ways() {
    let (_d, t) = fixture();
    let v = ok(&t, "get_links", json!({"note": "Alpha"}));
    assert_eq!(
        v["outgoing"],
        json!([{"from": "Projects/Alpha.md", "to": "Beta.md", "target": "Beta", "kind": "wikilink", "heading": "Goals", "block": null, "display": "goals", "line": 10}])
    );
    let inc: Vec<(&str, &str, Value)> = v["incoming"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["kind"].as_str().unwrap(),
                l["target"].as_str().unwrap(),
                l["heading"].clone(),
            )
        })
        .collect();
    // [[The First]] is an alias: Obsidian leaves it unresolved.
    assert_eq!(
        inc,
        vec![
            ("embed", "Alpha", Value::Null),
            ("wikilink", "Projects/Alpha", json!("Plan"))
        ]
    );
    let v = ok(&t, "get_links", json!({"note": "Alpha", "direction": "in"}));
    assert_eq!(v["outgoing"], json!([]));
}

#[test]
fn check_vault_reports() {
    let (_d, t) = fixture();
    let v = ok(&t, "check_vault", json!({}));
    assert_eq!(v["total_broken_links"], 1);
    assert_eq!(v["broken_links"][0]["target"], "The First");
    assert_eq!(v["broken_links"][0]["reason"], "missing_note");
    assert_eq!(v["broken_links"].as_array().unwrap().len(), 1);
    assert!(v.get("unlinked_notes").is_none());
}

#[test]
fn errors_say_what_next() {
    let (_d, t) = fixture();
    let e = err(&t, "read_note", json!({"note": "Alhpa"}));
    assert_eq!(e["error"], "not_found");
    assert_eq!(e["candidates"][0], "Alpha");
    for q in [
        "../etc/passwd",
        "/etc/passwd",
        ".obsidian/app.json",
        ".git/HEAD",
        "a/../../b",
    ] {
        assert_eq!(
            err(&t, "read_note", json!({"note": q}))["error"],
            "outside_vault",
            "{q}"
        );
    }
    assert_eq!(err(&t, "read_note", json!({}))["error"], "invalid_argument");
    assert_eq!(
        err(&t, "read_note", json!({"note": "Alpha", "bogus": 1}))["error"],
        "invalid_argument"
    );
    assert_eq!(err(&t, "nope", json!({}))["error"], "invalid_argument");
}
