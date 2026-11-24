use serde::{Deserialize, Serialize};
use serde_json::Value;
use vault::{Note, PropertyValue, Snapshot, Tag};

use crate::error::ToolError;
use crate::paging::{Cursor, fit_items};
use crate::views;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindNotesArgs {
    pub text: Option<String>,
    pub tag: Option<String>,
    pub property: Option<PropertyFilter>,
    pub folder: Option<String>,
    #[serde(skip_serializing)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyFilter {
    pub name: String,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FoundNote {
    pub path: String,
    pub name: String,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippets: Option<Vec<Snippet>>,
    /// `["tags"]` when tags were left out to keep the item small; read_note
    /// lists them all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Snippet {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FindNotesResult {
    pub notes: Vec<FoundNote>,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<String>,
}

const MAX_SNIPPETS: usize = 3;
const SNIPPET_BYTES: usize = 200;
/// Longest tag listed with a found note, and the most bytes of its tags.
const TAG_LIMIT: usize = 200;
const TAGS_LIMIT: usize = 1000;

pub fn find_notes(snap: &Snapshot, args: &FindNotesArgs) -> Result<FindNotesResult, ToolError> {
    let cursor = Cursor::new(
        "find_notes",
        &serde_json::to_value(args).unwrap_or_default(),
        args.page.as_deref(),
    )?;
    let text = args
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    if args.text.is_some() && text.is_none() {
        return Err(ToolError::invalid(
            "`text` is empty. Leave it out or give some text to search for.",
        ));
    }
    let tag = match args.tag.as_deref().map(Tag::new) {
        Some(t) if t.as_str().is_empty() => {
            return Err(ToolError::invalid(
                "`tag` is empty. Give a tag such as `project` or `#project/alpha`.",
            ));
        }
        t => t,
    };
    if let Some(p) = &args.property
        && p.name.trim().is_empty()
    {
        return Err(ToolError::invalid(
            "`property.name` is empty. Give the property name, e.g. `status`.",
        ));
    }
    let folder = match args.folder.as_deref().map(str::trim) {
        None | Some("") | Some("/") => None,
        Some(f) => Some(snap.safe_path(f).map_err(|e| ToolError::outside(f, e))?),
    };

    let matches: Vec<&Note> = snap
        .notes()
        .iter()
        .map(AsRef::as_ref)
        .filter(|n| folder.as_ref().is_none_or(|f| n.path.is_inside(f)))
        .filter(|n| tag.as_ref().is_none_or(|t| n.has_tag(t)))
        .filter(|n| {
            args.property
                .as_ref()
                .is_none_or(|p| property_matches(n, p))
        })
        .filter(|n| text.is_none_or(|t| n.contains_text(t)))
        .collect();

    let total = matches.len();
    let items: Vec<FoundNote> = matches
        .iter()
        .map(|n| {
            let (tags, tags_cut) = views::fit_list(views::tags(n), TAG_LIMIT, TAGS_LIMIT);
            FoundNote {
                path: n.path.to_string(),
                name: n.name().to_string(),
                tags,
                snippets: text.map(|t| snippets(n, t)),
                cut: if tags_cut { vec!["tags"] } else { Vec::new() },
            }
        })
        .collect();
    Ok(fit_items(&cursor, total, |s, e, next_page| {
        FindNotesResult {
            notes: items[s..e].to_vec(),
            total,
            next_page,
        }
    }))
}

fn property_matches(note: &Note, filter: &PropertyFilter) -> bool {
    let Some(value) = note.properties.get(filter.name.trim()) else {
        return false;
    };
    match &filter.value {
        None => true,
        Some(Value::Array(wanted)) => wanted.iter().all(|w| value_matches(value, w)),
        Some(w) => value_matches(value, w),
    }
}

/// Equal, or (for a list) containing it; text compares case-insensitively,
/// numbers and booleans also against their text form.
fn value_matches(have: &PropertyValue, wanted: &Value) -> bool {
    match have {
        PropertyValue::List(items) => items.iter().any(|i| scalar_matches(i, wanted)),
        other => scalar_matches(other, wanted),
    }
}

fn scalar_matches(have: &PropertyValue, wanted: &Value) -> bool {
    match (have, wanted) {
        (PropertyValue::Empty, Value::Null) => true,
        (PropertyValue::Integer(a), Value::Number(b)) => b.as_f64() == Some(*a as f64),
        (PropertyValue::Float(a), Value::Number(b)) => b.as_f64() == Some(*a),
        (PropertyValue::Bool(a), Value::Bool(b)) => a == b,
        (have, wanted) => {
            let wanted_text = match wanted {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return false,
            };
            match have.as_text() {
                Some(t) => {
                    let (t, w) = (t.trim().to_lowercase(), wanted_text.trim().to_lowercase());
                    // `[[Alpha]]` stored in a property matches `Alpha`.
                    t == w || t.trim_start_matches("[[").trim_end_matches("]]") == w
                }
                None => false,
            }
        }
    }
}

fn snippets(note: &Note, needle: &str) -> Vec<Snippet> {
    let needle = needle.to_lowercase();
    let mut out = Vec::new();
    for (i, line) in note.text.lines().enumerate() {
        if out.len() == MAX_SNIPPETS {
            break;
        }
        let lower = line.to_lowercase();
        let Some(pos) = lower.find(&needle) else {
            continue;
        };
        // Lower-casing can change byte lengths; then start from the line's start.
        let pos = if lower.len() == line.len() { pos } else { 0 };
        out.push(Snippet {
            line: i + 1,
            text: window(line, pos, needle.len()),
        });
    }
    out
}

/// About `SNIPPET_BYTES` of `line` around the match, with `…` where cut.
fn window(line: &str, pos: usize, len: usize) -> String {
    let line_trimmed = line.trim();
    if line_trimmed.len() <= SNIPPET_BYTES {
        return line_trimmed.to_string();
    }
    let lead = SNIPPET_BYTES.saturating_sub(len) / 2;
    let mut start = pos.saturating_sub(lead);
    let mut end = (start + SNIPPET_BYTES).min(line.len());
    start = end.saturating_sub(SNIPPET_BYTES).min(start);
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let mut s = String::new();
    if start > 0 {
        s.push('…');
    }
    s.push_str(line[start..end].trim());
    if end < line.len() {
        s.push('…');
    }
    s
}
