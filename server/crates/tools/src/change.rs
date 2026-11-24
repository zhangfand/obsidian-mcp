//! The five change tools. The vault works out which bytes change; this
//! module checks versions, picks trash names, writes, and words the result.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vault::{
    MoveError, Note, Property, PropertyEditError, SectionEdit, Snapshot, Vault, VaultPath,
    WriteError, new_note_text,
};

use crate::error::{ErrorKind, ToolError, found, quote};
use crate::paging::size;
use crate::read::{Header, Target, pick_target};
use crate::views::{self, LinkView, fit_list};

/// The most bytes a list in a change result may take.
const LIST_LIMIT: usize = 6000;

fn stale(note: &Note) -> ToolError {
    ToolError::new(
        ErrorKind::Stale,
        format!(
            "{} changed since you read it. Read it again with read_note and make the change on what it holds now; `version` is its current version.",
            note.path
        ),
    )
    .with("version", json!(views::version(note)))
}

fn check_version(note: &Note, given: Option<&str>) -> Result<(), ToolError> {
    match given {
        Some(v) if v.trim() != views::version(note) => Err(stale(note)),
        _ => Ok(()),
    }
}

fn required_version(note: &Note, given: &str) -> Result<(), ToolError> {
    if given.trim().is_empty() {
        return Err(ToolError::invalid(
            "`version` is empty. Pass the version that read_note gave for this note.",
        ));
    }
    check_version(note, Some(given))
}

/// A refused write, worded for the agent.
fn write_error(vault: &Vault, path: &VaultPath, e: WriteError) -> ToolError {
    match e {
        WriteError::Changed => {
            // Re-read so the agent gets the version that is there now.
            let mut err = ToolError::new(
                ErrorKind::Stale,
                format!(
                    "{path} changed while the change was being made; nothing was written to it. Read it again with read_note and retry."
                ),
            );
            if let Ok(bytes) = vault.read_file(path) {
                err = err.with(
                    "version",
                    json!(views::version_of(&String::from_utf8_lossy(&bytes))),
                );
            }
            err
        }
        WriteError::Exists => ToolError::new(
            ErrorKind::AlreadyExists,
            format!("{path} already exists. Choose another path."),
        )
        .with("path", json!(path.as_str())),
        WriteError::Io(e) => ToolError::write_io(e),
    }
}

/// Refuses a change to the text of a note that is not valid UTF-8: writing
/// its text back would replace its invalid bytes too.
fn writable(note: &Note) -> Result<(), ToolError> {
    if note.is_utf8() {
        Ok(())
    } else {
        Err(ToolError::invalid(format!(
            "{} is not valid UTF-8 text, so its text cannot be changed without also changing its unreadable bytes. Fix the file's encoding by hand first; it can still be moved or deleted.",
            note.path
        )))
    }
}

fn md_path(snap: &Snapshot, raw: &str) -> Result<VaultPath, ToolError> {
    let p = snap
        .safe_path(raw)
        .map_err(|e| ToolError::outside(raw, e))?;
    if p.is_note() {
        return Ok(p);
    }
    let with_md = format!("{}.md", p.as_str());
    snap.safe_path(&with_md)
        .map_err(|e| ToolError::outside(raw, e))
}

/// An existing file, other than `except`, whose path equals `path`
/// ignoring case.
fn taken<'a>(
    snap: &'a Snapshot,
    path: &VaultPath,
    except: Option<&VaultPath>,
) -> Option<&'a VaultPath> {
    let lower = path.as_str().to_lowercase();
    snap.files()
        .iter()
        .find(|f| Some(*f) != except && f.as_str().to_lowercase() == lower)
}

fn exists_error(existing: &VaultPath, why: &str) -> ToolError {
    ToolError::new(ErrorKind::AlreadyExists, format!("{} {why}", existing))
        .with("path", json!(existing.as_str()))
}

// ---- create_note ----

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateNoteArgs {
    pub path: String,
    #[serde(default)]
    pub content: String,
    pub properties: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CreateNoteResult {
    pub path: String,
    pub name: String,
    pub version: String,
    pub updated_notes: Vec<String>,
    pub total_updated_notes: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub same_name: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
}

/// Other notes named like `path`, leaving out `except` (the note moved).
fn same_name(snap: &Snapshot, path: &VaultPath, except: Option<&VaultPath>) -> Vec<String> {
    let name = path.stem().to_lowercase();
    let all: Vec<String> = snap
        .notes()
        .iter()
        .filter(|n| Some(&n.path) != except && n.path != *path && n.name().to_lowercase() == name)
        .map(|n| n.path.to_string())
        .collect();
    fit_list(all, 4096, 1500).0
}

fn path_list(paths: Vec<String>) -> (Vec<String>, usize, Vec<&'static str>) {
    let total = paths.len();
    let (list, cut) = fit_list(paths, 4096, LIST_LIMIT);
    (
        list,
        total,
        if cut {
            vec!["updated_notes"]
        } else {
            Vec::new()
        },
    )
}

fn plan_error(e: MoveError, doing: &str) -> ToolError {
    match e {
        MoveError::CannotRewrite { from, line } => ToolError::invalid(format!(
            "{doing} would need to rewrite a reference-style link in {from} (line {line}), which cannot be changed in place. Nothing was written."
        )),
        MoveError::WouldBreak { from, line } => ToolError::invalid(format!(
            "{doing} would leave a link in {from} (line {line}) pointing elsewhere. Nothing was written."
        )),
        MoveError::NotUtf8 { from } => ToolError::invalid(format!(
            "{doing} would need to rewrite a link in {from}, which is not valid UTF-8 text and cannot be rewritten without changing its unreadable bytes. Nothing was written."
        )),
    }
}

/// Checks every planned file is unchanged, then rewrites them one by one.
/// Returns the rewrites done. If one fails, those already done are put
/// back before the error is returned.
fn rewrite_all<'a>(
    vault: &Vault,
    rewrites: &'a [vault::FileRewrite],
) -> Result<Vec<&'a vault::FileRewrite>, ToolError> {
    for r in rewrites {
        match vault.read_file(&r.path) {
            Ok(bytes) if bytes == r.old_text.as_bytes() => {}
            Ok(_) => return Err(write_error(vault, &r.path, WriteError::Changed)),
            Err(e) => return Err(ToolError::io(e)),
        }
    }
    let mut done = Vec::new();
    for r in rewrites {
        if let Err(e) = vault.replace_file(&r.path, r.old_text.as_bytes(), r.new_text.as_bytes()) {
            return Err(undone(vault, write_error(vault, &r.path, e), &done));
        }
        done.push(r);
    }
    Ok(done)
}

/// Puts back the notes in `done` (rewritten by this change) and adds to
/// `err` what became of them.
fn undone(vault: &Vault, mut err: ToolError, done: &[&vault::FileRewrite]) -> ToolError {
    if done.is_empty() {
        return err;
    }
    let not_restored: Vec<String> = done
        .iter()
        .rev()
        .filter(|r| {
            vault
                .replace_file(&r.path, r.new_text.as_bytes(), r.old_text.as_bytes())
                .is_err()
        })
        .map(|r| r.path.to_string())
        .collect();
    if not_restored.is_empty() {
        err.message.push_str(&format!(
            " The links already updated in {} other notes were put back, so nothing changed.",
            done.len()
        ));
    } else {
        let total = not_restored.len();
        let (list, cut) = fit_list(not_restored, 512, 2000);
        err.message.push_str(&format!(
            " {total} other notes whose links were already updated could not be put back because they changed meanwhile: {}{}. Check their links with check_vault.",
            list.join(", "),
            if cut { ", ..." } else { "" }
        ));
    }
    err
}

pub fn create_note(
    vault: &Vault,
    snap: &Snapshot,
    args: &CreateNoteArgs,
) -> Result<CreateNoteResult, ToolError> {
    let path = md_path(snap, &args.path)?;
    if let Some(existing) = taken(snap, &path, None) {
        return Err(exists_error(
            existing,
            "already exists. Choose another path.",
        ));
    }
    let properties: Vec<Property> = match &args.properties {
        None => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| Property {
                name: k.clone(),
                value: views::from_json(v),
            })
            .collect(),
        Some(_) => {
            return Err(ToolError::invalid(
                "`properties` must be an object of property names and values.",
            ));
        }
    };
    let starts_with_block = args
        .content
        .lines()
        .next()
        .is_some_and(|l| l.trim_end() == "---");
    if !properties.is_empty() && starts_with_block {
        return Err(ToolError::invalid(
            "`content` starts with its own properties block and `properties` is given too. Pass properties in only one of them.",
        ));
    }
    let text = new_note_text(&properties, &args.content).ok_or_else(|| {
        ToolError::invalid(
            "A property name or value cannot be written as YAML so that it reads back unchanged.",
        )
    })?;
    let new_note = Note::parse(path.clone(), text.clone());
    let rewrites = snap
        .plan_create(&new_note)
        .map_err(|e| plan_error(e, "creating it"))?;
    vault
        .can_create(&path)
        .map_err(|e| write_error(vault, &path, e))?;
    let done = rewrite_all(vault, &rewrites)?;
    if let Err(e) = vault.create_file(&path, text.as_bytes()) {
        return Err(undone(vault, write_error(vault, &path, e), &done));
    }
    let updated = done.iter().map(|r| r.path.to_string()).collect();
    let (updated_notes, total_updated_notes, cut) = path_list(updated);
    Ok(CreateNoteResult {
        path: path.to_string(),
        name: path.stem().to_string(),
        version: views::version_of(&text),
        updated_notes,
        total_updated_notes,
        same_name: same_name(snap, &path, None),
        cut,
    })
}

// ---- edit_note ----

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EditMode {
    #[default]
    Replace,
    Append,
    Prepend,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EditNoteArgs {
    pub note: String,
    pub version: String,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub section: Option<String>,
    pub heading_line: Option<usize>,
    pub content: Option<String>,
    pub mode: Option<EditMode>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EditResult {
    pub path: String,
    pub version: String,
}

/// The most match lines listed in an `ambiguous_text` error.
const MATCH_LINES: usize = 20;

pub fn edit_note(
    vault: &Vault,
    snap: &Snapshot,
    args: &EditNoteArgs,
) -> Result<EditResult, ToolError> {
    let by_text = args.old_text.is_some() || args.new_text.is_some();
    let by_section = args.content.is_some() || args.mode.is_some();
    let named = args.section.is_some() || args.heading_line.is_some();
    if by_text == by_section || (by_section && !named) {
        return Err(ToolError::invalid(
            "Give exactly one way: `old_text` and `new_text` (by text), or `section` or `heading_line` with `content` and optionally `mode` (by section).",
        ));
    }
    let note = found(&args.note, snap.lookup(&args.note))?;
    let target = pick_target(note, args.section.as_deref(), args.heading_line)?;
    required_version(note, &args.version)?;
    writable(note)?;
    let new = if by_text {
        let (Some(old), Some(replacement)) = (&args.old_text, &args.new_text) else {
            return Err(ToolError::invalid(
                "An edit by text needs both `old_text` and `new_text` (`new_text` may be \"\").",
            ));
        };
        if old.is_empty() {
            return Err(ToolError::invalid(
                "`old_text` is empty. Pass the exact text to replace, as read_note shows it.",
            ));
        }
        let region = match &target {
            Some(Target::Section(s)) => s.range.clone(),
            Some(Target::Lead) => note.lead_range(),
            None => note.body_start..note.text.len(),
        };
        let matches = note.find_text(old, region);
        match matches.as_slice() {
            [one] => note.replace_text(one.clone(), replacement),
            [] => return Err(text_not_found(note, old, target.is_some())),
            many => {
                let lines: Vec<usize> = many
                    .iter()
                    .take(MATCH_LINES)
                    .map(|m| note.line_of(m.start))
                    .collect();
                return Err(ToolError::new(
                    ErrorKind::AmbiguousText,
                    format!(
                        "`old_text` occurs {} times in {}. Add surrounding text until it occurs once, or pass `section` to search one section.",
                        many.len(),
                        note.path
                    ),
                )
                .with("count", json!(many.len()))
                .with("lines", json!(lines)));
            }
        }
    } else {
        let target = target.expect("checked that a section is named");
        let edit = match args.mode.unwrap_or_default() {
            EditMode::Replace => SectionEdit::Replace,
            EditMode::Append => SectionEdit::Append,
            EditMode::Prepend => SectionEdit::Prepend,
        };
        note.edit_part(target.part(), edit, args.content.as_deref().unwrap_or(""))
    };
    if new != note.text {
        vault
            .replace_file(&note.path, note.text.as_bytes(), new.as_bytes())
            .map_err(|e| write_error(vault, &note.path, e))?;
    }
    Ok(EditResult {
        path: note.path.to_string(),
        version: views::version_of(&new),
    })
}

fn text_not_found(note: &Note, old: &str, limited: bool) -> ToolError {
    let in_properties = note
        .properties_block
        .clone()
        .is_some_and(|block| !note.find_text(old, block).is_empty());
    if in_properties {
        return ToolError::new(
            ErrorKind::TextNotFound,
            format!(
                "`old_text` occurs only in the properties block of {}, which is never edited by text. Change properties with set_property.",
                note.path
            ),
        )
        .with("in_properties", json!(true));
    }
    let place = if limited {
        format!("that part of {}", note.path)
    } else {
        note.path.to_string()
    };
    ToolError::new(
        ErrorKind::TextNotFound,
        format!(
            "`old_text` does not occur in {place}. Read the note again with read_note and copy the text exactly."
        ),
    )
}

// ---- set_property ----

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetPropertyArgs {
    pub note: String,
    pub name: String,
    pub value: Value,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SetPropertyResult {
    pub path: String,
    pub version: String,
    pub properties: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
}

pub fn set_property(
    vault: &Vault,
    snap: &Snapshot,
    args: &SetPropertyArgs,
) -> Result<SetPropertyResult, ToolError> {
    let note = found(&args.note, snap.lookup(&args.note))?;
    required_version(note, &args.version)?;
    writable(note)?;
    let value = (!args.value.is_null()).then(|| views::from_json(&args.value));
    let new = note.set_property(&args.name, value.as_ref()).map_err(|e| match e {
        PropertyEditError::InvalidBlock => ToolError::new(
            ErrorKind::InvalidProperties,
            format!("The properties block of {} is not valid YAML, so it was not rewritten. Fix the block by hand first.", note.path),
        ),
        PropertyEditError::BadName => {
            ToolError::invalid("`name` must be a non-empty property name on one line.")
        }
        PropertyEditError::NotRepresentable => ToolError::invalid(format!(
            "The value of {} cannot be written so that it reads back unchanged without touching other properties.",
            quote(&args.name)
        )),
    })?;
    if new != note.text {
        vault
            .replace_file(&note.path, note.text.as_bytes(), new.as_bytes())
            .map_err(|e| write_error(vault, &note.path, e))?;
    }
    let header = Header::of(&Note::parse(note.path.clone(), new.clone()));
    let cut = if header.cut.contains(&"properties") {
        vec!["properties"]
    } else {
        Vec::new()
    };
    Ok(SetPropertyResult {
        path: note.path.to_string(),
        version: views::version_of(&new),
        properties: header.properties,
        cut,
    })
}

// ---- move_note ----

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MoveNoteArgs {
    pub note: String,
    pub to: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MoveNoteResult {
    pub from: String,
    pub path: String,
    pub version: String,
    pub updated_notes: Vec<String>,
    pub total_updated_notes: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub same_name: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
}

pub fn move_note(
    vault: &Vault,
    snap: &Snapshot,
    args: &MoveNoteArgs,
) -> Result<MoveNoteResult, ToolError> {
    let note = found(&args.note, snap.lookup(&args.note))?;
    check_version(note, args.version.as_deref())?;
    let target = snap
        .safe_path(&args.to)
        .map_err(|e| ToolError::outside(&args.to, e))?;
    let folder_prefix = format!("{}/", target.as_str().to_lowercase());
    let is_folder = args.to.trim_end().ends_with(['/', '\\'])
        || snap
            .files()
            .iter()
            .any(|f| f.as_str().to_lowercase().starts_with(&folder_prefix));
    let to = if is_folder {
        snap.safe_path(&format!("{}/{}", target.as_str(), note.path.file_name()))
            .map_err(|e| ToolError::outside(&args.to, e))?
    } else {
        md_path(snap, &args.to)?
    };
    if to == note.path {
        return Err(ToolError::invalid(format!(
            "{} is already at that path.",
            note.path
        )));
    }
    if let Some(existing) = taken(snap, &to, Some(&note.path)) {
        return Err(exists_error(
            existing,
            "already exists. Choose another path.",
        ));
    }

    let plan = snap
        .plan_move(note, &to)
        .map_err(|e| plan_error(e, "The move"))?;
    // Check what can be checked before any linking note is rewritten.
    match vault.read_file(&note.path) {
        Ok(bytes) if bytes == note.bytes() => {}
        Ok(_) => return Err(write_error(vault, &note.path, WriteError::Changed)),
        Err(e) => return Err(ToolError::io(e)),
    }
    let case_only = to.as_str().to_lowercase() == note.path.as_str().to_lowercase();
    if !case_only {
        vault
            .can_create(&to)
            .map_err(|e| write_error(vault, &to, e))?;
    }
    let rewritten = rewrite_all(vault, &plan.rewrites)?;
    let moved = match &plan.note_text {
        Some(text) => vault
            .create_file(&to, text.as_bytes())
            .map_err(|e| write_error(vault, &to, e))
            .and_then(|()| {
                vault.remove_file(&note.path, note.bytes()).map_err(|e| {
                    // Take back the new copy so the note is in one place.
                    let _ = vault.remove_file(&to, text.as_bytes());
                    write_error(vault, &note.path, e)
                })
            })
            .map(|()| text.clone()),
        None => vault
            .move_file(&note.path, &to, note.bytes())
            .map_err(|e| write_error(vault, &note.path, e))
            .map(|()| note.text.clone()),
    };
    let final_text = moved.map_err(|e| undone(vault, e, &rewritten))?;
    let done: Vec<String> = rewritten.iter().map(|r| r.path.to_string()).collect();
    let (updated_notes, total_updated_notes, cut) = path_list(done);
    Ok(MoveNoteResult {
        from: note.path.to_string(),
        path: to.to_string(),
        version: views::version_of(&final_text),
        updated_notes,
        total_updated_notes,
        same_name: same_name(snap, &to, Some(&note.path)),
        cut,
    })
}

// ---- delete_note ----

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteNoteArgs {
    pub note: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeleteNoteResult {
    pub path: String,
    pub deleted: bool,
    pub trash_path: String,
    pub now_broken: Vec<LinkView>,
    pub total_now_broken: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
}

/// Names to try in `.trash/`: the file name, then `Name 1.md`, `Name 2.md`...
fn trash_candidates(file_name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, ext) = file_name
        .rsplit_once('.')
        .map_or((file_name, ""), |(s, e)| (s, e));
    std::iter::once(file_name.to_string()).chain((1..).map(move |n| {
        if ext.is_empty() {
            format!("{stem} {n}")
        } else {
            format!("{stem} {n}.{ext}")
        }
    }))
}

pub fn delete_note(
    vault: &Vault,
    snap: &Snapshot,
    args: &DeleteNoteArgs,
) -> Result<DeleteNoteResult, ToolError> {
    let note = found(&args.note, snap.lookup(&args.note))?;
    check_version(note, args.version.as_deref())?;
    let broken: Vec<LinkView> = snap.incoming(note).iter().map(LinkView::from).collect();
    let taken_names: Vec<String> = vault
        .trash_names()
        .map_err(ToolError::io)?
        .into_iter()
        .map(|n| n.to_lowercase())
        .collect();
    let mut trash_path = None;
    for name in trash_candidates(note.path.file_name()).take(10_000) {
        if taken_names.contains(&name.to_lowercase()) {
            continue;
        }
        match vault.move_to_trash(&note.path, &name, note.bytes()) {
            Ok(p) => {
                trash_path = Some(p);
                break;
            }
            // Taken since the listing: try the next number.
            Err(WriteError::Exists) => continue,
            Err(e) => return Err(write_error(vault, &note.path, e)),
        }
    }
    let trash_path = trash_path
        .ok_or_else(|| ToolError::write_io(std::io::Error::other("no free name in .trash")))?;
    let total = broken.len();
    let mut now_broken = Vec::new();
    let mut used = 0;
    for l in broken {
        used += size(&l) + 1;
        if used > LIST_LIMIT {
            break;
        }
        now_broken.push(l);
    }
    let cut = if now_broken.len() < total {
        vec!["now_broken"]
    } else {
        Vec::new()
    };
    Ok(DeleteNoteResult {
        path: note.path.to_string(),
        deleted: true,
        trash_path,
        now_broken,
        total_now_broken: total,
        cut,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use vault::{FileRewrite, VaultPath};

    fn rewrite(path: &str, old: &str, new: &str) -> FileRewrite {
        FileRewrite {
            path: VaultPath::parse(path).unwrap(),
            old_text: old.into(),
            new_text: new.into(),
        }
    }

    fn vault_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("vault");
        fs::create_dir_all(&root).unwrap();
        for (p, t) in files {
            fs::write(root.join(p), t).unwrap();
        }
        let v = Vault::open(&root)
            .unwrap()
            .with_tmp_dir(base.path().join("tmp"))
            .unwrap();
        (base, v)
    }

    #[test]
    fn a_rewrite_that_fails_part_way_puts_back_the_ones_done() {
        let (base, v) = vault_with(&[("a.md", "[[x]]"), ("b.md", "[[x]] too")]);
        // The third rewrite expects a.md as it was before the first one,
        // so it fails after a.md and b.md were already rewritten.
        let rewrites = [
            rewrite("a.md", "[[x]]", "[[y]]"),
            rewrite("b.md", "[[x]] too", "[[y]] too"),
            rewrite("a.md", "[[x]]", "[[z]]"),
        ];
        let e = rewrite_all(&v, &rewrites).unwrap_err();
        assert!(
            e.message.contains("2 other notes were put back"),
            "{}",
            e.message
        );
        let root = base.path().join("vault");
        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "[[x]]");
        assert_eq!(fs::read_to_string(root.join("b.md")).unwrap(), "[[x]] too");
    }

    #[test]
    fn a_note_changed_before_the_rewrites_stops_them_all() {
        let (base, v) = vault_with(&[("a.md", "[[x]]"), ("b.md", "edited meanwhile")]);
        let rewrites = [
            rewrite("a.md", "[[x]]", "[[y]]"),
            rewrite("b.md", "[[x]] too", "[[y]] too"),
        ];
        rewrite_all(&v, &rewrites).unwrap_err();
        let root = base.path().join("vault");
        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "[[x]]");
    }

    #[test]
    fn notes_that_cannot_be_put_back_are_named() {
        let (base, v) = vault_with(&[("a.md", "[[y]]")]);
        let done = rewrite("a.md", "[[x]]", "[[y]]");
        // Someone edits a.md after this change rewrote it.
        fs::write(base.path().join("vault/a.md"), "theirs").unwrap();
        let e = undone(&v, ToolError::write_io(io_err()), &[&done]);
        assert!(e.message.contains("could not be put back"), "{}", e.message);
        assert!(e.message.contains("a.md"), "{}", e.message);
        assert_eq!(
            fs::read_to_string(base.path().join("vault/a.md")).unwrap(),
            "theirs"
        );
    }

    fn io_err() -> std::io::Error {
        std::io::Error::other("disk full")
    }
}
