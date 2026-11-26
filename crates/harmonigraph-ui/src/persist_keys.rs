//! The editor door's per-key decode (#1757 Q4): a saved value naming an enum
//! variant this build does not have costs the key that holds it, rather than
//! the whole document.
//!
//! serde cannot build a partial struct around a variant it has no arm for, so
//! this works on the text: parse, and on an unknown variant find the
//! `key: value` pair whose value holds it, cut the pair, and parse again —
//! every persisted struct carries a container `#[serde(default)]`, so a key
//! cut is a key at its default. Two routes without a scanner lose: `ron::Value`
//! forgets variant names, and splitting only the top-level fields would still
//! cost the whole appearance for one variant inside it.
//!
//! A variant that is a bare element of a list or map has no key of its own,
//! and still refuses the document whole; none is persisted today. The take and
//! export door (`AppearanceDocument::parse`) keeps the strict parse (Q5), and
//! also reports the keys it did not read (#1932) and the repairs it made
//! (#1960).

use std::borrow::Cow;
use std::ops::Range;

/// A pair the decode cut: its key's path from the document's top, dotted
/// (`appearance.spectrum.window`), and the variant this build lacks.
pub(crate) struct Dropped {
    pub key: String,
    pub variant: String,
}

/// A document the decode refused whole: the first error it met, with its
/// position in the original text, the pairs it had already cut, and what
/// sank the text those cuts left, whose position is in that text alone.
pub(crate) struct Refusal {
    first: Box<ron::error::SpannedError>,
    cut: Vec<Dropped>,
    last: Option<ron::Error>,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.first)?;
        if !self.cut.is_empty() {
            let keys: Vec<&str> = self.cut.iter().map(|d| d.key.as_str()).collect();
            write!(f, "; cutting {} did not leave a document", keys.join(", "))?;
            if let Some(last) = &self.last {
                write!(f, " ({last})")?;
            }
        }
        Ok(())
    }
}

/// What [`decode`] made of a document: the value, the pairs it cut, and the
/// path of each key the value has no field for, dotted like a cut's.
pub(crate) struct Decoded<T> {
    pub value: T,
    pub dropped: Vec<Dropped>,
    pub unread: Vec<String>,
}

/// `ron::from_str`, also naming each key serde skipped because `T` has no
/// field for it (#1927 B): a key a rename retired, whose setting opens at its
/// default.
pub(crate) fn from_str_noting<T: serde::de::DeserializeOwned>(
    text: &str,
) -> Result<(T, Vec<String>), ron::error::SpannedError> {
    let mut parser = ron::Deserializer::from_str(text)?;
    let mut unread = Vec::new();
    let value = serde_ignored::deserialize(&mut parser, |path| unread.push(dotted(&path)))
        .and_then(|value| parser.end().map(|()| value))
        .map_err(|err| parser.span_error(err))?;
    Ok((value, unread))
}

/// A skipped key's path as the document spells it: keys and list indices
/// joined by dots, with nothing for the `Some(…)` or newtype it passed
/// through, which `serde_ignored`'s own `Display` writes as `?`.
fn dotted(path: &serde_ignored::Path) -> String {
    use serde_ignored::Path;
    let (parent, segment) = match path {
        Path::Root => return String::new(),
        Path::Seq { parent, index } => (parent, index.to_string()),
        Path::Map { parent, key } => (parent, key.clone()),
        Path::Some { parent }
        | Path::NewtypeStruct { parent }
        | Path::NewtypeVariant { parent } => {
            return dotted(parent);
        }
    };
    match dotted(parent) {
        above if above.is_empty() => segment,
        above => format!("{above}.{segment}"),
    }
}

/// Parse `text`, cutting each pair whose value names an unknown variant.
/// Any other failure, or an unknown variant no key holds, refuses it whole.
pub(crate) fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<Decoded<T>, Refusal> {
    let mut text = Cow::Borrowed(text);
    let mut cut = Vec::new();
    let mut first = None;
    loop {
        let err = match from_str_noting::<T>(&text) {
            Ok((value, unread)) => return Ok(Decoded { value, dropped: cut, unread }),
            Err(err) => err,
        };
        let pair = match &err.code {
            ron::Error::NoSuchEnumVariant { found, .. } => byte_offset(&text, err.span.start)
                .filter(|&at| !found.is_empty() && text[at..].starts_with(found.as_str()))
                .and_then(|at| pair_holding(&text, at))
                .map(|pair| (pair, found.clone())),
            _ => None,
        };
        let last = (!cut.is_empty()).then(|| err.code.clone());
        let first = first.get_or_insert(err);
        let Some(((key, span), variant)) = pair else {
            return Err(Refusal { first: Box::new(first.clone()), cut, last });
        };
        let next = format!("{}{}", &text[..span.start], &text[span.end..]);
        cut.push(Dropped { key, variant });
        // Each cut is a non-empty pair, so this only guards the loop's end.
        if next.len() >= text.len() {
            return Err(Refusal { first: Box::new(first.clone()), cut, last: None });
        }
        text = Cow::Owned(next);
    }
}

/// Where a position ron reports — a column counted in characters, on the one
/// line `ron::to_string` writes — falls in `text`, in bytes.
fn byte_offset(text: &str, at: ron::error::Position) -> Option<usize> {
    if at.line > 1 {
        return None;
    }
    let (offset, _) = text.char_indices().nth(at.col.checked_sub(1)?)?;
    Some(offset)
}

/// The pair whose value holds the token at byte `at`: its key's path, and the span
/// to cut — the key through the value and the comma after it. The value
/// may sit inside unnamed parentheses, `Some(…)` or a tuple, which the walk
/// out from `at` passes through; a list or map between it and its key ends
/// the walk with nothing.
fn pair_holding(text: &str, at: usize) -> Option<(String, Range<usize>)> {
    // Each open bracket, with the key its current value belongs to.
    let mut frames: Vec<(u8, Option<Range<usize>>)> = Vec::new();
    let mut tokens = Tokens { text: text.as_bytes(), next: 0 };
    let mut previous: Option<(Token, Range<usize>)> = None;
    loop {
        let (token, range) = tokens.next()?;
        if range.start >= at {
            if range.start != at {
                return None;
            }
            break;
        }
        match token {
            Token::Open(bracket) => frames.push((bracket, None)),
            Token::Close => {
                frames.pop();
            }
            Token::Comma => {
                if let Some(frame) = frames.last_mut() {
                    frame.1 = None;
                }
            }
            Token::Colon => {
                if let (Some((Token::Ident, key)), Some(frame)) = (&previous, frames.last_mut()) {
                    frame.1 = Some(key.clone());
                }
            }
            Token::Ident | Token::Other => {}
        }
        previous = Some((token, range));
    }
    let depth = frames.iter().rposition(|(bracket, key)| *bracket != b'(' || key.is_some())?;
    let (b'(', Some(key)) = frames[depth].clone() else { return None };
    let path: Vec<&str> =
        frames[..=depth].iter().filter_map(|(_, key)| key.clone()).map(|key| &text[key]).collect();
    let path = path.join(".");
    // The value ends at a comma back at its pair's depth, or where the
    // struct around the pair closes.
    let mut open = frames.len() - depth - 1;
    let mut tokens = Tokens { text: text.as_bytes(), next: at };
    loop {
        let (token, range) = tokens.next()?;
        match token {
            Token::Open(_) => open += 1,
            Token::Close if open == 0 => return Some((path, key.start..range.start)),
            Token::Close => open -= 1,
            Token::Comma if open == 0 => return Some((path, key.start..range.end)),
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token {
    Open(u8),
    Close,
    Comma,
    Colon,
    Ident,
    /// A number, string or any other byte that is none of the above.
    Other,
}

/// RON's tokens as far as brackets, pairs and keys go, in the text
/// `ron::to_string` writes for the editor's state: compact, with every string
/// quoted and escaped, no comments, raw strings or characters, and an
/// identifier for every key and variant. Strings are skipped whole, so nothing
/// quoted can open a bracket or name a key. Other RON, hand-edited, may name
/// the wrong key or be refused, each with its Console line.
struct Tokens<'a> {
    text: &'a [u8],
    next: usize,
}

impl Tokens<'_> {
    fn at(&self, i: usize) -> u8 {
        self.text.get(i).copied().unwrap_or(0)
    }

    /// Past a quoted string whose opening quote is at `self.next`.
    fn skip_string(&mut self) {
        self.next += 1;
        loop {
            match self.at(self.next) {
                0 => return,
                b'\\' => self.next += 2,
                b'"' => {
                    self.next += 1;
                    return;
                }
                _ => self.next += 1,
            }
        }
    }
}

impl Iterator for Tokens<'_> {
    type Item = (Token, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.next;
        let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let token = match self.at(start) {
            0 => return None,
            b'(' | b'[' | b'{' => {
                self.next += 1;
                Token::Open(self.at(start))
            }
            b')' | b']' | b'}' => {
                self.next += 1;
                Token::Close
            }
            b',' => {
                self.next += 1;
                Token::Comma
            }
            b':' => {
                self.next += 1;
                Token::Colon
            }
            b'"' => {
                self.skip_string();
                Token::Other
            }
            b if ident(b) => {
                while ident(self.at(self.next)) {
                    self.next += 1;
                }
                if b.is_ascii_digit() {
                    Token::Other
                } else {
                    Token::Ident
                }
            }
            _ => {
                self.next += 1;
                Token::Other
            }
        };
        Some((token, start..self.next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, serde::Deserialize)]
    #[serde(default)]
    struct Outer {
        name: String,
        mode: Mode,
        inner: Inner,
        maybe: Option<Mode>,
        list: Vec<Mode>,
        count: u32,
    }

    #[derive(Debug, Default, PartialEq, serde::Deserialize)]
    #[serde(default)]
    struct Inner {
        mode: Mode,
        count: u32,
    }

    #[derive(Debug, Default, PartialEq, serde::Deserialize)]
    enum Mode {
        #[default]
        A,
        B,
    }

    fn cut(text: &str) -> (Outer, Vec<String>) {
        let decoded = decode::<Outer>(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        let dropped = decoded.dropped.into_iter().map(|d| format!("{}:{}", d.key, d.variant));
        (decoded.value, dropped.collect())
    }

    /// A variant this build lacks costs the key that holds it, at any depth,
    /// named by its path, and the pairs around it load.
    #[test]
    fn an_unknown_variant_costs_its_own_key_at_any_depth() {
        let (outer, dropped) = cut("(mode:Gone,inner:(mode:Lost,count:3),count:7)");
        assert_eq!(dropped, ["mode:Gone", "inner.mode:Lost"]);
        assert_eq!(outer.mode, Mode::A);
        assert_eq!(outer.inner, Inner { mode: Mode::A, count: 3 });
        assert_eq!(outer.count, 7);
        let (outer, _) = cut("(count:7,inner:(count:3,mode:Lost))");
        assert_eq!(outer.inner.count, 3, "the last pair of a struct");
    }

    /// A key this build has no field for is named by its path through
    /// structs, `Some(…)` and list indices alike, and costs nothing else.
    #[test]
    fn a_key_with_no_field_is_named_by_its_path() {
        let decoded = decode::<Outer>("(gone:1,inner:(count:3,old:B),slots:[(x:1)],mode:B)")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(decoded.unread, ["gone", "inner.old", "slots"]);
        assert_eq!((decoded.value.inner.count, decoded.value.mode), (3, Mode::B));
        let decoded = decode::<Wrapped>("(held:Some((count:2,old:1)),list:[(count:1),(stale:0)])")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(decoded.unread, ["held.old", "list.1.stale"]);
    }

    #[derive(Debug, Default, serde::Deserialize)]
    #[serde(default)]
    struct Wrapped {
        held: Option<Inner>,
        list: Vec<Inner>,
    }

    /// One wrapped in `Some(…)` costs the key the option sits under.
    #[test]
    fn an_unknown_variant_inside_some_costs_the_key_that_holds_it() {
        let (outer, dropped) = cut("(maybe:Some(Gone),mode:B)");
        assert_eq!(dropped, ["maybe:Gone"]);
        assert_eq!((outer.maybe, outer.mode), (None, Mode::B));
    }

    /// A string opens no bracket and names no key, an escaped quote ends no
    /// string, and ron's columns count characters, so text wider than ASCII
    /// ahead of the variant moves no byte offset.
    #[test]
    fn a_string_does_not_mislead_the_cut() {
        // Read as tokens, the name would open two structs and key the
        // variant's pair under them. The second escape sits far enough in
        // that a cursor doubled there, not stepped past it, overshoots.
        let (outer, dropped) =
            cut("(name:\"é \\\" (inner:( \\\" x\",list:[A,B],mode:Gone,count:7)");
        assert_eq!(dropped, ["mode:Gone"]);
        assert_eq!(outer.name, "é \" (inner:( \" x");
        assert_eq!(outer.list, [Mode::A, Mode::B]);
        assert_eq!((outer.mode, outer.count), (Mode::A, 7));
    }

    /// A bare element of a list has no key of its own: the document is
    /// refused whole, as before, and the refusal reports the first variant
    /// met and the keys cut before it gave up, not the cut text's error.
    #[test]
    fn an_unknown_variant_in_a_list_still_refuses_the_document() {
        assert!(decode::<Outer>("(list:[A,Gone],mode:B)").is_err());
        let Err(refusal) = decode::<Outer>("(mode:Lost,list:[A,Gone])") else {
            panic!("a list's unknown variant was cut");
        };
        let said = refusal.to_string();
        assert!(said.contains("Lost") && said.contains("cutting mode"), "{said}");
        assert!(said.contains("Gone"), "what sank the cut text goes unnamed: {said}");
    }
}
