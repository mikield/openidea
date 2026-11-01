//! Coordinate conversion between the client (UTF-8 byte offsets, as GPUI's
//! rope uses) and the protocol (UTF-16 code units, see `ide_core::text`).

use ide_core::{Pos, TextChange};

/// UTF-16 length of `s`.
pub fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

/// UTF-16 offset of byte offset `byte` (clamped to `text`, floored to a char boundary).
pub fn byte_to_utf16(text: &str, byte: usize) -> u32 {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    utf16_len(&text[..byte])
}

/// Byte offset of UTF-16 offset `units` (clamped; a split surrogate pair snaps forward).
pub fn utf16_to_byte(text: &str, units: u32) -> usize {
    let mut seen = 0u32;
    for (i, c) in text.char_indices() {
        if seen >= units {
            return i;
        }
        seen += c.len_utf16() as u32;
    }
    text.len()
}

/// Byte offset of a protocol position (line / UTF-16 column), clamped to the line.
pub fn pos_to_byte(text: &str, pos: Pos) -> usize {
    let mut start = 0usize;
    for _ in 0..pos.line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len(),
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    start + utf16_to_byte(&text[start..end], pos.col)
}

/// Protocol position of byte offset `byte`.
pub fn byte_to_pos(text: &str, byte: usize) -> Pos {
    let byte = byte.min(text.len());
    let before = &text[..byte];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = before.matches('\n').count() as u32;
    Pos::new(line, byte_to_utf16(text, byte) - byte_to_utf16(text, line_start))
}

/// The single change turning `old` into `new` (common prefix / suffix trimmed),
/// in UTF-16 offsets of `old`; `None` when they are equal.
///
/// The editor reports "text changed" without a delta, so the client derives
/// one. O(n) per edit: fine for a spike, replaced by editor deltas later.
pub fn diff(old: &str, new: &str) -> Option<TextChange> {
    if old == new {
        return None;
    }
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(&old[prefix..], &new[prefix..]);
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    Some(TextChange {
        from: byte_to_utf16(old, prefix),
        to: byte_to_utf16(old, old_end),
        insert: new[prefix..new_end].to_owned(),
    })
}

/// Applies protocol changes (UTF-16 offsets into `text` *before* the set) in place.
pub fn apply(text: &mut String, changes: &[TextChange]) {
    // Non-overlapping and ordered: apply back to front so earlier offsets hold.
    let mut spans: Vec<(usize, usize, &str)> =
        changes.iter().map(|c| (utf16_to_byte(text, c.from), utf16_to_byte(text, c.to), c.insert.as_str())).collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.0));
    for (from, to, insert) in spans {
        text.replace_range(from..to.max(from), insert);
    }
}

fn common_prefix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for ((i, ca), cb) in a.char_indices().zip(b.chars()) {
        if ca != cb {
            return i;
        }
        n = i + ca.len_utf8();
    }
    n
}

fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (ca, cb) in a.chars().rev().zip(b.chars().rev()) {
        if ca != cb {
            break;
        }
        n += ca.len_utf8();
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_round_trip_with_astral_and_cyrillic() {
        let s = "aб😀c";
        assert_eq!(utf16_len(s), 5); // a(1) б(1) 😀(2) c(1)
        assert_eq!(byte_to_utf16(s, s.find('c').unwrap()), 4);
        assert_eq!(utf16_to_byte(s, 4), s.find('c').unwrap());
        assert_eq!(byte_to_utf16(s, 2), 1); // inside "б" floors to its start
    }

    #[test]
    fn positions() {
        let s = "fn main() {\n    let я = 1;\n}";
        let b = s.find('=').unwrap();
        let p = byte_to_pos(s, b);
        assert_eq!(p, Pos::new(1, 10));
        assert_eq!(pos_to_byte(s, p), b);
        assert_eq!(pos_to_byte(s, Pos::new(9, 0)), s.len());
        assert_eq!(pos_to_byte(s, Pos::new(0, 999)), s.find('\n').unwrap());
    }

    #[test]
    fn diff_insert_delete_replace() {
        assert_eq!(diff("abc", "abc"), None);
        assert_eq!(diff("abc", "abXc"), Some(TextChange { from: 2, to: 2, insert: "X".into() }));
        assert_eq!(diff("abXc", "abc"), Some(TextChange { from: 2, to: 3, insert: String::new() }));
        assert_eq!(diff("😀a", "😀b"), Some(TextChange { from: 2, to: 3, insert: "b".into() }));
        // repeated chars: prefix wins, the change stays minimal
        assert_eq!(diff("aaa", "aaaa"), Some(TextChange { from: 3, to: 3, insert: "a".into() }));
    }

    #[test]
    fn diff_then_apply_reproduces_new() {
        let cases = [("", "x"), ("hello", ""), ("привіт світ", "привіт, світе"), ("a😀b", "a😁b")];
        for (old, new) in cases {
            let mut s = old.to_owned();
            if let Some(c) = diff(old, new) {
                apply(&mut s, &[c]);
            }
            assert_eq!(s, new, "{old:?} -> {new:?}");
        }
    }
}
