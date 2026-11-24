//! How vault terms look to an agent, as JSON.

use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use vault::{Note, Properties, PropertyValue, ResolvedLink};

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
}

impl From<&ResolvedLink<'_>> for LinkView {
    fn from(r: &ResolvedLink<'_>) -> LinkView {
        LinkView {
            from: r.from.to_string(),
            to: r.to.map(|p| p.to_string()),
            target: r.link.target.clone(),
            kind: r.link.kind.as_str(),
            heading: r.link.heading.clone(),
            block: r.link.block.clone(),
            display: r.link.display.clone(),
            line: r.link.line,
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

pub fn properties(props: &Properties) -> Value {
    Value::Object(props.entries.iter().map(|p| (p.name.clone(), property_value(&p.value))).collect())
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
