//! The 8 KB reply limit and the opaque `page` tokens.
//!
//! A token carries the tool name, a hash of the call's other arguments and
//! an offset, hex-encoded. Nothing is stored on the server.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::ToolError;

/// The largest reply text, in bytes.
pub const REPLY_LIMIT: usize = 8192;

/// Where a paged call is: the call it belongs to and the offset reached.
#[derive(Debug, Clone)]
pub struct Cursor {
    tool: &'static str,
    args_hash: String,
    pub offset: usize,
}

impl Cursor {
    /// Starts at offset 0, or where `page` points. `args` are the call's
    /// arguments without `page`.
    pub fn new(tool: &'static str, args: &Value, page: Option<&str>) -> Result<Cursor, ToolError> {
        let args_hash = hash_args(args);
        let offset = match page {
            None => 0,
            Some(token) => {
                let decoded = decode(token).ok_or_else(ToolError::bad_page)?;
                let mut parts = decoded.splitn(3, ':');
                let (Some(t), Some(h), Some(o)) = (parts.next(), parts.next(), parts.next()) else {
                    return Err(ToolError::bad_page());
                };
                if t != tool || h != args_hash {
                    return Err(ToolError::bad_page());
                }
                o.parse().map_err(|_| ToolError::bad_page())?
            }
        };
        Ok(Cursor { tool, args_hash, offset })
    }

    /// The token for continuing at `offset`.
    pub fn token(&self, offset: usize) -> String {
        encode(&format!("{}:{}:{}", self.tool, self.args_hash, offset))
    }
}

fn hash_args(args: &Value) -> String {
    let mut args = args.clone();
    if let Value::Object(m) = &mut args {
        m.remove("page");
    }
    // serde_json maps are sorted, so equal arguments hash equally.
    let digest = Sha256::digest(args.to_string().as_bytes());
    digest[..6].iter().map(|b| format!("{b:02x}")).collect()
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn decode(s: &str) -> Option<String> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

pub fn size<T: Serialize>(value: &T) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |s| s.len())
}

/// Takes as many of the `total - start` remaining items as fit in one reply.
/// `build(start, end, next)` makes the page for items `start..end`, with the
/// token for the next page when `end < total`. At least one item is taken
/// when any remain.
pub fn fit_items<T: Serialize>(
    cursor: &Cursor,
    total: usize,
    build: impl Fn(usize, usize, Option<String>) -> T,
) -> T {
    let start = cursor.offset.min(total);
    let page_for = |end: usize| build(start, end, (end < total).then(|| cursor.token(end)));
    let whole = page_for(total);
    if size(&whole) <= REPLY_LIMIT || total - start <= 1 {
        return whole;
    }
    // Largest end in start+1..total that fits; size grows with end.
    let (mut lo, mut hi) = (start + 1, total - 1);
    if size(&page_for(lo)) > REPLY_LIMIT {
        return page_for(lo);
    }
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if size(&page_for(mid)) <= REPLY_LIMIT {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    page_for(lo)
}

/// Takes as much of `text[start..]` as fits in one reply, ending at a line
/// break where one fits. `build(chunk, next)` makes the page.
pub fn fit_text<T: Serialize>(
    cursor: &Cursor,
    text: &str,
    build: impl Fn(&str, Option<String>) -> T,
) -> Result<T, ToolError> {
    let start = cursor.offset;
    if start > text.len() || !text.is_char_boundary(start) {
        return Err(ToolError::bad_page());
    }
    let page_for = |end: usize| build(&text[start..end], (end < text.len()).then(|| cursor.token(end)));
    let whole = page_for(text.len());
    if size(&whole) <= REPLY_LIMIT {
        return Ok(whole);
    }
    let (mut lo, mut hi) = (start, text.len());
    while lo < hi {
        let mid = floor_char(text, lo + (hi - lo).div_ceil(2));
        if mid <= lo {
            break;
        }
        if size(&page_for(mid)) <= REPLY_LIMIT {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut end = floor_char(text, lo);
    if let Some(nl) = text[start..end].rfind('\n')
        && nl > 0
    {
        end = start + nl + 1;
    }
    if end == start {
        // Not even one character fits beside the other fields; send one anyway
        // so the reader always advances.
        end = start + text[start..].chars().next().map_or(0, char::len_utf8);
    }
    Ok(page_for(end))
}

fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn token_round_trip_and_mismatch() {
        let args = json!({"text": "a"});
        let c = Cursor::new("find_notes", &args, None).unwrap();
        let t = c.token(42);
        assert_eq!(Cursor::new("find_notes", &args, Some(&t)).unwrap().offset, 42);
        assert!(Cursor::new("find_notes", &json!({"text": "b"}), Some(&t)).is_err());
        assert!(Cursor::new("get_links", &args, Some(&t)).is_err());
        assert!(Cursor::new("find_notes", &args, Some("zz")).is_err());
    }

    #[test]
    fn items_fit_and_cover_once() {
        let items: Vec<String> = (0..500).map(|i| format!("item-{i:04}-{}", "x".repeat(40))).collect();
        let args = json!({});
        let mut seen = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let c = Cursor::new("t", &args, page.as_deref()).unwrap();
            let v = fit_items(&c, items.len(), |s, e, next| json!({"items": &items[s..e], "next_page": next}));
            assert!(size(&v) <= REPLY_LIMIT);
            seen.extend(v["items"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()));
            match v["next_page"].as_str() {
                Some(n) => page = Some(n.to_string()),
                None => break,
            }
        }
        assert_eq!(seen, items);
    }

    #[test]
    fn text_fits_and_rejoins() {
        let text: String = (0..2000).map(|i| format!("line {i} with \"quotes\" and é\n")).collect();
        let args = json!({});
        let mut joined = String::new();
        let mut page: Option<String> = None;
        loop {
            let c = Cursor::new("t", &args, page.as_deref()).unwrap();
            let v = fit_text(&c, &text, |chunk, next| json!({"content": chunk, "next_page": next})).unwrap();
            assert!(size(&v) <= REPLY_LIMIT);
            let chunk = v["content"].as_str().unwrap();
            assert!(chunk.ends_with('\n'));
            joined.push_str(chunk);
            match v["next_page"].as_str() {
                Some(n) => page = Some(n.to_string()),
                None => break,
            }
        }
        assert_eq!(joined, text);
    }
}
