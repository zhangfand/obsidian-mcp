//! Changes to one note's text, as byte splices of the original: the bytes
//! outside the target are never rebuilt.

use std::ops::Range;

use crate::note::{Note, Properties, Property, PropertyValue, Section};
use crate::parse;
use crate::path::VaultPath;

/// A part of a note that edits by section work on.
#[derive(Clone, Copy, Debug)]
pub enum Part<'a> {
    Section(&'a Section),
    /// The text before the first heading.
    Lead,
}

/// Where new text goes in a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionEdit {
    /// Replace the body, subsections included.
    Replace,
    /// Add after the body's last line.
    Append,
    /// Add before the body's first line.
    Prepend,
}

/// Why a property cannot be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyEditError {
    /// The properties block is not valid YAML; nothing may be rewritten.
    InvalidBlock,
    /// The property name is empty or cannot be a YAML key on one line.
    BadName,
    /// Written as YAML, the value would not read back as itself, or would
    /// change another property.
    NotRepresentable,
}

impl Note {
    /// The line ending the file uses: `\r\n` when its first line break is
    /// one, else `\n`.
    pub fn line_ending(&self) -> &'static str {
        line_ending_of(&self.text)
    }

    /// The lead: the text after the properties block and before the first
    /// heading (all of it when there is no heading).
    pub fn lead_range(&self) -> Range<usize> {
        self.body_start..self.headings.first().map_or(self.text.len(), |h| h.start)
    }

    /// The text below a part's heading line: for a section, from the end of
    /// its heading line to its end; for the lead, the lead.
    pub fn part_region(&self, part: Part<'_>) -> Range<usize> {
        match part {
            Part::Section(s) => self.headings[s.heading].end..s.range.end,
            Part::Lead => self.lead_range(),
        }
    }

    /// A part's body: its region without the blank lines at its start and
    /// at its end. A region of blank lines only gives the empty range at
    /// its end.
    pub fn body_range(&self, region: Range<usize>) -> Range<usize> {
        let text = &self.text;
        let blank = |r: Range<usize>| text[r].trim().is_empty();
        let mut start = region.start;
        while start < region.end {
            let end = text[start..region.end]
                .find('\n')
                .map_or(region.end, |i| start + i + 1);
            if !blank(start..end) {
                break;
            }
            start = end;
        }
        if start >= region.end {
            return region.end..region.end;
        }
        let mut end = region.end;
        while end > start {
            let lines = &text[start..end];
            let line_start = lines
                .strip_suffix('\n')
                .unwrap_or(lines)
                .rfind('\n')
                .map_or(start, |i| start + i + 1);
            if !blank(line_start..end) {
                break;
            }
            end = line_start;
        }
        start..end
    }

    /// The text with a part's body changed: replaced, or `content` added at
    /// its start or end. The heading line and the blank lines around the
    /// body stay; only the target bytes differ.
    pub fn edit_part(&self, part: Part<'_>, edit: SectionEdit, content: &str) -> String {
        let le = self.line_ending();
        let body = self.body_range(self.part_region(part));
        let mut new = with_line_ending(content, le);
        let (at, until) = match edit {
            SectionEdit::Replace => (body.start, body.end),
            SectionEdit::Append => (body.end, body.end),
            SectionEdit::Prepend => (body.start, body.start),
        };
        // The text before is the file's last line and has no line break.
        if !new.is_empty() && at > 0 && !self.text[..at].ends_with('\n') {
            new.insert_str(0, le);
        }
        let text_follows = until < self.text.len();
        let replaced_had_break = self.text[at..until].ends_with('\n');
        let inserting = edit != SectionEdit::Replace;
        let at_eof_without_break = until == self.text.len() && !self.text.ends_with('\n');
        if !new.is_empty()
            && !new.ends_with('\n')
            && (text_follows || replaced_had_break || (inserting && !at_eof_without_break))
        {
            new.push_str(le);
        }
        splice(&self.text, &[(at..until, new)])
    }

    /// Where `needle` occurs within `region` of the text, comparing `\r\n`
    /// and `\n` as equal; non-overlapping, in file order.
    pub fn find_text(&self, needle: &str, region: Range<usize>) -> Vec<Range<usize>> {
        let needle = needle.replace("\r\n", "\n");
        if needle.is_empty() {
            return Vec::new();
        }
        let hay = &self.text[region.clone()];
        // The text without the `\r` of each `\r\n`, and each byte's offset in the file.
        let mut flat = String::with_capacity(hay.len());
        let mut at: Vec<usize> = Vec::with_capacity(hay.len() + 1);
        let bytes = hay.as_bytes();
        for (i, ch) in hay.char_indices() {
            if ch == '\r' && bytes.get(i + 1) == Some(&b'\n') {
                continue;
            }
            flat.push(ch);
            at.extend(std::iter::repeat_n(region.start + i, ch.len_utf8()));
        }
        at.push(region.end);
        flat.match_indices(&needle)
            .map(|(i, m)| {
                let mut start = at[i];
                // A match that starts at the `\n` of a `\r\n` takes the `\r` too.
                if m.starts_with('\n') && start > 0 && self.text.as_bytes()[start - 1] == b'\r' {
                    start -= 1;
                }
                let last = at[i + m.len() - 1];
                let end = last + self.text[last..].chars().next().map_or(1, char::len_utf8);
                start..end
            })
            .collect()
    }

    /// The text with `range` replaced by `new`, written in the file's line
    /// ending.
    pub fn replace_text(&self, range: Range<usize>, new: &str) -> String {
        splice(
            &self.text,
            &[(range, with_line_ending(new, self.line_ending()))],
        )
    }

    /// The 1-based line of a byte offset.
    pub fn line_of(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())]
            .matches('\n')
            .count()
            + 1
    }

    /// The text with property `name` set to `value`, or removed for `None`.
    /// Only the lines of that property change; a new property becomes the
    /// block's last line; a block is added at the top when there is none.
    /// The result is parsed again and refused unless it reads back as
    /// exactly the intended properties.
    pub fn set_property(
        &self,
        name: &str,
        value: Option<&PropertyValue>,
    ) -> Result<String, PropertyEditError> {
        let name = name.trim();
        if name.is_empty() || name.contains(['\n', '\r']) {
            return Err(PropertyEditError::BadName);
        }
        if !self.properties.valid {
            return Err(PropertyEditError::InvalidBlock);
        }
        let le = self.line_ending();
        let new_text = match (&self.properties_block, self.property_span(name)) {
            (Some(_), Some((range, key))) => match value {
                Some(v) => splice(&self.text, &[(range, render_property(&key, v, le))]),
                None => splice(&self.text, &[(range, String::new())]),
            },
            (Some(block), None) => match value {
                Some(v) => {
                    let mut line = render_property(&yaml_key(name), v, le);
                    if !self.text[..block.end].ends_with('\n') && block.end > block.start {
                        line.insert_str(0, le);
                    }
                    splice(&self.text, &[(block.end..block.end, line)])
                }
                None => return Ok(self.text.clone()),
            },
            (None, _) => match value {
                Some(v) => format!(
                    "---{le}{}---{le}{}",
                    render_property(&yaml_key(name), v, le),
                    self.text
                ),
                None => return Ok(self.text.clone()),
            },
        };
        let expected: Vec<Property> = expected_properties(&self.properties, name, value);
        let reread = parse::parse_note(self.path.clone(), new_text.clone());
        if !reread.properties.valid || !same_properties(&reread.properties.entries, &expected) {
            return Err(PropertyEditError::NotRepresentable);
        }
        Ok(new_text)
    }

    /// The lines of top-level property `name` (case-insensitive) and its
    /// key as written: the key line plus the indented or `- ` lines under
    /// it, without trailing blank lines.
    pub fn property_span(&self, name: &str) -> Option<(Range<usize>, String)> {
        let block = self.properties_block.clone()?;
        let wanted = name.trim().to_lowercase();
        let lines = line_ranges(&self.text, block.clone());
        let mut i = 0;
        while i < lines.len() {
            let line = &self.text[lines[i].clone()];
            let Some((key_raw, key)) = top_level_key(line) else {
                i += 1;
                continue;
            };
            let mut last = i;
            let mut j = i + 1;
            while j < lines.len() {
                let l = &self.text[lines[j].clone()];
                let blank = l.trim().is_empty();
                let continues = l.starts_with([' ', '\t']) || l.starts_with('-');
                if !(blank || continues) {
                    break;
                }
                if !blank {
                    last = j;
                }
                j += 1;
            }
            if key.to_lowercase() == wanted {
                return Some((lines[i].start..lines[last].end, key_raw));
            }
            i = j;
        }
        None
    }
}

/// The text of a new note: a properties block holding `properties` (none
/// when empty), then `body`. `None` when a value cannot be written so that
/// it reads back as itself.
pub fn new_note_text(properties: &[Property], body: &str) -> Option<String> {
    let le = line_ending_of(body);
    let mut text = String::new();
    if !properties.is_empty() {
        text.push_str("---");
        text.push_str(le);
        for p in properties {
            if p.name.trim().is_empty() || p.name.contains(['\n', '\r']) {
                return None;
            }
            text.push_str(&render_property(&yaml_key(p.name.trim()), &p.value, le));
        }
        text.push_str("---");
        text.push_str(le);
    }
    text.push_str(body);
    let reread = parse::parse_note(
        VaultPath::parse("new.md").expect("static path"),
        text.clone(),
    );
    let expected: Vec<Property> = properties
        .iter()
        .map(|p| Property {
            name: p.name.trim().to_string(),
            value: p.value.clone(),
        })
        .collect();
    (reread.properties.valid && same_properties(&reread.properties.entries, &expected))
        .then_some(text)
}

fn line_ending_of(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

fn with_line_ending(content: &str, le: &str) -> String {
    let unix = content.replace("\r\n", "\n");
    if le == "\n" {
        unix
    } else {
        unix.replace('\n', le)
    }
}

/// Applies non-overlapping replacements given in any order.
pub(crate) fn splice(text: &str, edits: &[(Range<usize>, String)]) -> String {
    let mut sorted: Vec<&(Range<usize>, String)> = edits.iter().collect();
    sorted.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for (r, new) in sorted {
        out.push_str(&text[pos..r.start]);
        out.push_str(new);
        pos = r.end;
    }
    out.push_str(&text[pos..]);
    out
}

fn line_ranges(text: &str, within: Range<usize>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut pos = within.start;
    while pos < within.end {
        let end = text[pos..within.end]
            .find('\n')
            .map_or(within.end, |i| pos + i + 1);
        out.push(pos..end);
        pos = end;
    }
    out
}

/// For a line starting a top-level mapping entry: the key as written and
/// the key's value.
fn top_level_key(line: &str) -> Option<(String, String)> {
    let first = line.chars().next()?;
    if first.is_whitespace() || first == '#' || first == '-' {
        return None;
    }
    if first == '"' || first == '\'' {
        let close = line[1..].find(first)? + 1;
        let raw = &line[..=close];
        if !line[close + 1..].trim_start().starts_with(':') {
            return None;
        }
        let key = raw[1..raw.len() - 1].to_string();
        return Some((raw.to_string(), key));
    }
    let bytes = line.as_bytes();
    let colon = (0..bytes.len())
        .find(|&i| bytes[i] == b':' && bytes.get(i + 1).is_none_or(|c| c.is_ascii_whitespace()))?;
    let raw = line[..colon].trim_end();
    Some((raw.to_string(), raw.to_string()))
}

/// A key for a new property: plain when YAML reads it back unchanged.
fn yaml_key(name: &str) -> String {
    if plain_ok(name) && !name.contains(['#', ':']) {
        name.to_string()
    } else {
        quoted(name)
    }
}

fn render_property(key: &str, value: &PropertyValue, le: &str) -> String {
    match value {
        PropertyValue::List(items) if items.is_empty() => format!("{key}: []{le}"),
        PropertyValue::List(items) if items.iter().all(is_scalar) => {
            let mut s = format!("{key}:{le}");
            for item in items {
                s.push_str(&format!("  - {}{le}", scalar(item)));
            }
            s
        }
        PropertyValue::Empty => format!("{key}:{le}"),
        v if is_scalar(v) => format!("{key}: {}{le}", scalar(v)),
        other => {
            // Nested values: a flow-style line, which JSON-like syntax keeps exact.
            format!("{key}: {}{le}", flow(other))
        }
    }
}

fn is_scalar(v: &PropertyValue) -> bool {
    !matches!(v, PropertyValue::List(_) | PropertyValue::Object(_))
}

fn scalar(v: &PropertyValue) -> String {
    match v {
        PropertyValue::Empty => "null".into(),
        PropertyValue::Bool(b) => b.to_string(),
        PropertyValue::Integer(i) => i.to_string(),
        PropertyValue::Float(f) if f.is_finite() && f.fract() == 0.0 => format!("{f:.1}"),
        PropertyValue::Float(f) => f.to_string(),
        PropertyValue::Text(t) if plain_ok(t) => t.clone(),
        PropertyValue::Text(t) => quoted(t),
        _ => flow(v),
    }
}

fn flow(v: &PropertyValue) -> String {
    match v {
        PropertyValue::List(items) => format!(
            "[{}]",
            items.iter().map(flow_item).collect::<Vec<_>>().join(", ")
        ),
        PropertyValue::Object(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(k, v)| format!("{}: {}", quoted(k), flow_item(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => scalar(other),
    }
}

fn flow_item(v: &PropertyValue) -> String {
    match v {
        PropertyValue::Text(t) => quoted(t),
        other if is_scalar(other) => scalar(other),
        other => flow(other),
    }
}

/// True when `t` written plain reads back as the same text.
fn plain_ok(t: &str) -> bool {
    if t.is_empty() || t.trim() != t || t.contains(['\n', '\r', '\t']) {
        return false;
    }
    let first = t.chars().next().expect("not empty");
    if "-?:,[]{}#&*!|>'\"%@`".contains(first)
        || t.contains(": ")
        || t.contains(" #")
        || t.ends_with(':')
    {
        return false;
    }
    matches!(yaml_serde::from_str::<yaml_serde::Value>(&format!("k: {t}")),
        Ok(yaml_serde::Value::Mapping(m)) if m.get("k").and_then(|v| v.as_str()) == Some(t))
}

/// A YAML double-quoted string; JSON's escapes are valid YAML escapes.
fn quoted(t: &str) -> String {
    serde_json::to_string(t).expect("strings serialize")
}

fn expected_properties(
    old: &Properties,
    name: &str,
    value: Option<&PropertyValue>,
) -> Vec<Property> {
    let wanted = name.to_lowercase();
    let mut out = Vec::new();
    let mut replaced = false;
    for p in &old.entries {
        if p.name.to_lowercase() == wanted && !replaced {
            replaced = true;
            if let Some(v) = value {
                out.push(Property {
                    name: p.name.clone(),
                    value: v.clone(),
                });
            }
        } else {
            out.push(p.clone());
        }
    }
    if !replaced && let Some(v) = value {
        out.push(Property {
            name: name.to_string(),
            value: v.clone(),
        });
    }
    out
}

fn same_properties(a: &[Property], b: &[Property]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.name == y.name && same_value(&x.value, &y.value))
}

fn same_value(a: &PropertyValue, b: &PropertyValue) -> bool {
    use PropertyValue as P;
    match (a, b) {
        (P::Float(x), P::Float(y)) => x == y || (x.is_nan() && y.is_nan()),
        (P::Float(x), P::Integer(y)) | (P::Integer(y), P::Float(x)) => *x == *y as f64,
        (P::List(x), P::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_value(p, q))
        }
        (P::Object(x), P::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((k1, v1), (k2, v2))| k1 == k2 && same_value(v1, v2))
        }
        _ => a == b,
    }
}
