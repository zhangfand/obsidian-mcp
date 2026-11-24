use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vault::{Note, Part, Section, SectionLookup, Snapshot};

use crate::error::{ErrorKind, ToolError, found, quote};
use crate::paging::{Cursor, REPLY_LIMIT, fit_items, fit_text, size};
use crate::views::{self, FIELD_LIMIT, clip, fit_list};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadNoteArgs {
    pub note: String,
    pub section: Option<String>,
    pub heading_line: Option<usize>,
    #[serde(default)]
    pub full: bool,
    #[serde(default)]
    pub raw_properties: bool,
    #[serde(skip_serializing)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReadMode {
    Full,
    Outline,
    Section,
    Properties,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OutlineEntry {
    pub section: String,
    pub level: u8,
    pub line: usize,
    pub bytes: usize,
    /// `["section"]` when the section path was shortened; read the section
    /// with `heading_line`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
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
    /// Header fields shortened or left out to keep the reply under 8 KB.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<String>,
}

/// Byte budgets for the first page's header, so the text beside it always
/// has room.
const PROPERTIES_LIMIT: usize = 2500;
const PROPERTY_VALUE_LIMIT: usize = 500;
const NAMES_LIMIT: usize = 800;
const NAME_LIMIT: usize = 200;
/// The most bytes of headings listed in a section error.
const HEADINGS_LIMIT: usize = 4000;

pub fn read_note(snap: &Snapshot, args: &ReadNoteArgs) -> Result<ReadNoteResult, ToolError> {
    let cursor = Cursor::new(
        "read_note",
        &serde_json::to_value(args).unwrap_or_default(),
        args.page.as_deref(),
    )?;
    let picked = [
        args.section.is_some(),
        args.heading_line.is_some(),
        args.raw_properties,
        args.full,
    ];
    if picked.iter().filter(|p| **p).count() > 1 {
        return Err(ToolError::invalid(
            "Give at most one of `section`, `heading_line`, `raw_properties` and `full`.",
        ));
    }
    let note = found(&args.note, snap.lookup(&args.note))?;
    let first = cursor.offset == 0;
    let header = first.then(|| Header::of(note));
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
            cut: Vec::new(),
            next_page: None,
        };
        if let Some(h) = &header {
            r.name = Some(note.name().to_string());
            r.aliases = Some(h.aliases.clone());
            r.tags = Some(h.tags.clone());
            r.properties = Some(h.properties.clone());
            r.cut = h.cut.clone();
        }
        r
    };

    if args.raw_properties {
        return fit_text(&cursor, note.properties_text(), |chunk, next_page| {
            ReadNoteResult {
                content: Some(chunk.to_string()),
                next_page,
                ..head(ReadMode::Properties)
            }
        });
    }

    if let Some(target) = pick_target(note, args.section.as_deref(), args.heading_line)? {
        let (name, range) = match &target {
            Target::Section(s) => (note.short_section_path(s), s.range.clone()),
            Target::Lead => (String::new(), note.lead_range()),
        };
        let (path, path_cut) = clip(&name, FIELD_LIMIT);
        let text = &note.text[range];
        return fit_text(&cursor, text, |chunk, next_page| {
            let mut r = ReadNoteResult {
                section: Some(path.clone()),
                content: Some(chunk.to_string()),
                next_page,
                ..head(ReadMode::Section)
            };
            if path_cut {
                r.cut.push("section");
            }
            r
        });
    }

    let whole = ReadNoteResult {
        content: Some(note.body().to_string()),
        ..head(ReadMode::Full)
    };
    // A long note without headings has no outline to offer, so it is paged.
    if args.full || note.headings.is_empty() || (first && size(&whole) <= REPLY_LIMIT) {
        return fit_text(&cursor, note.body(), |chunk, next_page| ReadNoteResult {
            content: Some(chunk.to_string()),
            next_page,
            ..head(ReadMode::Full)
        });
    }

    let outline = outline(note);
    Ok(fit_items(&cursor, outline.len(), |s, e, next_page| {
        ReadNoteResult {
            outline: Some(outline[s..e].to_vec()),
            next_page,
            ..head(ReadMode::Outline)
        }
    }))
}

/// The first page's note-wide fields, kept within their budgets.
pub(crate) struct Header {
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub properties: Value,
    pub cut: Vec<&'static str>,
}

impl Header {
    pub(crate) fn of(note: &Note) -> Header {
        let mut cut = Vec::new();
        let (aliases, a_cut) = fit_list(
            note.aliases.iter().map(|a| a.0.clone()).collect(),
            NAME_LIMIT,
            NAMES_LIMIT,
        );
        if a_cut {
            cut.push("aliases");
        }
        let (tags, t_cut) = fit_list(views::tags(note), NAME_LIMIT, NAMES_LIMIT);
        if t_cut {
            cut.push("tags");
        }
        let mut props = Map::new();
        let mut used = 2;
        let mut p_cut = false;
        for p in &note.properties.entries {
            let value = views::property_value(&p.value);
            let len = size(&p.name) + size(&value) + 2;
            if p.name.len() > NAME_LIMIT
                || size(&value) > PROPERTY_VALUE_LIMIT
                || used + len > PROPERTIES_LIMIT
            {
                p_cut = true;
                continue;
            }
            used += len;
            props.insert(p.name.clone(), value);
        }
        if p_cut {
            cut.push("properties");
        }
        Header {
            aliases,
            tags,
            properties: Value::Object(props),
            cut,
        }
    }
}

/// A part of a note named by `section` or `heading_line`.
pub(crate) enum Target {
    Section(Section),
    Lead,
}

impl Target {
    pub(crate) fn part(&self) -> Part<'_> {
        match self {
            Target::Section(s) => Part::Section(s),
            Target::Lead => Part::Lead,
        }
    }
}

/// The part named by `section` (`""` for the lead) or `heading_line`;
/// `None` when neither is given. Giving both is refused.
pub(crate) fn pick_target(
    note: &Note,
    section: Option<&str>,
    heading_line: Option<usize>,
) -> Result<Option<Target>, ToolError> {
    match (section, heading_line) {
        (Some(_), Some(_)) => Err(ToolError::invalid(
            "Give at most one of `section` and `heading_line`.",
        )),
        (Some(q), None) if q.trim().is_empty() => Ok(Some(Target::Lead)),
        (Some(q), None) => find_section(note, q).map(|s| Some(Target::Section(s))),
        (None, Some(line)) => note
            .section_at_line(line)
            .map(|s| Some(Target::Section(s)))
            .ok_or_else(|| {
                section_error(
                    ErrorKind::SectionNotFound,
                    format!("{} has no heading on line {line}.", note.path),
                    note.sections()
                        .iter()
                        .map(|s| note.short_section_path(s))
                        .collect(),
                )
            }),
        (None, None) => Ok(None),
    }
}

pub(crate) fn find_section(note: &Note, query: &str) -> Result<Section, ToolError> {
    match note.find_section(query) {
        SectionLookup::Found(s) => Ok(s),
        SectionLookup::Ambiguous(paths) => Err(section_error(
            ErrorKind::AmbiguousSection,
            format!(
                "{} matches {} headings in {}. Pass one of `headings` as `section`.",
                quote(query),
                paths.len(),
                note.path
            ),
            paths,
        )),
        SectionLookup::NotFound => Err(section_error(
            ErrorKind::SectionNotFound,
            format!(
                "{} has no heading {}. Pass one of `headings` as `section`.",
                note.path,
                quote(query)
            ),
            note.sections()
                .iter()
                .map(|s| note.short_section_path(s))
                .collect(),
        )),
    }
}

pub(crate) fn section_error(kind: ErrorKind, message: String, headings: Vec<String>) -> ToolError {
    let (headings, cut) = fit_list(headings, FIELD_LIMIT, HEADINGS_LIMIT);
    let mut e = ToolError::new(kind, message).with("headings", json!(headings));
    if cut {
        e.message.push_str(" Some headings are too long or too many to list; read_note without `section` gives the outline, and `heading_line` reads a section by its line.");
        e = e.with("cut", json!(["headings"]));
    }
    e
}

fn outline(note: &Note) -> Vec<OutlineEntry> {
    let lead = note.lead_range();
    let lead_entry = (!note.text[lead.clone()].trim().is_empty()).then(|| OutlineEntry {
        section: String::new(),
        level: 0,
        line: note.line_of(lead.start),
        bytes: lead.len(),
        cut: Vec::new(),
    });
    lead_entry
        .into_iter()
        .chain(note.sections().iter().map(|s| {
            let h = &note.headings[s.heading];
            let (section, was_cut) = clip(&note.short_section_path(s), FIELD_LIMIT);
            OutlineEntry {
                section,
                level: h.level,
                line: h.line,
                bytes: s.range.len(),
                cut: if was_cut { vec!["section"] } else { Vec::new() },
            }
        }))
        .collect()
}
