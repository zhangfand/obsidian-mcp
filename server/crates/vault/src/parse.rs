//! Obsidian syntax inside one file.
//!
//! pulldown-cmark finds the document structure: code, math, headings and
//! Markdown links. Wikilinks and embeds come from our own scanner because
//! pulldown-cmark 0.13 gets two Obsidian forms wrong: `[[n\|t]]` inside a
//! table (it yields the target `n\`) and a `[[...]]` that spans lines
//! (Obsidian never links across a line break).

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag as MdTag, TagEnd};

use crate::note::{
    Alias, BlockId, Heading, Link, LinkKind, Note, Properties, Property, PropertyValue, Tag,
};
use crate::path::VaultPath;

pub(crate) fn parse_note(path: VaultPath, text: String) -> Note {
    let lines = LineIndex::new(&text);
    let (front, body_start) = split_front_matter(&text);
    let properties = front.map(parse_properties).unwrap_or_default();

    let properties_block = front.map(|f| {
        let offset = f.as_ptr() as usize - text.as_ptr() as usize;
        offset..offset + f.len()
    });
    let mut links = Vec::new();
    if let Some(front) = front {
        let offset = front.as_ptr() as usize - text.as_ptr() as usize;
        for (range, link) in scan_wikilinks(front, &[]) {
            links.push(link.placed(lines.line_of(offset + range.start), offset));
        }
    }

    let body = &text[body_start..];
    let structure = scan_structure(body);
    let mut body_links: Vec<(usize, Link)> = Vec::new();
    let mut skip_for_tags = structure.code.clone();
    for (range, link) in scan_wikilinks(body, &structure.code) {
        skip_for_tags.push(range.clone());
        body_links.push((
            range.start,
            link.placed(lines.line_of(body_start + range.start), body_start),
        ));
    }
    for (start, link) in structure.md_links {
        body_links.push((
            start,
            link.placed(lines.line_of(body_start + start), body_start),
        ));
    }
    body_links.sort_by_key(|(start, _)| *start);
    links.extend(body_links.into_iter().map(|(_, l)| l));

    let headings = structure
        .headings
        .into_iter()
        .map(|(level, range)| {
            let abs = body_start + range.start;
            let line_end = text[abs..].find('\n').map_or(text.len(), |i| abs + i);
            let mut end = (body_start + range.end).max(line_end.min(text.len()));
            if !text[..end].ends_with('\n') {
                end = text[end..].find('\n').map_or(text.len(), |i| end + i + 1);
            }
            Heading {
                level,
                text: heading_text(&text[abs..line_end]),
                line: lines.line_of(abs),
                start: abs,
                end,
            }
        })
        .collect();

    let mut tags: Vec<Tag> = Vec::new();
    for raw in property_tags(&properties)
        .into_iter()
        .chain(scan_tags(body, &skip_for_tags))
    {
        let tag = Tag::new(&raw);
        if !tag.as_str().is_empty() && !tags.contains(&tag) {
            tags.push(tag);
        }
    }

    let aliases = property_aliases(&properties);
    let block_ids = scan_block_ids(body, &structure.code);

    Note {
        path,
        text,
        body_start,
        properties_block,
        properties,
        headings,
        links,
        tags,
        aliases,
        block_ids,
    }
}

struct LineIndex(Vec<usize>);

impl LineIndex {
    fn new(text: &str) -> LineIndex {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex(starts)
    }

    /// 1-based line number of a byte offset.
    fn line_of(&self, offset: usize) -> usize {
        self.0.partition_point(|&s| s <= offset)
    }
}

/// Returns the YAML between the `---` fences and where the body starts.
/// The block must start on the first line of the file.
fn split_front_matter(text: &str) -> (Option<&str>, usize) {
    let first_end = match text.find('\n') {
        Some(i) => i,
        None => return (None, 0),
    };
    if text[..first_end].trim_end() != "---" {
        return (None, 0);
    }
    let yaml_start = first_end + 1;
    let mut pos = yaml_start;
    while pos <= text.len() {
        let end = text[pos..].find('\n').map_or(text.len(), |i| pos + i);
        if text[pos..end].trim_end() == "---" {
            let body_start = if end < text.len() { end + 1 } else { end };
            return (Some(&text[yaml_start..pos]), body_start);
        }
        if end >= text.len() {
            break;
        }
        pos = end + 1;
    }
    (None, 0)
}

fn parse_properties(yaml: &str) -> Properties {
    if yaml.trim().is_empty() {
        return Properties {
            entries: Vec::new(),
            valid: true,
        };
    }
    match yaml_serde::from_str::<yaml_serde::Value>(yaml) {
        Ok(yaml_serde::Value::Mapping(map)) => Properties {
            entries: map
                .into_iter()
                .map(|(k, v)| Property {
                    name: yaml_key(&k),
                    value: from_yaml(v),
                })
                .collect(),
            valid: true,
        },
        Ok(yaml_serde::Value::Null) => Properties {
            entries: Vec::new(),
            valid: true,
        },
        _ => Properties {
            entries: Vec::new(),
            valid: false,
        },
    }
}

fn yaml_key(k: &yaml_serde::Value) -> String {
    match from_yaml(k.clone()).as_text() {
        Some(s) => s,
        None => yaml_serde::to_string(k)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn from_yaml(v: yaml_serde::Value) -> PropertyValue {
    use yaml_serde::Value as Y;
    match v {
        Y::Null => PropertyValue::Empty,
        Y::Bool(b) => PropertyValue::Bool(b),
        Y::Number(n) => match n.as_i64() {
            Some(i) => PropertyValue::Integer(i),
            None => PropertyValue::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        Y::String(s) => PropertyValue::Text(s),
        Y::Sequence(items) => PropertyValue::List(items.into_iter().map(from_yaml).collect()),
        Y::Mapping(map) => PropertyValue::Object(
            map.into_iter()
                .map(|(k, v)| (yaml_key(&k), from_yaml(v)))
                .collect(),
        ),
        Y::Tagged(t) => from_yaml(t.value),
    }
}

fn texts_of(value: &PropertyValue) -> Vec<String> {
    match value {
        PropertyValue::List(items) => items.iter().filter_map(PropertyValue::as_text).collect(),
        other => other.as_text().into_iter().collect(),
    }
}

/// `tags` (and the older `tag`): a list, or one string of names separated
/// by commas or spaces.
fn property_tags(props: &Properties) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["tags", "tag"] {
        if let Some(v) = props.get(key) {
            for s in texts_of(v) {
                out.extend(
                    s.split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|t| !t.is_empty())
                        .map(str::to_string),
                );
            }
        }
    }
    out
}

/// `aliases` (and the older `alias`): a list, or a single string.
fn property_aliases(props: &Properties) -> Vec<Alias> {
    let mut out: Vec<Alias> = Vec::new();
    for key in ["aliases", "alias"] {
        if let Some(v) = props.get(key) {
            for s in texts_of(v) {
                let s = s.trim();
                if !s.is_empty() && !out.iter().any(|a| a.0 == s) {
                    out.push(Alias(s.to_string()));
                }
            }
        }
    }
    out
}

impl Link {
    /// Sets the line and moves the target span from scan-local to file
    /// offsets.
    fn placed(mut self, line: usize, base: usize) -> Link {
        self.line = line;
        self.span = self.span.map(|r| r.start + base..r.end + base);
        self
    }
}

struct Structure {
    /// Byte ranges (in the body) of code and math, where no Obsidian syntax
    /// counts.
    code: Vec<Range<usize>>,
    /// Top-level headings: level and byte range.
    headings: Vec<(u8, Range<usize>)>,
    /// Markdown links and images with their start offset.
    md_links: Vec<(usize, Link)>,
}

fn scan_structure(body: &str) -> Structure {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_WIKILINKS);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_MATH);

    let mut code = Vec::new();
    let mut headings = Vec::new();
    let mut md_links = Vec::new();
    let mut container_depth = 0usize;
    // A Markdown link being read: start, kind, type, destination, display text.
    let mut open_link: Option<(usize, LinkKind, LinkType, String, String)> = None;

    for (event, range) in Parser::new_ext(body, opts).into_offset_iter() {
        match event {
            Event::Code(_) | Event::InlineMath(_) | Event::DisplayMath(_) => code.push(range),
            Event::Start(MdTag::CodeBlock(_)) => code.push(range),
            Event::Start(MdTag::BlockQuote(_))
            | Event::Start(MdTag::List(_))
            | Event::Start(MdTag::FootnoteDefinition(_)) => container_depth += 1,
            Event::End(TagEnd::BlockQuote(_))
            | Event::End(TagEnd::List(_))
            | Event::End(TagEnd::FootnoteDefinition) => container_depth -= 1,
            Event::Start(MdTag::Heading { level, .. }) if container_depth == 0 => {
                headings.push((level as u8, range));
            }
            Event::Start(MdTag::Link {
                link_type,
                dest_url,
                ..
            }) if is_markdown_link(link_type) => {
                open_link = Some((
                    range.start,
                    LinkKind::Markdown,
                    link_type,
                    dest_url.to_string(),
                    String::new(),
                ));
            }
            Event::Start(MdTag::Image {
                link_type,
                dest_url,
                ..
            }) if is_markdown_link(link_type) => {
                open_link = Some((
                    range.start,
                    LinkKind::Embed,
                    link_type,
                    dest_url.to_string(),
                    String::new(),
                ));
            }
            Event::Text(t) => {
                if let Some((_, _, _, _, display)) = open_link.as_mut() {
                    display.push_str(&t);
                }
            }
            Event::End(TagEnd::Link) | Event::End(TagEnd::Image) => {
                if let Some((start, kind, link_type, dest, display)) = open_link.take()
                    && let Some(mut link) = markdown_link(kind, &dest, display)
                {
                    if link_type == LinkType::Inline
                        && let Some((span, angle)) = inline_dest_span(body, start)
                        && percent_decode(&body[span.clone()]).trim() == link.target
                    {
                        link.span = Some(span);
                        link.angle = angle;
                    }
                    md_links.push((start, link));
                }
            }
            _ => {}
        }
    }
    Structure {
        code,
        headings,
        md_links,
    }
}

fn is_markdown_link(t: LinkType) -> bool {
    matches!(
        t,
        LinkType::Inline
            | LinkType::Reference
            | LinkType::Collapsed
            | LinkType::Shortcut
            | LinkType::ReferenceUnknown
            | LinkType::CollapsedUnknown
            | LinkType::ShortcutUnknown
    )
}

/// A Markdown link destination as an Obsidian link; `None` for URLs.
fn markdown_link(kind: LinkKind, dest: &str, display: String) -> Option<Link> {
    let dest = dest.trim();
    if dest.is_empty() || has_url_scheme(dest) {
        return None;
    }
    let decoded = percent_decode(dest);
    let (target, fragment) = match decoded.find('#') {
        Some(i) => (decoded[..i].to_string(), Some(decoded[i + 1..].to_string())),
        None => (decoded, None),
    };
    let (heading, block) = split_fragment(fragment.as_deref());
    if target.is_empty() && heading.is_none() && block.is_none() {
        return None;
    }
    Some(Link {
        kind,
        target: target.trim().to_string(),
        heading,
        block,
        display: Some(display).filter(|d| !d.is_empty()),
        line: 0,
        relative: true,
        span: None,
        angle: false,
    })
}

/// Where the path part of an inline link's destination is written, for
/// `[text](dest#frag "title")` starting at `start`; and whether it is in
/// `<...>`. `None` when the source does not have that shape.
fn inline_dest_span(src: &str, start: usize) -> Option<(Range<usize>, bool)> {
    let b = src.as_bytes();
    let mut i = start;
    if b.get(i) == Some(&b'!') {
        i += 1;
    }
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut depth = 0usize;
    let close = loop {
        match b.get(i)? {
            b'\\' => i += 1,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break i;
                }
            }
            _ => {}
        }
        i += 1;
    };
    if b.get(close + 1) != Some(&b'(') {
        return None;
    }
    let mut j = close + 2;
    while matches!(b.get(j), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        j += 1;
    }
    let (dest_start, dest_end, angle) = if b.get(j) == Some(&b'<') {
        let end = j + 1 + src[j + 1..].find('>')?;
        (j + 1, end, true)
    } else {
        let mut k = j;
        let mut parens = 0usize;
        while let Some(&c) = b.get(k) {
            match c {
                b'\\' => k += 1,
                b'(' => parens += 1,
                b')' if parens == 0 => break,
                b')' => parens -= 1,
                c if c.is_ascii_whitespace() => break,
                _ => {}
            }
            k += 1;
        }
        (j, k.min(b.len()), false)
    };
    let raw = &src[dest_start..dest_end];
    let path_end = dest_start + raw.find('#').unwrap_or(raw.len());
    Some((dest_start..path_end, angle))
}

fn has_url_scheme(dest: &str) -> bool {
    let Some(colon) = dest.find(':') else {
        return false;
    };
    let scheme = &dest[..colon];
    scheme.len() > 1
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// Splits what follows the first `#`: `^id` is a block, anything else a
/// section path (empty parts dropped).
fn split_fragment(fragment: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(f) = fragment else {
        return (None, None);
    };
    let f = f.trim();
    if let Some(block) = f.strip_prefix('^') {
        let block = block.trim();
        return (None, Some(block.to_string()).filter(|b| !b.is_empty()));
    }
    let heading = f
        .split('#')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("#");
    (Some(heading).filter(|h| !h.is_empty()), None)
}

fn inside(pos: usize, ranges: &[Range<usize>]) -> bool {
    ranges.iter().any(|r| r.contains(&pos))
}

/// Finds `[[...]]` and `![[...]]` on single lines outside `skip`.
fn scan_wikilinks(text: &str, skip: &[Range<usize>]) -> Vec<(Range<usize>, Link)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(found) = text[i..].find("[[") {
        let open = i + found;
        i = open + 2;
        if inside(open, skip) || (open > 0 && bytes[open - 1] == b'\\') {
            continue;
        }
        let rest = &text[open + 2..];
        let Some(close) = rest.find("]]") else { break };
        let inner = &rest[..close];
        if inner.contains('\n') || inner.contains("[[") {
            continue;
        }
        let embed = open > 0 && bytes[open - 1] == b'!';
        let start = if embed { open - 1 } else { open };
        let end = open + 2 + close + 2;
        i = end;
        if let Some(mut link) = wikilink(inner, embed) {
            link.span = link.span.map(|r| r.start + open + 2..r.end + open + 2);
            out.push((start..end, link));
        }
    }
    out
}

fn wikilink(inner: &str, embed: bool) -> Option<Link> {
    let (target_part, display) = match inner.find('|') {
        Some(i) => {
            // In tables Obsidian writes `[[n\|t]]`; the `\` is not part of the name.
            let target = inner[..i].strip_suffix('\\').unwrap_or(&inner[..i]);
            (
                target,
                Some(inner[i + 1..].trim().to_string()).filter(|d| !d.is_empty()),
            )
        }
        None => (inner, None),
    };
    let (target, fragment) = match target_part.find('#') {
        Some(i) => (&target_part[..i], Some(&target_part[i + 1..])),
        None => (target_part, None),
    };
    let (heading, block) = split_fragment(fragment);
    let target = target.trim();
    if target.is_empty() && heading.is_none() && block.is_none() {
        return None;
    }
    let offset = target.as_ptr() as usize - inner.as_ptr() as usize;
    Some(Link {
        kind: if embed {
            LinkKind::Embed
        } else {
            LinkKind::Wikilink
        },
        target: target.to_string(),
        heading,
        block,
        display,
        line: 0,
        relative: false,
        span: Some(offset..offset + target.len()),
        angle: false,
    })
}

/// Inline `#tags`: after a line start or whitespace, made of letters,
/// digits, `_`, `-` and `/`, with at least one character that is not a
/// digit.
fn scan_tags(text: &str, skip: &[Range<usize>]) -> Vec<String> {
    let mut out = Vec::new();
    for (pos, _) in text.match_indices('#') {
        if inside(pos, skip) {
            continue;
        }
        let before = text[..pos].chars().next_back();
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let rest = &text[pos + 1..];
        let len: usize = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
            .map(char::len_utf8)
            .sum();
        let tag = rest[..len].trim_end_matches('/');
        if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit() || c == '/') {
            out.push(tag.to_string());
        }
    }
    out
}

/// `^id` at the end of a line, after whitespace or alone on the line.
fn scan_block_ids(text: &str, skip: &[Range<usize>]) -> Vec<BlockId> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if inside(start, skip) {
            continue;
        }
        let trimmed = line.trim_end();
        let Some(caret) = trimmed.rfind('^') else {
            continue;
        };
        let id = &trimmed[caret + 1..];
        let before_ok = caret == 0 || trimmed[..caret].ends_with(char::is_whitespace);
        if before_ok && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            out.push(BlockId(id.to_string()));
        }
    }
    out
}

/// The heading text as written: ATX `#` marks and a closing `#` run removed,
/// or the first line of a setext heading.
fn heading_text(line: &str) -> String {
    let t = line.trim();
    if t.starts_with('#') {
        let t = t.trim_start_matches('#');
        let t = t.trim();
        let without_closing = t.trim_end_matches('#');
        if without_closing.is_empty() || without_closing.ends_with(char::is_whitespace) {
            return without_closing.trim().to_string();
        }
        t.to_string()
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::SectionLookup;

    fn note(text: &str) -> Note {
        Note::parse(VaultPath::parse("n.md").unwrap(), text.to_string())
    }

    /// Kind, target, heading, block, display.
    type Parts<'a> = (
        LinkKind,
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
    );

    fn targets(n: &Note) -> Vec<Parts<'_>> {
        n.links
            .iter()
            .map(|l| {
                (
                    l.kind,
                    l.target.as_str(),
                    l.heading.as_deref(),
                    l.block.as_deref(),
                    l.display.as_deref(),
                )
            })
            .collect()
    }

    #[test]
    fn wikilink_forms() {
        let n = note(
            "[[a#h]] [[b#^blk]] [[c|text]] ![[d]] [[e#h|t]] [[#self]] [[f#A#B]] ![[img.png]]\n",
        );
        use LinkKind::*;
        assert_eq!(
            targets(&n),
            vec![
                (Wikilink, "a", Some("h"), None, None),
                (Wikilink, "b", None, Some("blk"), None),
                (Wikilink, "c", None, None, Some("text")),
                (Embed, "d", None, None, None),
                (Wikilink, "e", Some("h"), None, Some("t")),
                (Wikilink, "", Some("self"), None, None),
                (Wikilink, "f", Some("A#B"), None, None),
                (Embed, "img.png", None, None, None),
            ]
        );
    }

    #[test]
    fn table_pipe_and_line_breaks() {
        let n = note("| a | b |\n|---|---|\n| [[n\\|t]] | x |\n\n[[broken\nacross]]\n");
        assert_eq!(
            targets(&n),
            vec![(LinkKind::Wikilink, "n", None, None, Some("t"))]
        );
        assert_eq!(n.links[0].line, 3);
    }

    #[test]
    fn code_hides_syntax() {
        let n = note(
            "`[[x]] #t1`\n\n```\n[[y]]\n#t2\n## not a heading\n```\n\n    [[z]] #t3\n\n$[[m]]$\n\n# Real\n",
        );
        assert!(n.links.is_empty(), "{:?}", n.links);
        assert!(n.tags.is_empty(), "{:?}", n.tags);
        assert_eq!(n.headings.len(), 1);
        assert_eq!(n.headings[0].text, "Real");
    }

    #[test]
    fn markdown_links() {
        let n = note(
            "[t](Folder/My%20Note.md#Sec) [u](<Other Note.md>) ![i](pic.png) [w](https://x.org/a.md) [m](mailto:a@b)\n",
        );
        use LinkKind::*;
        assert_eq!(
            targets(&n),
            vec![
                (Markdown, "Folder/My Note.md", Some("Sec"), None, Some("t")),
                (Markdown, "Other Note.md", None, None, Some("u")),
                (Embed, "pic.png", None, None, Some("i")),
            ]
        );
    }

    #[test]
    fn properties_tags_aliases() {
        let n = note(
            "---\ntags: [project/alpha, Active]\naliases:\n  - Alpha Project\n  - AP\nstatus: done\ncount: 3\nrelated: \"[[Beta]]\"\n---\n# Title\nbody #inline/tag and #123 and x#no and #Active\n",
        );
        let tags: Vec<&str> = n.tags.iter().map(Tag::as_str).collect();
        assert_eq!(tags, vec!["project/alpha", "Active", "inline/tag"]);
        assert_eq!(
            n.aliases,
            vec![Alias("Alpha Project".into()), Alias("AP".into())]
        );
        assert_eq!(
            n.properties.get("Status"),
            Some(&PropertyValue::Text("done".into()))
        );
        assert_eq!(n.properties.get("count"), Some(&PropertyValue::Integer(3)));
        assert_eq!(n.links.len(), 1);
        assert_eq!(n.links[0].target, "Beta");
        assert_eq!(n.links[0].line, 8);
        assert_eq!(n.headings[0].line, 10);
        assert!(n.has_tag(&Tag::new("#project")));
        assert!(!n.has_tag(&Tag::new("proj")));
    }

    #[test]
    fn tags_as_string() {
        let n = note("---\ntags: a, b c\n---\n");
        let tags: Vec<&str> = n.tags.iter().map(Tag::as_str).collect();
        assert_eq!(tags, vec!["a", "b", "c"]);
    }

    #[test]
    fn invalid_yaml_gives_no_properties() {
        let n = note("---\n: : [\n---\nbody\n");
        assert!(!n.properties.valid);
        assert!(n.properties.entries.is_empty());
        assert_eq!(n.body(), "body\n");
    }

    #[test]
    fn block_ids() {
        let n = note("para text ^abc-1\n\n^solo\n\nnot^this\n");
        assert_eq!(
            n.block_ids,
            vec![BlockId("abc-1".into()), BlockId("solo".into())]
        );
    }

    #[test]
    fn sections_and_lookup() {
        let n = note("# Week 1\n## Plan\na\n## Notes\nb\n# Week 2\n## Plan\nc\n### Deep\nd\n");
        let secs = n.sections();
        assert_eq!(secs.len(), 6);
        assert_eq!(&n.text[secs[1].range.clone()], "## Plan\na\n");
        assert_eq!(&n.text[secs[4].range.clone()], "## Plan\nc\n### Deep\nd\n");
        assert_eq!(
            n.find_section("plan"),
            SectionLookup::Ambiguous(vec!["Week 1#Plan".into(), "Week 2#Plan".into()])
        );
        match n.find_section("week 2 # plan") {
            SectionLookup::Found(s) => assert_eq!(s.heading, 4),
            other => panic!("{other:?}"),
        }
        match n.find_section("Week 2#Deep") {
            SectionLookup::Found(s) => assert_eq!(s.heading, 5),
            other => panic!("{other:?}"),
        }
        assert_eq!(n.find_section("nope"), SectionLookup::NotFound);
        assert_eq!(n.short_section_path(&secs[1]), "Week 1#Plan");
        assert_eq!(n.short_section_path(&secs[2]), "Notes");
    }

    #[test]
    fn headings_in_quotes_do_not_split() {
        let n = note("# A\n> # quoted\n- # in list\n## B\n");
        let texts: Vec<&str> = n.headings.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, vec!["A", "B"]);
    }

    #[test]
    fn setext_and_closing_hashes() {
        let n = note("Title\n=====\n## Sub ##\n");
        let texts: Vec<&str> = n.headings.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, vec!["Title", "Sub"]);
    }
}
