//! Notes unchanged since an earlier read are not parsed again, and a
//! changed note is always read afresh.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use vault::{Vault, VaultPath};

/// Writes a note last changed an hour ago, as most notes in a real vault.
fn write_old(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, text).unwrap();
    set_modified(&p, SystemTime::now() - Duration::from_secs(3600));
}

fn set_modified(p: &Path, t: SystemTime) {
    fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn text_of(v: &Vault, rel: &str) -> String {
    let snap = v.snapshot().unwrap();
    let path = VaultPath::parse(rel).unwrap();
    snap.note(&path).unwrap().text.clone()
}

#[test]
fn a_big_vault_is_parsed_once() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..2000 {
        write_old(
            dir.path(),
            &format!("F{}/Note {i}.md", i % 20),
            &format!(
                "# Note {i}\nSee [[Note {}]] and #tag{}.\n",
                (i + 1) % 2000,
                i % 9
            ),
        );
    }
    let v = Vault::open(dir.path()).unwrap();
    assert_eq!(v.snapshot().unwrap().notes().len(), 2000);
    assert_eq!(v.last_parsed(), 2000);
    let again = v.snapshot().unwrap();
    assert_eq!(v.last_parsed(), 0, "unchanged notes come from the cache");
    assert_eq!(again.notes().len(), 2000);
    assert_eq!(again.broken_links().len(), 0);
    // A clone of the vault (as each tool call holds) shares the cache.
    let _ = v.clone().snapshot().unwrap();
    assert_eq!(v.last_parsed(), 0);
}

#[test]
fn a_changed_note_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write_old(r, "A.md", "first\n");
    write_old(r, "B.md", "bee\n");
    let v = Vault::open(r).unwrap();
    v.snapshot().unwrap();
    v.snapshot().unwrap();
    assert_eq!(v.last_parsed(), 0);

    // Same size, same modification time, rewritten in place: the change
    // time still differs, so it is read again.
    let a = r.join("A.md");
    let was = fs::metadata(&a).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&a, "fresh\n").unwrap();
    set_modified(&a, was);
    assert_eq!(text_of(&v, "A.md"), "fresh\n");
    assert_eq!(v.last_parsed(), 1);

    // Replaced by a rename, as Obsidian and this server write.
    write_old(r, "B.tmp", "bees\n");
    fs::rename(r.join("B.tmp"), r.join("B.md")).unwrap();
    assert_eq!(text_of(&v, "B.md"), "bees\n");

    // Deleted notes are gone.
    fs::remove_file(r.join("B.md")).unwrap();
    assert!(
        v.snapshot()
            .unwrap()
            .note(&VaultPath::parse("B.md").unwrap())
            .is_none()
    );
}

#[test]
fn a_note_changed_just_now_is_never_kept() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("New.md"), "new\n").unwrap();
    let v = Vault::open(dir.path()).unwrap();
    v.snapshot().unwrap();
    v.snapshot().unwrap();
    assert_eq!(
        v.last_parsed(),
        1,
        "a note changed within 2 s is read every time"
    );
}
