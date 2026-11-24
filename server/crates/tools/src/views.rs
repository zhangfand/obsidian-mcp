//! How vault terms look to an agent, as JSON.

use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use vault::{Note, PropertyValue, ResolvedLink};

/// The most bytes one text field of a list item may take; longer text is
/// cut and the item's `cut` names the field.
pub const FIELD_LIMIT: usize = 512;

/// Marks the end of text that was cut.
pub const CUT_MARK: char = '…';

/// `s` cut to at most `max` bytes (at a character boundary, ending in `…`),
/// and whether it was cut.
pub fn clip(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max.saturating_sub(CUT_MARK.len_utf8());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}{CUT_MARK}", &s[..end]), true)
}

/// Keeps items in order while their JSON stays within `total_max` bytes,
/// dropping any item longer than `item_max` (a shortened name would be a
/// wrong name); says whether anything was left out.
pub fn fit_list(items: Vec<String>, item_max: usize, total_max: usize) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut used = 2;
    let mut cut = false;
    for item in items {
        let len = serde_json::to_string(&item).map_or(usize::MAX, |s| s.len()) + 1;
        if item.len() > item_max || used + len > total_max {
            cut = true;
            continue;
        }
        used += len;
        out.push(item);
    }
    (out, cut)
}

fn clip_opt(s: &Option<String>, field: &'static str, cut: &mut Vec<&'static str>) -> Option<String> {
    s.as_deref().map(|s| {
        let (v, was_cut) = clip(s, FIELD_LIMIT);
        if was_cut {
            cut.push(field);
        }
        v
    })
}

/// One link occurrence, as in TOOLS.md.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LinkView {
    pub from: String,
    pub to: Option<String>,
    pub target: String,
    pub kind: &'static str,
    pub heading: Option<String>,
    pub block: Option<String>,
    pub display: Option<String>,
    pub line: usize,
    /// Fields shortened to `FIELD_LIMIT`; the whole text is on `line` of
    /// `from`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<&'static str>,
}

impl From<&ResolvedLink<'_>> for LinkView {
    fn from(r: &ResolvedLink<'_>) -> LinkView {
        let mut cut = Vec::new();
        let target = clip_opt(&Some(r.link.target.clone()), "target", &mut cut).unwrap_or_default();
        let heading = clip_opt(&r.link.heading, "heading", &mut cut);
        let block = clip_opt(&r.link.block, "block", &mut cut);
        let display = clip_opt(&r.link.display, "display", &mut cut);
        LinkView {
            from: r.from.to_string(),
            to: r.to.map(|p| p.to_string()),
            target,
            kind: r.link.kind.as_str(),
            heading,
            block,
            display,
            line: r.link.line,
            cut,
        }
    }
}

/// A note in a list.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NoteRef {
    pub path: String,
    pub name: String,
}

impl From<&Note> for NoteRef {
    fn from(n: &Note) -> NoteRef {
        NoteRef { path: n.path.to_string(), name: n.name().to_string() }
    }
}

/// The note's version: changes whenever any byte of the file changes.
pub fn version(note: &Note) -> String {
    let digest = Sha256::digest(note.text.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn tags(note: &Note) -> Vec<String> {
    note.tags.iter().map(|t| t.as_str().to_string()).collect()
}

pub fn property_value(v: &PropertyValue) -> Value {
    match v {
        PropertyValue::Empty => Value::Null,
        PropertyValue::Bool(b) => json!(b),
        PropertyValue::Integer(i) => json!(i),
        PropertyValue::Float(f) => json!(f),
        PropertyValue::Text(s) => json!(s),
        PropertyValue::List(items) => Value::Array(items.iter().map(property_value).collect()),
        PropertyValue::Object(entries) => {
            Value::Object(entries.iter().map(|(k, v)| (k.clone(), property_value(v))).collect::<Map<_, _>>())
        }
    }
}
