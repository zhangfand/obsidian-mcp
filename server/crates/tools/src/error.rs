use serde_json::{Map, Value, json};
use vault::{Note, NoteLookup, PathError};

use crate::views::{clip, fit_list};

/// A user-given value for an error message, in backticks, cut to 200 bytes.
pub fn quote(s: &str) -> String {
    format!("`{}`", clip(s, 200).0)
}

/// A refusal, worded for an agent: what went wrong and what to try next.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
    /// Extra fields beside `error` and `message`.
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    NotFound,
    Ambiguous,
    OutsideVault,
    SectionNotFound,
    AmbiguousSection,
    InvalidArgument,
    BadPage,
    Stale,
    AlreadyExists,
    IoError,
    TooLarge,
    InvalidProperties,
}

impl ErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::NotFound => "not_found",
            ErrorKind::Ambiguous => "ambiguous",
            ErrorKind::OutsideVault => "outside_vault",
            ErrorKind::SectionNotFound => "section_not_found",
            ErrorKind::AmbiguousSection => "ambiguous_section",
            ErrorKind::InvalidArgument => "invalid_argument",
            ErrorKind::BadPage => "bad_page",
            ErrorKind::Stale => "stale",
            ErrorKind::AlreadyExists => "already_exists",
            ErrorKind::IoError => "io_error",
            ErrorKind::TooLarge => "too_large",
            ErrorKind::InvalidProperties => "invalid_properties",
        }
    }
}

impl ToolError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> ToolError {
        ToolError {
            kind,
            message: message.into(),
            extra: Map::new(),
        }
    }

    pub fn with(mut self, key: &str, value: Value) -> ToolError {
        self.extra.insert(key.to_string(), value);
        self
    }

    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("error".into(), json!(self.kind.as_str()));
        obj.insert("message".into(), json!(self.message));
        obj.extend(self.extra.clone());
        Value::Object(obj)
    }

    pub fn invalid(message: impl Into<String>) -> ToolError {
        ToolError::new(ErrorKind::InvalidArgument, message)
    }

    pub fn outside(what: &str, e: PathError) -> ToolError {
        ToolError::new(
            ErrorKind::OutsideVault,
            format!(
                "{} is refused: {e}. Name a note by its name, an alias, or a path inside the vault.",
                quote(what)
            ),
        )
    }

    pub fn io(e: std::io::Error) -> ToolError {
        ToolError::new(
            ErrorKind::IoError,
            format!(
                "The vault could not be read: {e}. Try again; if it persists, the server's vault folder needs attention."
            ),
        )
    }

    pub fn bad_page() -> ToolError {
        ToolError::new(
            ErrorKind::BadPage,
            "This `page` token was not given by this tool for these same arguments. Repeat the first call without `page` and follow its `next_page`.",
        )
    }
}

/// Turns a lookup into the note or the matching refusal.
pub fn found<'a>(query: &str, lookup: NoteLookup<'a>) -> Result<&'a Note, ToolError> {
    match lookup {
        NoteLookup::Found(n) => Ok(n),
        NoteLookup::Ambiguous(ns) => {
            let count = ns.len();
            let (paths, cut) =
                fit_list(ns.iter().map(|n| n.path.to_string()).collect(), 1024, 4000);
            let mut e = ToolError::new(
                ErrorKind::Ambiguous,
                format!(
                    "{} names {count} notes. Pass one of the paths in `candidates` as `note`.",
                    quote(query)
                ),
            )
            .with("candidates", json!(paths));
            if cut {
                e.message
                    .push_str(" Not all fit here; find_notes lists them all.");
                e = e.with("cut", json!(["candidates"]));
            }
            Err(e)
        }
        NoteLookup::NotFound(closest) => {
            let hint = if closest.is_empty() {
                "The vault has no notes.".to_string()
            } else {
                format!(
                    "Closest names: {}. Pass one of them, or use find_notes to search.",
                    closest.join(", ")
                )
            };
            Err(ToolError::new(
                ErrorKind::NotFound,
                format!(
                    "No note is named {} by path, name or alias. {hint}",
                    quote(query)
                ),
            )
            .with("candidates", json!(closest)))
        }
        NoteLookup::Refused(e) => Err(ToolError::outside(query, e)),
    }
}
