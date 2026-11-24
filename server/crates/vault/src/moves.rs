//! Moving or adding a note without changing where any existing link points.

use std::ops::Range;

use crate::edit::splice;
use crate::note::{Link, LinkKind, Note};
use crate::path::VaultPath;
use crate::vault::Snapshot;

/// Everything a move writes, worked out before anything is written.
#[derive(Clone, Debug)]
pub struct MovePlan {
    pub from: VaultPath,
    pub to: VaultPath,
    /// The moved note's new text when its own links had to change.
    pub note_text: Option<String>,
    /// Other notes whose links change, by their path (unchanged by the move).
    pub rewrites: Vec<FileRewrite>,
}

#[derive(Clone, Debug)]
pub struct FileRewrite {
    pub path: VaultPath,
    pub old_text: String,
    pub new_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveError {
    /// A link would need rewriting but its target is not written in place
    /// (a reference-style Markdown link).
    CannotRewrite { from: VaultPath, line: usize },
    /// After rewriting, a link would not point where it did. Nothing is
    /// written.
    WouldBreak { from: VaultPath, line: usize },
}

impl Snapshot {
    /// Works out the move of `note` to `to`: every link that would point
    /// elsewhere after the move (links to the note, links in it, and links
    /// to files whose name it would capture) gets the shortest target that
    /// still reaches its old file. Links that already do, and broken links,
    /// stay as written.
    pub fn plan_move(&self, note: &Note, to: &VaultPath) -> Result<MovePlan, MoveError> {
        let from = note.path.clone();
        let map = |p: &VaultPath| if *p == from { to.clone() } else { p.clone() };
        let after = self.changed(Some((&from, to)), None, &[]);
        let new_texts = self.keep_links(&after, &map, |r| self.changed(Some((&from, to)), None, r))?;
        let mut note_text = None;
        let mut rewrites = Vec::new();
        for (path, new_text) in new_texts {
            if path == from {
                note_text = Some(new_text);
            } else {
                rewrites.push(self.rewrite_of(path, new_text));
            }
        }
        Ok(MovePlan { from, to: to.clone(), note_text, rewrites })
    }

    /// Works out the links to rewrite when a note `new` is added: those
    /// whose name the new note would capture keep their old file.
    pub fn plan_create(&self, new: &Note) -> Result<Vec<FileRewrite>, MoveError> {
        let same = |p: &VaultPath| p.clone();
        let after = self.changed(None, Some(new), &[]);
        let new_texts = self.keep_links(&after, &same, |r| self.changed(None, Some(new), r))?;
        Ok(new_texts.into_iter().map(|(p, t)| self.rewrite_of(p, t)).collect())
    }

    fn rewrite_of(&self, path: VaultPath, new_text: String) -> FileRewrite {
        let old_text = self.note(&path).expect("planned from this snapshot").text.clone();
        FileRewrite { path, old_text, new_text }
    }

    /// For every note of this snapshot (at `map`ped paths in `after`), the
    /// new text that keeps each of its resolved links on the same file;
    /// only notes that change are returned, by their path here. `check`
    /// builds the final vault from the rewritten notes, in which every link
    /// is checked again.
    fn keep_links(
        &self,
        after: &Snapshot,
        map: &dyn Fn(&VaultPath) -> VaultPath,
        check: impl Fn(&[Note]) -> Snapshot,
    ) -> Result<Vec<(VaultPath, String)>, MoveError> {
        let before: Vec<Vec<Option<VaultPath>>> = self
            .notes()
            .iter()
            .map(|n| n.links.iter().map(|l| self.resolve(l, &n.path).map(map)).collect())
            .collect();

        let mut new_texts: Vec<(VaultPath, String)> = Vec::new();
        for (n, expected) in self.notes().iter().zip(&before) {
            let here = map(&n.path);
            let mut edits: Vec<(Range<usize>, String)> = Vec::new();
            for (l, want) in n.links.iter().zip(expected) {
                let Some(want) = want else { continue };
                if after.resolve(l, &here) == Some(want) {
                    continue;
                }
                let span = l.span.clone().ok_or(MoveError::CannotRewrite { from: n.path.clone(), line: l.line })?;
                let target = shortest_target(after, l, &here, want)
                    .ok_or(MoveError::WouldBreak { from: n.path.clone(), line: l.line })?;
                edits.push((span, written_target(l, &target)));
            }
            if !edits.is_empty() {
                new_texts.push((n.path.clone(), splice(&n.text, &edits)));
            }
        }

        // Read the rewritten files back and check every link once more.
        let reparsed: Vec<Note> = new_texts.iter().map(|(p, t)| Note::parse(map(p), t.clone())).collect();
        let final_vault = check(&reparsed);
        for (n, expected) in self.notes().iter().zip(&before) {
            let here = map(&n.path);
            let now = final_vault.note(&here).expect("every note survives the change");
            if now.links.len() != n.links.len() {
                return Err(MoveError::WouldBreak { from: n.path.clone(), line: 0 });
            }
            for (l, want) in now.links.iter().zip(expected) {
                if let Some(want) = want
                    && final_vault.resolve(l, &here) != Some(want)
                {
                    return Err(MoveError::WouldBreak { from: n.path.clone(), line: l.line });
                }
            }
        }
        Ok(new_texts)
    }
}

/// The shortest target text that, written in `link`'s form in the note at
/// `here`, resolves to `want` alone: the file name, then with more parent
/// folders. A shared name is ambiguous, so it never qualifies.
fn shortest_target(after: &Snapshot, link: &Link, here: &VaultPath, want: &VaultPath) -> Option<String> {
    let keep_md = link.kind == LinkKind::Markdown || link.target.to_lowercase().ends_with(".md");
    let full = if want.is_note() && !keep_md {
        want.as_str()[..want.as_str().len() - 3].to_string()
    } else {
        want.as_str().to_string()
    };
    let parts: Vec<&str> = full.split('/').collect();
    (0..parts.len()).rev().map(|k| parts[k..].join("/")).find(|cand| {
        let probe = Link { target: cand.clone(), ..link.clone() };
        after.resolve(&probe, here) == Some(want)
    })
}

/// A target as it is written in the link's form: Markdown destinations
/// outside `<...>` have spaces percent-encoded.
fn written_target(link: &Link, target: &str) -> String {
    if link.kind != LinkKind::Wikilink && link.relative && !link.angle {
        target.replace('%', "%25").replace(' ', "%20")
    } else {
        target.to_string()
    }
}
