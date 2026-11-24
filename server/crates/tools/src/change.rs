//! The five change tools. The vault works out which bytes change; this
//! module checks versions, picks trash names, writes, and words the result.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vault::{
    MoveError, Note, Property, PropertyEditError, SectionEdit, Snapshot, Vault, VaultPath, WriteError, new_note_text,
};

use crate::error::{ErrorKind, ToolError, found, quote};
use crate::paging::size;
use crate::read::{Header, find_section, section_error};
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
        return Err(ToolError::invalid("`version` is empty. Pass the version that read_note gave for this note."));
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
                format!("{path} changed while the change was being made; nothing was written to it. Read it again with read_note and retry."),
            );
            if let Ok(bytes) = vault.read_file(path) {
                err = err.with("version", json!(views::version_of(&String::from_utf8_lossy(&bytes))));
            }
            err
        }
        WriteError::Exists => ToolError::new(
            ErrorKind::AlreadyExists,
            format!("{path} already exists. Choose another path."),
        )
        .with("path", json!(path.as_str())),
        WriteError::Io(e) => ToolError::io(e),
    }
}

fn md_path(snap: &Snapshot, raw: &str) -> Result<VaultPath, ToolError> {
    let p = snap.safe_path(raw).map_err(|e| ToolError::outside(raw, e))?;
    if p.is_note() {
        return Ok(p);
    }
    let with_md = format!("{}.md", p.as_str());
    snap.safe_path(&with_md).map_err(|e| ToolError::outside(raw, e))
}

/// The existing file whose path equals `path` ignoring case.
fn taken<'a>(snap: &'a Snapshot, path: &VaultPath) -> Option<&'a VaultPath> {
    let lower = path.as_str().to_lowercase();
    snap.files().iter().find(|f| f.as_str().to_lowercase() == lower)
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
    (list, total, if cut { vec!["updated_notes"] } else { Vec::new() })
}

fn plan_error(e: MoveError, doing: &str) -> ToolError {
    match e {
        MoveError::CannotRewrite { from, line } => ToolError::invalid(format!(
            "{doing} would need to rewrite a reference-style link in {from} (line {line}), which cannot be changed in place. Nothing was written."
        )),
        MoveError::WouldBreak { from, line } => ToolError::invalid(format!(
            "{doing} would leave a link in {from} (line {line}) pointing elsewhere. Nothing was written."
        )),
    }
}

/// Checks every planned file is unchanged, then rewrites them one by one.
/// Returns their paths.
fn rewrite_all(vault: &Vault, rewrites: &[vault::FileRewrite], not_done: &str) -> Result<Vec<String>, ToolError> {
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
            let mut err = write_error(vault, &r.path, e);
            err.message.push_str(&format!(
                " This stopped part way: {} other notes were already rewritten and {not_done}. Retry.",
                done.len()
            ));
            return Err(err);
        }
        done.push(r.path.to_string());
    }
    Ok(done)
}

pub fn create_note(vault: &Vault, snap: &Snapshot, args: &CreateNoteArgs) -> Result<CreateNoteResult, ToolError> {
    let path = md_path(snap, &args.path)?;
    if let Some(existing) = taken(snap, &path) {
        return Err(exists_error(existing, "already exists. Choose another path."));
    }
    let properties: Vec<Property> = match &args.properties {
        None => Vec::new(),
        Some(Value::Object(map)) => {
            map.iter().map(|(k, v)| Property { name: k.clone(), value: views::from_json(v) }).collect()
        }
        Some(_) => return Err(ToolError::invalid("`properties` must be an object of property names and values.")),
    };
    let starts_with_block = args.content.lines().next().is_some_and(|l| l.trim_end() == "---");
    if !properties.is_empty() && starts_with_block {
        return Err(ToolError::invalid(
            "`content` starts with its own properties block and `properties` is given too. Pass properties in only one of them.",
        ));
    }
    let text = new_note_text(&properties, &args.content).ok_or_else(|| {
        ToolError::invalid("A property name or value cannot be written as YAML so that it reads back unchanged.")
    })?;
    let new_note = Note::parse(path.clone(), text.clone());
    let rewrites = snap.plan_create(&new_note).map_err(|e| plan_error(e, "creating it"))?;
    let updated = rewrite_all(vault, &rewrites, "the note was not created")?;
    vault.create_file(&path, text.as_bytes()).map_err(|e| write_error(vault, &path, e))?;
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

// ---- edit_section ----

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
pub struct EditSectionArgs {
    pub note: String,
    pub section: Option<String>,
    pub heading_line: Option<usize>,
    pub content: String,
    #[serde(default)]
    pub mode: EditMode,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EditResult {
    pub path: String,
    pub version: String,
}

pub fn edit_section(vault: &Vault, snap: &Snapshot, args: &EditSectionArgs) -> Result<EditResult, ToolError> {
    let note = found(&args.note, snap.lookup(&args.note))?;
    let section = match (&args.section, args.heading_line) {
        (Some(q), None) => find_section(note, q)?,
        (None, Some(line)) => note.section_at_line(line).ok_or_else(|| {
            section_error(
                ErrorKind::SectionNotFound,
                format!("{} has no heading on line {line}.", note.path),
                note.sections().iter().map(|s| note.short_section_path(s)).collect(),
            )
        })?,
        _ => return Err(ToolError::invalid("Give exactly one of `section` and `heading_line`.")),
    };
    required_version(note, &args.version)?;
    let edit = match args.mode {
        EditMode::Replace => SectionEdit::Replace,
        EditMode::Append => SectionEdit::Append,
        EditMode::Prepend => SectionEdit::Prepend,
    };
    let new = note.edit_section(&section, edit, &args.content);
    vault.replace_file(&note.path, note.text.as_bytes(), new.as_bytes()).map_err(|e| write_error(vault, &note.path, e))?;
    Ok(EditResult { path: note.path.to_string(), version: views::version_of(&new) })
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

pub fn set_property(vault: &Vault, snap: &Snapshot, args: &SetPropertyArgs) -> Result<SetPropertyResult, ToolError> {
    let note = found(&args.note, snap.lookup(&args.note))?;
    required_version(note, &args.version)?;
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
        vault.replace_file(&note.path, note.text.as_bytes(), new.as_bytes()).map_err(|e| write_error(vault, &note.path, e))?;
    }
    let header = Header::of(&Note::parse(note.path.clone(), new.clone()));
    let cut = if header.cut.contains(&"properties") { vec!["properties"] } else { Vec::new() };
    Ok(SetPropertyResult { path: note.path.to_string(), version: views::version_of(&new), properties: header.properties, cut })
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

pub fn move_note(vault: &Vault, snap: &Snapshot, args: &MoveNoteArgs) -> Result<MoveNoteResult, ToolError> {
    let note = found(&args.note, snap.lookup(&args.note))?;
    check_version(note, args.version.as_deref())?;
    let target = snap.safe_path(&args.to).map_err(|e| ToolError::outside(&args.to, e))?;
    let folder_prefix = format!("{}/", target.as_str().to_lowercase());
    let is_folder = args.to.trim_end().ends_with(['/', '\\'])
        || snap.files().iter().any(|f| f.as_str().to_lowercase().starts_with(&folder_prefix));
    let to = if is_folder {
        snap.safe_path(&format!("{}/{}", target.as_str(), note.path.file_name())).map_err(|e| ToolError::outside(&args.to, e))?
    } else {
        md_path(snap, &args.to)?
    };
    if to == note.path {
        return Err(ToolError::invalid(format!("{} is already at that path.", note.path)));
    }
    if let Some(existing) = taken(snap, &to)
        && *existing != note.path
    {
        return Err(exists_error(existing, "already exists. Choose another path."));
    }

    let plan = snap.plan_move(note, &to).map_err(|e| plan_error(e, "The move"))?;
    let done = rewrite_all(vault, &plan.rewrites, "the note was not moved")?;
    let final_text = match &plan.note_text {
        Some(text) => {
            vault.create_file(&to, text.as_bytes()).map_err(|e| write_error(vault, &to, e))?;
            vault.remove_file(&note.path, note.text.as_bytes()).map_err(|e| write_error(vault, &note.path, e))?;
            text.clone()
        }
        None => {
            vault.move_file(&note.path, &to, note.text.as_bytes()).map_err(|e| write_error(vault, &note.path, e))?;
            note.text.clone()
        }
    };
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
    let (stem, ext) = file_name.rsplit_once('.').map_or((file_name, ""), |(s, e)| (s, e));
    std::iter::once(file_name.to_string())
        .chain((1..).map(move |n| if ext.is_empty() { format!("{stem} {n}") } else { format!("{stem} {n}.{ext}") }))
}

pub fn delete_note(vault: &Vault, snap: &Snapshot, args: &DeleteNoteArgs) -> Result<DeleteNoteResult, ToolError> {
    let note = found(&args.note, snap.lookup(&args.note))?;
    check_version(note, args.version.as_deref())?;
    let broken: Vec<LinkView> = snap.incoming(note).iter().map(LinkView::from).collect();
    let taken_names: Vec<String> =
        vault.trash_names().map_err(ToolError::io)?.into_iter().map(|n| n.to_lowercase()).collect();
    let mut trash_path = None;
    for name in trash_candidates(note.path.file_name()).take(10_000) {
        if taken_names.contains(&name.to_lowercase()) {
            continue;
        }
        match vault.move_to_trash(&note.path, &name, note.text.as_bytes()) {
            Ok(p) => {
                trash_path = Some(p);
                break;
            }
            // Taken since the listing: try the next number.
            Err(WriteError::Exists) => continue,
            Err(e) => return Err(write_error(vault, &note.path, e)),
        }
    }
    let trash_path = trash_path.ok_or_else(|| ToolError::io(std::io::Error::other("no free name in .trash")))?;
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
    let cut = if now_broken.len() < total { vec!["now_broken"] } else { Vec::new() };
    Ok(DeleteNoteResult { path: note.path.to_string(), deleted: true, trash_path, now_broken, total_now_broken: total, cut })
}
