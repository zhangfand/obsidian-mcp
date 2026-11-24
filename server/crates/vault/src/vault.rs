use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::note::{Link, LinkKind, Note};
use crate::path::{PathError, VaultPath};

/// A folder of notes, as Obsidian opens it.
#[derive(Clone, Debug)]
pub struct Vault {
    pub(crate) root: PathBuf,
    /// Where new file contents are written before being renamed into place.
    pub(crate) tmp_dir: PathBuf,
}

/// Every visible file of the vault, read at one moment. Answers about
/// names, links and lookups come from this, never from a stored index.
#[derive(Debug)]
pub struct Snapshot {
    root: PathBuf,
    notes: Vec<Note>,
    /// Every visible file, notes included, sorted by path.
    files: Vec<VaultPath>,
    by_path: HashMap<String, usize>,
    by_file_name: HashMap<String, Vec<usize>>,
    note_index: HashMap<VaultPath, usize>,
}

/// The outcome of naming a note by path, name or alias.
#[derive(Debug)]
pub enum NoteLookup<'a> {
    Found(&'a Note),
    Ambiguous(Vec<&'a Note>),
    /// The closest note names, closest first.
    NotFound(Vec<String>),
    Refused(PathError),
}

/// A link occurrence together with where it leads.
#[derive(Clone, Debug)]
pub struct ResolvedLink<'a> {
    pub from: &'a VaultPath,
    pub link: &'a Link,
    /// The file it resolves to, or `None` when broken or ambiguous.
    pub to: Option<&'a VaultPath>,
    /// The candidates, sorted, when the target matches several files.
    pub ambiguous: Vec<&'a VaultPath>,
}

/// Where a link leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution<'a> {
    File(&'a VaultPath),
    /// The name or path suffix matches several files; none is picked.
    Ambiguous(Vec<&'a VaultPath>),
    Missing,
}

impl<'a> ResolvedLink<'a> {
    fn new(from: &'a VaultPath, link: &'a Link, r: Resolution<'a>) -> ResolvedLink<'a> {
        match r {
            Resolution::File(p) => ResolvedLink {
                from,
                link,
                to: Some(p),
                ambiguous: Vec::new(),
            },
            Resolution::Ambiguous(c) => ResolvedLink {
                from,
                link,
                to: None,
                ambiguous: c,
            },
            Resolution::Missing => ResolvedLink {
                from,
                link,
                to: None,
                ambiguous: Vec::new(),
            },
        }
    }
}

impl ResolvedLink<'_> {
    /// True when the target names a non-note file (`img.png`), so a broken
    /// link is a missing file rather than a missing note.
    pub fn targets_attachment(&self) -> bool {
        attachment_extension(&self.link.target).is_some()
    }
}

impl Vault {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Vault> {
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "the vault is not a folder",
            ));
        }
        let tmp_dir = Vault::default_tmp_dir(&root);
        Ok(Vault { root, tmp_dir })
    }

    /// Uses `dir` for temporary files instead of the default hidden sibling
    /// of the vault folder. It must be outside the vault.
    pub fn with_tmp_dir(mut self, dir: impl AsRef<Path>) -> io::Result<Vault> {
        fs::create_dir_all(dir.as_ref())?;
        let dir = fs::canonicalize(dir)?;
        if dir.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the temporary folder must be outside the vault",
            ));
        }
        self.tmp_dir = dir;
        Ok(self)
    }

    pub fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reads every visible file. Dot-folders are skipped; symlinked folders
    /// are not followed; symlinked files count only when they stay inside
    /// the vault.
    pub fn snapshot(&self) -> io::Result<Snapshot> {
        let mut files = Vec::new();
        walk(&self.root, &self.root, &mut files)?;
        files.sort();
        let note_paths: Vec<&VaultPath> = files.iter().filter(|p| p.is_note()).collect();
        let notes = read_notes(&self.root, &note_paths)?;
        Ok(Snapshot::build(self.root.clone(), files, notes))
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<VaultPath>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        if ft.is_symlink() {
            match fs::canonicalize(&path) {
                Ok(real) if real.starts_with(root) && real.is_file() => {}
                _ => continue,
            }
        } else if !ft.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .expect("walked path is under the root");
        if let Some(rel) = rel.to_str()
            && let Ok(vp) = VaultPath::parse(rel)
        {
            out.push(vp);
        }
    }
    Ok(())
}

fn read_notes(root: &Path, paths: &[&VaultPath]) -> io::Result<Vec<Note>> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    let chunk = paths.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .filter_map(|p| match fs::read(root.join(p.as_str())) {
                            Ok(bytes) => {
                                let text = String::from_utf8(bytes).unwrap_or_else(|e| {
                                    String::from_utf8_lossy(e.as_bytes()).into_owned()
                                });
                                Some(Ok(Note::parse((*p).clone(), text)))
                            }
                            // Deleted between listing and reading.
                            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                            Err(e) => Some(Err(e)),
                        })
                        .collect::<io::Result<Vec<Note>>>()
                })
            })
            .collect();
        let mut notes = Vec::with_capacity(paths.len());
        for h in handles {
            notes.extend(h.join().expect("note reader thread panicked")?);
        }
        Ok(notes)
    })
}

/// The extension of a link target when it names a non-note file: a short
/// alphanumeric suffix after the last `.` of the last path part, other than
/// `md`. `v1.2 notes` has none.
fn attachment_extension(target: &str) -> Option<&str> {
    let last = target.rsplit('/').next().unwrap_or(target);
    let (_, ext) = last.rsplit_once('.')?;
    let ok = !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric());
    (ok && !ext.eq_ignore_ascii_case("md")).then_some(ext)
}

impl Snapshot {
    fn build(root: PathBuf, files: Vec<VaultPath>, notes: Vec<Note>) -> Snapshot {
        let mut by_path = HashMap::new();
        let mut by_file_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            by_path.insert(f.as_str().to_lowercase(), i);
            by_file_name
                .entry(f.file_name().to_lowercase())
                .or_default()
                .push(i);
        }
        let note_index = notes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.path.clone(), i))
            .collect();
        Snapshot {
            root,
            notes,
            files,
            by_path,
            by_file_name,
            note_index,
        }
    }

    /// This vault as it would be with the file `rename.0` at `rename.1`,
    /// with `added` as a new note, and with `replaced` notes (given at their
    /// new paths) in place of the read ones.
    pub(crate) fn changed(
        &self,
        rename: Option<(&VaultPath, &VaultPath)>,
        added: Option<&Note>,
        replaced: &[Note],
    ) -> Snapshot {
        let map = |p: &VaultPath| match rename {
            Some((from, to)) if p == from => to.clone(),
            _ => p.clone(),
        };
        let mut files: Vec<VaultPath> = self.files.iter().map(map).collect();
        let mut notes: Vec<Note> = self
            .notes
            .iter()
            .map(|n| {
                let p = map(&n.path);
                replaced
                    .iter()
                    .find(|r| r.path == p)
                    .cloned()
                    .unwrap_or_else(|| Note {
                        path: p,
                        ..n.clone()
                    })
            })
            .collect();
        if let Some(new) = added {
            files.push(new.path.clone());
            notes.push(new.clone());
        }
        files.sort();
        notes.sort_by(|a, b| a.path.cmp(&b.path));
        Snapshot::build(self.root.clone(), files, notes)
    }

    /// Notes sorted by path.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub fn files(&self) -> &[VaultPath] {
        &self.files
    }

    pub fn note(&self, path: &VaultPath) -> Option<&Note> {
        self.note_index.get(path).map(|&i| &self.notes[i])
    }

    /// Finds a note by vault-relative path (with or without `.md`), then by
    /// name or path suffix, then by alias; case-insensitive. The first step
    /// with any match decides.
    pub fn lookup(&self, query: &str) -> NoteLookup<'_> {
        let path = match self.safe_path(query) {
            Ok(p) => p,
            Err(e) => return NoteLookup::Refused(e),
        };
        let q = path.as_str().to_lowercase();
        let with_md = if q.ends_with(".md") {
            q.clone()
        } else {
            format!("{q}.md")
        };

        // A bare name (no folder, no `.md`) that several notes have is
        // ambiguous even when one of them is at the vault root.
        let bare = !q.contains('/') && !q.ends_with(".md");
        if bare {
            let named: Vec<&Note> = self
                .notes
                .iter()
                .filter(|n| n.name().to_lowercase() == q)
                .collect();
            if named.len() > 1 {
                return decide(named);
            }
        }
        let exact: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| n.path.as_str().to_lowercase() == with_md)
            .collect();
        if !exact.is_empty() {
            return decide(exact);
        }
        let suffix = format!("/{with_md}");
        let by_name: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| n.path.as_str().to_lowercase().ends_with(&suffix))
            .collect();
        if !by_name.is_empty() {
            return decide(by_name);
        }
        let wanted = query.trim().to_lowercase();
        let by_alias: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| n.aliases.iter().any(|a| a.0.to_lowercase() == wanted))
            .collect();
        if !by_alias.is_empty() {
            return decide(by_alias);
        }
        NoteLookup::NotFound(self.closest_names(query, 5))
    }

    /// Refuses a path when any part of it that exists on disk is a symlink
    /// leading outside the vault. Every tool argument that names a path
    /// goes through this, after `VaultPath::parse`.
    pub fn check_path(&self, path: &VaultPath) -> Result<(), PathError> {
        let mut prefixes: Vec<String> = Vec::new();
        let mut acc = String::new();
        for part in path.as_str().split('/') {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            prefixes.push(acc.clone());
        }
        if !path.is_note() {
            prefixes.push(format!("{}.md", path.as_str()));
        }
        for rel in prefixes {
            let full = self.root.join(&rel);
            if fs::symlink_metadata(&full).is_err() {
                continue;
            }
            match fs::canonicalize(&full) {
                Ok(real) if real.starts_with(&self.root) => {}
                _ => return Err(PathError::LeavesVault),
            }
        }
        Ok(())
    }

    /// Parses a path argument and refuses it if it would leave the vault.
    pub fn safe_path(&self, raw: &str) -> Result<VaultPath, PathError> {
        let path = VaultPath::parse(raw)?;
        self.check_path(&path)?;
        Ok(path)
    }

    /// Note names (or aliases) closest to `query` by edit distance; each
    /// note appears once, by its name.
    pub fn closest_names(&self, query: &str, limit: usize) -> Vec<String> {
        // Edit distance is quadratic; nobody types a 200-character name.
        let q: String = query.trim().to_lowercase().chars().take(200).collect();
        let q = q.strip_suffix(".md").unwrap_or(&q).to_string();
        let q_name = q.rsplit('/').next().unwrap_or(&q).to_string();
        let mut scored: Vec<(usize, &str)> = self
            .notes
            .iter()
            .map(|n| {
                let name = n.name().to_lowercase();
                let mut best = name_distance(&q_name, &name)
                    .min(name_distance(&q, &n.path.as_str().to_lowercase()));
                for a in &n.aliases {
                    best = best.min(name_distance(&q_name, &a.0.to_lowercase()));
                }
                (best, n.name())
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let mut out: Vec<String> = Vec::new();
        for (_, name) in scored {
            if out.len() == limit {
                break;
            }
            if !out.iter().any(|o| o == name) {
                out.push(name.to_string());
            }
        }
        out
    }

    /// Where a link in note `from` leads: the note itself for an empty
    /// target; else the target as a vault path, then (Markdown links)
    /// relative to the linking note's folder, either of which wins
    /// outright; then as a file name or path suffix anywhere, which must
    /// match exactly one file, or the link is ambiguous. Aliases never
    /// resolve links.
    pub fn resolution(&self, link: &Link, from: &VaultPath) -> Resolution<'_> {
        if link.target.is_empty() {
            return self
                .note(from)
                .map_or(Resolution::Missing, |n| Resolution::File(&n.path));
        }
        let target = link.target.replace('\\', "/").to_lowercase();
        let target = target.trim_start_matches('/');
        let candidates: Vec<String> =
            if attachment_extension(target).is_some() || target.ends_with(".md") {
                vec![target.to_string()]
            } else {
                vec![format!("{target}.md"), target.to_string()]
            };
        for cand in &candidates {
            if let Some(i) = self.by_exact(cand) {
                return Resolution::File(&self.files[i]);
            }
            let relative = link.relative || cand.starts_with("./") || cand.starts_with("../");
            if relative
                && let Some(joined) = join_relative(from.folder(), cand)
                && let Some(i) = self.by_exact(&joined)
            {
                return Resolution::File(&self.files[i]);
            }
        }
        for cand in &candidates {
            let found = self.by_suffix(cand);
            match found.len() {
                0 => continue,
                1 => return Resolution::File(&self.files[found[0]]),
                _ => {
                    let mut paths: Vec<&VaultPath> =
                        found.iter().map(|&i| &self.files[i]).collect();
                    paths.sort();
                    return Resolution::Ambiguous(paths);
                }
            }
        }
        Resolution::Missing
    }

    /// The one file a link leads to; `None` when broken or ambiguous.
    pub fn resolve(&self, link: &Link, from: &VaultPath) -> Option<&VaultPath> {
        match self.resolution(link, from) {
            Resolution::File(p) => Some(p),
            _ => None,
        }
    }

    fn by_exact(&self, lower_path: &str) -> Option<usize> {
        let clean = VaultPath::parse(lower_path).ok()?;
        self.by_path.get(clean.as_str()).copied()
    }

    /// Files whose path is `lower` or ends with `/lower`.
    fn by_suffix(&self, lower: &str) -> Vec<usize> {
        let lower = lower.trim_start_matches("./");
        let Some(file_name) = lower.rsplit('/').next() else {
            return Vec::new();
        };
        let Some(pool) = self.by_file_name.get(file_name) else {
            return Vec::new();
        };
        let suffix = format!("/{lower}");
        pool.iter()
            .copied()
            .filter(|&i| {
                let p = self.files[i].as_str().to_lowercase();
                p == lower || p.ends_with(&suffix)
            })
            .collect()
    }

    /// Every link of every note with where it leads, notes in path order,
    /// links in file order.
    pub fn all_links(&self) -> Vec<ResolvedLink<'_>> {
        self.notes
            .iter()
            .flat_map(|n| {
                n.links
                    .iter()
                    .map(move |l| ResolvedLink::new(&n.path, l, self.resolution(l, &n.path)))
            })
            .collect()
    }

    /// The links written in `note`, in file order.
    pub fn outgoing<'a>(&'a self, note: &'a Note) -> Vec<ResolvedLink<'a>> {
        note.links
            .iter()
            .map(|l| ResolvedLink::new(&note.path, l, self.resolution(l, &note.path)))
            .collect()
    }

    /// The links in other notes that lead to `note`, by source path then
    /// line.
    pub fn incoming(&self, note: &Note) -> Vec<ResolvedLink<'_>> {
        self.all_links()
            .into_iter()
            .filter(|r| r.from != &note.path && r.to == Some(&note.path))
            .collect()
    }

    /// Links that lead nowhere.
    pub fn broken_links(&self) -> Vec<ResolvedLink<'_>> {
        self.all_links()
            .into_iter()
            .filter(|r| r.to.is_none())
            .collect()
    }
}

impl LinkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkKind::Wikilink => "wikilink",
            LinkKind::Embed => "embed",
            LinkKind::Markdown => "markdown",
        }
    }
}

fn decide(mut found: Vec<&Note>) -> NoteLookup<'_> {
    if found.len() == 1 {
        NoteLookup::Found(found.remove(0))
    } else {
        found.sort_by(|a, b| a.path.cmp(&b.path));
        NoteLookup::Ambiguous(found)
    }
}

/// Joins a relative target to a folder, resolving `.` and `..`; `None` if
/// it climbs above the vault root.
fn join_relative(folder: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/").to_lowercase())
}

fn name_distance(a: &str, b: &str) -> usize {
    if b.contains(a) && !a.is_empty() {
        // A typed fragment of a longer name is close, not far.
        return (b.chars().count() - a.chars().count()) / 4;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}
