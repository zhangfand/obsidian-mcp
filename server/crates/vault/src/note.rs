use std::ops::Range;

use crate::parse;
use crate::path::VaultPath;

/// A Markdown file in the vault, read at one moment.
#[derive(Clone, Debug)]
pub struct Note {
    pub path: VaultPath,
    /// The whole file, properties block included.
    pub text: String,
    /// Byte offset where the text after the properties block starts.
    pub body_start: usize,
    /// Byte range of the YAML between the `---` fences, when there is a
    /// properties block.
    pub properties_block: Option<Range<usize>>,
    pub properties: Properties,
    pub headings: Vec<Heading>,
    /// Every link occurrence, in file order.
    pub links: Vec<Link>,
    pub tags: Vec<Tag>,
    pub aliases: Vec<Alias>,
    pub block_ids: Vec<BlockId>,
}

/// A heading line. Only headings at the top level of the document count;
/// headings inside quotes, callouts or lists do not split sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// 1-based line in the file.
    pub line: usize,
    /// Byte offset of the heading line in the file.
    pub start: usize,
    /// Byte offset just after the heading's last line break (or the end of
    /// the file).
    pub end: usize,
}

/// A heading and everything under it, up to the next heading of the same or
/// a higher level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Index into `Note::headings`.
    pub heading: usize,
    /// The heading texts from the outermost parent down to this heading.
    pub chain: Vec<String>,
    /// Byte range in the file, heading line included.
    pub range: Range<usize>,
}

impl Section {
    /// The full section path, parents joined by `#`.
    pub fn path(&self) -> String {
        self.chain.join("#")
    }
}

/// The outcome of naming a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionLookup {
    Found(Section),
    /// Several headings match; their full section paths in file order.
    Ambiguous(Vec<String>),
    NotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkKind {
    /// `[[target]]`
    Wikilink,
    /// `![[target]]` or `![alt](target)`
    Embed,
    /// `[text](target)`
    Markdown,
}

/// One link occurrence as written in a note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// The target without `#heading`, `#^block` and `|display`; `""` for a
    /// link to the note itself (`[[#heading]]`). Markdown targets are
    /// percent-decoded.
    pub target: String,
    /// Section path after `#`, parents joined by `#`.
    pub heading: Option<String>,
    /// Block id after `#^`, without `^`.
    pub block: Option<String>,
    pub display: Option<String>,
    /// 1-based line in the file.
    pub line: usize,
    /// Markdown links resolve relative to the linking note's folder first.
    pub relative: bool,
    /// Byte range in the file of the target as written (percent-encoded,
    /// inside `<...>` if so). `None` for a form whose target is written
    /// elsewhere (reference-style Markdown links).
    pub span: Option<Range<usize>>,
    /// A Markdown destination written in `<...>`.
    pub angle: bool,
}

/// A tag without its leading `#`, spelled as written.
#[derive(Clone, Debug)]
pub struct Tag(pub String);

impl Tag {
    /// Accepts `#a/b` or `a/b`.
    pub fn new(raw: &str) -> Tag {
        Tag(raw.trim().trim_start_matches('#').to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when this tag is `parent` or nested below it (`a/b` is within
    /// `a`, `ab` is not). Tags compare case-insensitively.
    pub fn is_within(&self, parent: &Tag) -> bool {
        let me = self.0.to_lowercase();
        let parent = parent.0.to_lowercase();
        let parent = parent.trim_end_matches('/');
        me == parent || me.starts_with(&format!("{parent}/"))
    }
}

impl PartialEq for Tag {
    fn eq(&self, other: &Tag) -> bool {
        self.0.to_lowercase() == other.0.to_lowercase()
    }
}

/// Another name for a note, from its `aliases` property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias(pub String);

/// A block id (`^id` at the end of a block), without `^`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockId(pub String);

/// One entry of the properties block.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: PropertyValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Empty,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<PropertyValue>),
    Object(Vec<(String, PropertyValue)>),
}

impl PropertyValue {
    /// The value as text, for scalar values.
    pub fn as_text(&self) -> Option<String> {
        match self {
            PropertyValue::Bool(b) => Some(b.to_string()),
            PropertyValue::Integer(i) => Some(i.to_string()),
            PropertyValue::Float(f) => Some(f.to_string()),
            PropertyValue::Text(s) => Some(s.clone()),
            _ => None,
        }
    }
}

/// The properties block (YAML front matter), in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Properties {
    pub entries: Vec<Property>,
    /// False when a block is present but is not valid YAML; `entries` is
    /// then empty, as Obsidian shows no properties for it.
    pub valid: bool,
}

impl Properties {
    /// Property names compare case-insensitively.
    pub fn get(&self, name: &str) -> Option<&PropertyValue> {
        let name = name.to_lowercase();
        self.entries
            .iter()
            .find(|p| p.name.to_lowercase() == name)
            .map(|p| &p.value)
    }
}

impl Note {
    pub fn parse(path: VaultPath, text: String) -> Note {
        parse::parse_note(path, text)
    }

    /// The note name: file name without `.md`.
    pub fn name(&self) -> &str {
        self.path.stem()
    }

    /// The text after the properties block.
    pub fn body(&self) -> &str {
        &self.text[self.body_start..]
    }

    /// True when the note has `tag` or a tag nested below it.
    pub fn has_tag(&self, tag: &Tag) -> bool {
        self.tags.iter().any(|t| t.is_within(tag))
    }

    /// Case-insensitive search in the note name and the whole file text.
    pub fn contains_text(&self, needle: &str) -> bool {
        let needle = needle.to_lowercase();
        self.name().to_lowercase().contains(&needle) || self.text.to_lowercase().contains(&needle)
    }

    /// The properties block as written, without its fences; `""` when
    /// there is none.
    pub fn properties_text(&self) -> &str {
        self.properties_block.clone().map_or("", |r| &self.text[r])
    }

    /// The section whose heading is on `line` (1-based).
    pub fn section_at_line(&self, line: usize) -> Option<Section> {
        self.sections().into_iter().find(|s| self.headings[s.heading].line == line)
    }

    /// Every section, in file order.
    pub fn sections(&self) -> Vec<Section> {
        let mut out = Vec::with_capacity(self.headings.len());
        let mut stack: Vec<(u8, String)> = Vec::new();
        for (i, h) in self.headings.iter().enumerate() {
            while stack.last().is_some_and(|(lvl, _)| *lvl >= h.level) {
                stack.pop();
            }
            stack.push((h.level, h.text.clone()));
            let end = self.headings[i + 1..]
                .iter()
                .find(|n| n.level <= h.level)
                .map_or(self.text.len(), |n| n.start);
            out.push(Section {
                heading: i,
                chain: stack.iter().map(|(_, t)| t.clone()).collect(),
                range: h.start..end,
            });
        }
        out
    }

    /// Finds a section by section path: heading text, optionally preceded
    /// by parent headings joined with `#` (parents may skip levels).
    /// Case-insensitive, surrounding spaces ignored.
    pub fn find_section(&self, query: &str) -> SectionLookup {
        let wanted: Vec<String> = query
            .split('#')
            .map(normalize_heading)
            .filter(|s| !s.is_empty())
            .collect();
        let Some((last, parents)) = wanted.split_last() else {
            return SectionLookup::NotFound;
        };
        let mut found: Vec<Section> = self
            .sections()
            .into_iter()
            .filter(|s| {
                let chain: Vec<String> = s.chain.iter().map(|t| normalize_heading(t)).collect();
                let (own, ancestors) = chain.split_last().expect("a section has its own heading");
                own == last && is_subsequence(parents, ancestors)
            })
            .collect();
        match found.len() {
            0 => SectionLookup::NotFound,
            1 => SectionLookup::Found(found.remove(0)),
            _ => SectionLookup::Ambiguous(found.iter().map(Section::path).collect()),
        }
    }

    /// The shortest section path that names only this section; the full
    /// path when even that is shared.
    pub fn short_section_path(&self, section: &Section) -> String {
        for start in (0..section.chain.len()).rev() {
            let candidate = section.chain[start..].join("#");
            if let SectionLookup::Found(s) = self.find_section(&candidate)
                && s.heading == section.heading
            {
                return candidate;
            }
        }
        section.path()
    }
}

fn normalize_heading(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn is_subsequence(needles: &[String], hay: &[String]) -> bool {
    let mut it = hay.iter();
    needles.iter().all(|n| it.any(|h| h == n))
}
