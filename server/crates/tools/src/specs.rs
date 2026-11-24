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
            description: "Read a note: its properties, tags, aliases and version, and its text. A short note comes back whole; a long one as an outline of headings, so ask again with section to read one part, or full=true to page through all of it. Give at most one of section, heading_line, raw_properties, full.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "note": {"type": "string", "description": NOTE},
                    "section": {"type": "string", "description": "A heading to read, e.g. \"Plan\"; add parent headings joined by # when the text repeats, e.g. \"Week 1#Plan\"."},
                    "full": {"type": "boolean", "description": "Return the whole text, paged, even when it is long."},
                    "heading_line": {"type": "integer", "minimum": 1, "description": "Read the section whose heading is on this line (from the outline); for headings too long to name or that repeat."},
                    "raw_properties": {"type": "boolean", "description": "Return the properties block as written (YAML), paged; for properties too large for the first reply."},
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
            description: "Broken links in the whole vault: links that lead nowhere, and ambiguous links whose name matches several files.",
            input_schema: json!({
                "type": "object",
                "properties": {"page": {"type": "string", "description": PAGE}},
                "additionalProperties": false
            }),
            read_only: true,
        },
        ToolSpec {
            name: "create_note",
            description: "Create a note at a path, with optional properties and body text. Refused when a file already has that path. Existing links that the new note would capture are rewritten to keep their target.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Vault-relative path; .md is added when missing."},
                    "content": {"type": "string", "description": "The body text."},
                    "properties": {"type": "object", "description": "Properties to write in a properties block at the top."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            read_only: false,
        },
        ToolSpec {
            name: "edit_note",
            description: "Change one part of a note, in one of two ways. By text: old_text (must occur exactly once after the properties block; heading lines and the text before the first heading count) is replaced by new_text. By section: section or heading_line with content and mode replaces, appends to or prepends to the section's body (subsections included; the heading line and blank lines around the body stay). section \"\" is the text before the first heading. Only the target bytes change. Properties: use set_property. Needs the version from read_note; refused as stale if the note changed since.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "note": {"type": "string", "description": NOTE},
                    "version": {"type": "string", "description": "The version read_note gave."},
                    "old_text": {"type": "string", "description": "By text: the exact text to replace. Line endings compare equal."},
                    "new_text": {"type": "string", "description": "By text: what to put instead (may be empty)."},
                    "section": {"type": "string", "description": "By section: the section path, as in read_note; \"\" for the text before the first heading. With old_text: search only this section."},
                    "heading_line": {"type": "integer", "minimum": 1, "description": "Instead of section: the line of the section's heading."},
                    "content": {"type": "string", "description": "By section: the new text."},
                    "mode": {"type": "string", "enum": ["replace", "append", "prepend"], "description": "By section; default replace."}
                },
                "required": ["note", "version"],
                "additionalProperties": false
            }),
            read_only: false,
        },
        ToolSpec {
            name: "set_property",
            description: "Set one property of a note, or remove it with value null. Only that property's lines change. Needs the version from read_note.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "note": {"type": "string", "description": NOTE},
                    "name": {"type": "string"},
                    "value": {"description": "Any JSON value; null removes the property."},
                    "version": {"type": "string", "description": "The version read_note gave."}
                },
                "required": ["note", "name", "value", "version"],
                "additionalProperties": false
            }),
            read_only: false,
        },
        ToolSpec {
            name: "move_note",
            description: "Move or rename a note. Every link that pointed to a file keeps pointing to it: links that would break or become ambiguous are rewritten in their target only.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "note": {"type": "string", "description": NOTE},
                    "to": {"type": "string", "description": "The new vault-relative path, or an existing folder to move into."},
                    "version": {"type": "string", "description": "Optional: refuse if the note changed since this version."}
                },
                "required": ["note", "to"],
                "additionalProperties": false
            }),
            read_only: false,
        },
        ToolSpec {
            name: "delete_note",
            description: "Move a note to the vault's .trash folder (never overwriting a file there) and list the links that now lead nowhere.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "note": {"type": "string", "description": NOTE},
                    "version": {"type": "string", "description": "Optional: refuse if the note changed since this version."}
                },
                "required": ["note"],
                "additionalProperties": false
            }),
            read_only: false,
        },
    ]
}
