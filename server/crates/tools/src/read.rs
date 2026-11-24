use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vault::{Note, SectionLookup, Snapshot};

use crate::error::{ErrorKind, ToolError, found};
use crate::paging::{Cursor, REPLY_LIMIT, fit_items, fit_text, size};
use crate::views;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadNoteArgs {
    pub note: String,
    pub section: Option<String>,
    #[serde(default)]
    pub full: bool,
    #[serde(skip_serializing)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReadMode {
    Full,
    Outline,
    Section,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OutlineEntry {
    pub section: String,
    pub level: u8,
    pub line: usize,
    pub bytes: usize,
}

/// Fields that only the first page carries are `None` on later pages.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReadNoteResult {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Value>,
    pub mode: ReadMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<OutlineEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<String>,
}

pub fn read_note(snap: &Snapshot, args: &ReadNoteArgs) -> Result<ReadNoteResult, ToolError> {
    let cursor = Cursor::new("read_note", &serde_json::to_value(args).unwrap_or_default(), args.page.as_deref())?;
    let note = found(&args.note, snap.lookup(&args.note))?;
    let first = cursor.offset == 0;
    let head = |mode: ReadMode| {
        let mut r = ReadNoteResult {
            path: note.path.to_string(),
            name: None,
            version: views::version(note),
            aliases: None,
            tags: None,
            properties: None,
            mode,
            section: None,
            content: None,
            outline: None,
            next_page: None,
        };
        if first {
            r.name = Some(note.name().to_string());
            r.aliases = Some(note.aliases.iter().map(|a| a.0.clone()).collect());
            r.tags = Some(views::tags(note));
            r.properties = Some(views::properties(&note.properties));
        }
        r
    };

    if let Some(query) = args.section.as_deref() {
        let section = match note.find_section(query) {
            SectionLookup::Found(s) => s,
            SectionLookup::Ambiguous(paths) => {
                return Err(ToolError::new(
                    ErrorKind::AmbiguousSection,
                    format!("`{query}` matches {} headings in {}. Pass one of `headings` as `section`.", paths.len(), note.path),
                )
                .with("headings", json!(paths)));
            }
            SectionLookup::NotFound => {
                let headings: Vec<String> = note.sections().iter().map(|s| note.short_section_path(s)).collect();
                return Err(ToolError::new(
                    ErrorKind::SectionNotFound,
                    format!("{} has no heading `{query}`. Pass one of `headings` as `section`.", note.path),
                )
                .with("headings", json!(headings)));
            }
        };
        let path = note.short_section_path(&section);
        let text = &note.text[section.range.clone()];
        return fit_text(&cursor, text, |chunk, next_page| ReadNoteResult {
            section: Some(path.clone()),
            content: Some(chunk.to_string()),
            next_page,
            ..head(ReadMode::Section)
        });
    }

    let whole = ReadNoteResult { content: Some(note.body().to_string()), ..head(ReadMode::Full) };
    // A long note without headings has no outline to offer, so it is paged.
    if args.full || note.headings.is_empty() || (first && size(&whole) <= REPLY_LIMIT) {
        return fit_text(&cursor, note.body(), |chunk, next_page| ReadNoteResult {
            content: Some(chunk.to_string()),
            next_page,
            ..head(ReadMode::Full)
        });
    }

    let outline = outline(note);
    Ok(fit_items(&cursor, outline.len(), |s, e, next_page| ReadNoteResult {
        outline: Some(outline[s..e].to_vec()),
        next_page,
        ..head(ReadMode::Outline)
    }))
}

fn outline(note: &Note) -> Vec<OutlineEntry> {
    note.sections()
        .iter()
        .map(|s| {
            let h = &note.headings[s.heading];
            OutlineEntry { section: note.short_section_path(s), level: h.level, line: h.line, bytes: s.range.len() }
        })
        .collect()
}
