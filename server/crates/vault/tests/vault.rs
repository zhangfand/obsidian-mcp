use std::fs;
use std::path::Path;

use vault::{NoteLookup, PathError, Vault};

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// A small vault exercising name collisions, aliases, embeds, hidden
/// folders and symlinks.
fn fixture() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "Home.md", "# Home\nSee [[Alpha]] and [[Projects/Beta|the beta]] and ![[diagram.png]].\n[[Nowhere]] and [md](Projects/Beta.md#Plan)\n");
    write(r, "Alpha.md", "---\naliases: [First Letter]\ntags: [greek]\n---\nBack to [[home]]. Self [[#Top]].\n# Top\n");
    write(r, "Projects/Beta.md", "---\naliases: Second\n---\n# Plan\n[[Gamma]] ![[Missing.pdf]] [rel](../Alpha.md)\n");
    write(r, "Projects/Gamma.md", "[[First Letter]]\n");
    write(r, "Archive/Gamma.md", "old gamma\n");
    write(r, "Lonely.md", "no links here, only ![[diagram.png]]\n");
    write(r, "assets/diagram.png", "png");
    write(r, ".obsidian/app.json", "{}");
    write(r, ".obsidian/Hidden.md", "[[Alpha]]");
    write(r, ".trash/Old.md", "[[Alpha]]");
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "secret.md", "secret");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path().join("secret.md"), r.join("Escape.md")).unwrap();
        std::os::unix::fs::symlink(outside.path(), r.join("Outdir")).unwrap();
    }
    std::mem::forget(outside);
    let v = Vault::open(r).unwrap();
    (dir, v)
}

fn found(l: NoteLookup<'_>) -> String {
    match l {
        NoteLookup::Found(n) => n.path.to_string(),
        other => panic!("expected a note, got {other:?}"),
    }
}

#[test]
fn lookup_by_path_name_alias() {
    let (_d, v) = fixture();
    let s = v.snapshot().unwrap();
    assert_eq!(found(s.lookup("Projects/Beta.md")), "Projects/Beta.md");
    assert_eq!(found(s.lookup("projects/beta")), "Projects/Beta.md");
    assert_eq!(found(s.lookup("beta")), "Projects/Beta.md");
    assert_eq!(found(s.lookup("second")), "Projects/Beta.md");
    assert_eq!(found(s.lookup("first letter")), "Alpha.md");
    match s.lookup("Gamma") {
        NoteLookup::Ambiguous(ns) => {
            let paths: Vec<&str> = ns.iter().map(|n| n.path.as_str()).collect();
            assert_eq!(paths, vec!["Archive/Gamma.md", "Projects/Gamma.md"]);
        }
        other => panic!("{other:?}"),
    }
    match s.lookup("Alpah") {
        NoteLookup::NotFound(c) => assert_eq!(c[0], "Alpha"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn refuses_outside() {
    let (_d, v) = fixture();
    let s = v.snapshot().unwrap();
    for (q, e) in [
        ("../x", PathError::ParentDir),
        ("/etc/passwd", PathError::Absolute),
        (".obsidian/app.json", PathError::Hidden),
        (".git/config", PathError::Hidden),
        (".obsidian/Hidden", PathError::Hidden),
    ] {
        match s.lookup(q) {
            NoteLookup::Refused(got) => assert_eq!(got, e, "{q}"),
            other => panic!("{q}: {other:?}"),
        }
    }
    #[cfg(unix)]
    for q in ["Outdir", "Outdir/none/x.md"] {
        assert_eq!(s.safe_path(q), Err(PathError::LeavesVault), "{q}");
    }
    #[cfg(unix)]
    for q in ["Escape.md", "Escape", "Outdir/secret.md"] {
        match s.lookup(q) {
            NoteLookup::Refused(PathError::LeavesVault) => {}
            other => panic!("{q}: {other:?}"),
        }
    }
}

#[test]
fn hidden_and_escaping_files_are_not_notes() {
    let (_d, v) = fixture();
    let s = v.snapshot().unwrap();
    let paths: Vec<&str> = s.notes().iter().map(|n| n.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["Alpha.md", "Archive/Gamma.md", "Home.md", "Lonely.md", "Projects/Beta.md", "Projects/Gamma.md"]
    );
}

#[test]
fn links_resolve_like_obsidian() {
    let (_d, v) = fixture();
    let s = v.snapshot().unwrap();
    let home = s.notes().iter().find(|n| n.name() == "Home").unwrap();
    let out: Vec<(String, Option<String>)> = s
        .outgoing(home)
        .iter()
        .map(|r| (r.link.target.clone(), r.to.map(|p| p.to_string())))
        .collect();
    assert_eq!(
        out,
        vec![
            ("Alpha".into(), Some("Alpha.md".into())),
            ("Projects/Beta".into(), Some("Projects/Beta.md".into())),
            ("diagram.png".into(), Some("assets/diagram.png".into())),
            ("Nowhere".into(), None),
            ("Projects/Beta.md".into(), Some("Projects/Beta.md".into())),
        ]
    );

    let beta = s.notes().iter().find(|n| n.name() == "Beta").unwrap();
    let out: Vec<Option<String>> = s.outgoing(beta).iter().map(|r| r.to.map(|p| p.to_string())).collect();
    // [[Gamma]] prefers the note in the linking note's folder; ../Alpha.md is relative.
    assert_eq!(out, vec![Some("Projects/Gamma.md".into()), None, Some("Alpha.md".into())]);

    // An alias does not resolve a link.
    let gamma = s.notes().iter().find(|n| n.path.as_str() == "Projects/Gamma.md").unwrap();
    assert_eq!(s.outgoing(gamma)[0].to, None);

    let alpha = s.notes().iter().find(|n| n.name() == "Alpha").unwrap();
    let inc: Vec<String> = s.incoming(alpha).iter().map(|r| format!("{}:{}", r.from, r.link.line)).collect();
    assert_eq!(inc, vec!["Home.md:2", "Projects/Beta.md:5"]);
}

#[test]
fn broken_and_unlinked() {
    let (_d, v) = fixture();
    let s = v.snapshot().unwrap();
    let broken: Vec<String> = s
        .broken_links()
        .iter()
        .map(|r| format!("{}>{}:{}", r.from, r.link.target, r.targets_attachment()))
        .collect();
    assert_eq!(
        broken,
        vec!["Home.md>Nowhere:false", "Projects/Beta.md>Missing.pdf:true", "Projects/Gamma.md>First Letter:false"]
    );
    let unlinked: Vec<&str> = s.unlinked_notes().iter().map(|n| n.path.as_str()).collect();
    // Lonely only embeds an image; Archive/Gamma loses the name collision.
    assert_eq!(unlinked, vec!["Archive/Gamma.md", "Lonely.md"]);
}
