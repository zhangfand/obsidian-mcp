use serde_json::{Value, json};

/// A tool as listed to agents.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
    pub read_only: bool,
}

const NOTE: &str = "The note: its name, an alias, or its path in the vault (with or without .md).";
const PAGE: &str = "The next_page value from the previous reply, to continue a long result. Pass the same other arguments again.";

/// The tools this server offers, in a stable order.
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "find_notes",
            description: "Find notes by text, tag, property and folder; every given filter must match. A tag matches its nested tags (project matches project/alpha). With text, each note carries up to 3 matching lines as snippets. Results are sorted by path and paged.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "Case-insensitive text to find in the note name or anywhere in the file."},
                    "tag": {"type": "string", "description": "A tag, with or without #. Nested tags below it match too."},
                    "property": {
                        "type": "object",
                        "description": "A property the note must have; with value, the property must equal it or (for a list) contain it.",
                        "properties": {"name": {"type": "string"}, "value": {}},
                        "required": ["name"],
                        "additionalProperties": false
                    },
                    "folder": {"type": "string", "description": "Only notes in this folder or below it."},
                    "page": {"type": "string", "description": PAGE}
                },
                "additionalProperties": false
            }),
            read_only: true,
        },
        ToolSpec {
            name: "read_note",
            description: "Read a note: its properties, tags, aliases and version, and its text. A short note comes back whole; a long one as an outline of headings, so ask again with section to read one part, or full=true to page through all of it.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "note": {"type": "string", "description": NOTE},
                    "section": {"type": "string", "description": "A heading to read, e.g. \"Plan\"; add parent headings joined by # when the text repeats, e.g. \"Week 1#Plan\"."},
                    "full": {"type": "boolean", "description": "Return the whole text, paged, even when it is long."},
                    "page": {"type": "string", "description": PAGE}
                },
                "required": ["note"],
                "additionalProperties": false
            }),
            read_only: true,
        },
        ToolSpec {
            name: "get_links",
            description: "Links of a note: outgoing (every link and embed written in it, resolved or not) and incoming (every link in other notes that leads to it).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "note": {"type": "string", "description": NOTE},
                    "direction": {"type": "string", "enum": ["out", "in", "both"], "description": "Which links to list; default both."},
                    "page": {"type": "string", "description": PAGE}
                },
                "required": ["note"],
                "additionalProperties": false
            }),
            read_only: true,
        },
        ToolSpec {
            name: "check_vault",
            description: "Health of the whole vault: links that lead nowhere, and notes with no links to or from other notes.",
            input_schema: json!({
                "type": "object",
                "properties": {"page": {"type": "string", "description": PAGE}},
                "additionalProperties": false
            }),
            read_only: true,
        },
    ]
}
