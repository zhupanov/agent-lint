//! Path-aware JSON source locator shared by validator domains.
//!
//! Validators that already hold a parsed `serde_json::Value` use this scanner
//! to recover the byte range of the value (or key token) at a structural path
//! so diagnostics can carry exact source spans. It is a locator, not a parser:
//! parse ownership stays with the domain that loaded the document.

use std::ops::Range;

/// One access-path segment used to locate a JSON value's source span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seg<'a> {
    Key(&'a str),
    Index(usize),
}

/// Best-effort source-span locator for an already-parsed JSON document.
///
/// The document is known to parse (serde accepted it), so scanning stays lenient
/// and returns `None` rather than failing when a shape is unexpected; a missing
/// location simply omits the optional metadata. On a duplicate key the first
/// occurrence is located.
pub(crate) struct JsonScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonScanner<'a> {
    pub(crate) fn locate(source: &'a str, path: &[Seg]) -> Option<Range<usize>> {
        let mut scanner = Self {
            bytes: source.as_bytes(),
            pos: 0,
        };
        scanner.skip_ws();
        scanner.value_range(path)
    }

    /// Locate the key token (including quotes) of the final `Seg::Key` segment
    /// instead of its value. Returns `None` when the path is empty, ends in an
    /// index, or the key is absent.
    pub(crate) fn locate_key(source: &'a str, path: &[Seg]) -> Option<Range<usize>> {
        let (last, parents) = path.split_last()?;
        let Seg::Key(wanted) = *last else {
            return None;
        };
        let parent = Self::locate(source, parents)?;
        let mut scanner = Self {
            bytes: source.as_bytes(),
            pos: parent.start,
        };
        scanner.key_range(wanted)
    }

    fn key_range(&mut self, wanted: &str) -> Option<Range<usize>> {
        self.skip_ws();
        self.take(b'{')?;
        loop {
            self.skip_ws();
            if self.take(b'}').is_some() {
                return None;
            }
            let start = self.pos;
            let key = self.parse_string()?;
            let end = self.pos;
            self.skip_ws();
            self.take(b':')?;
            self.skip_ws();
            if key == wanted {
                return Some(start..end);
            }
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
        }
    }

    fn value_range(&mut self, path: &[Seg]) -> Option<Range<usize>> {
        let Some((first, rest)) = path.split_first() else {
            let start = self.pos;
            self.skip_value()?;
            return Some(start..self.pos);
        };
        match *first {
            Seg::Key(key) => self.descend_object(key, rest),
            Seg::Index(index) => self.descend_array(index, rest),
        }
    }

    fn descend_object(&mut self, wanted: &str, rest: &[Seg]) -> Option<Range<usize>> {
        if self.take(b'{').is_none() {
            self.skip_value();
            return None;
        }
        loop {
            self.skip_ws();
            if self.take(b'}').is_some() {
                return None;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.take(b':')?;
            self.skip_ws();
            if key == wanted {
                return self.value_range(rest);
            }
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
        }
    }

    fn descend_array(&mut self, wanted: usize, rest: &[Seg]) -> Option<Range<usize>> {
        if self.take(b'[').is_none() {
            self.skip_value();
            return None;
        }
        let mut index = 0;
        loop {
            self.skip_ws();
            if self.take(b']').is_some() {
                return None;
            }
            if index == wanted {
                return self.value_range(rest);
            }
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
            index += 1;
        }
    }

    fn skip_value(&mut self) -> Option<()> {
        self.skip_ws();
        match self.bytes.get(self.pos)? {
            b'"' => self.skip_string(),
            b'{' => self.skip_object(),
            b'[' => self.skip_array(),
            _ => self.skip_scalar(),
        }
    }

    fn skip_object(&mut self) -> Option<()> {
        self.take(b'{')?;
        loop {
            self.skip_ws();
            if self.take(b'}').is_some() {
                return Some(());
            }
            self.skip_string()?;
            self.skip_ws();
            self.take(b':')?;
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
        }
    }

    fn skip_array(&mut self) -> Option<()> {
        self.take(b'[')?;
        loop {
            self.skip_ws();
            if self.take(b']').is_some() {
                return Some(());
            }
            self.skip_value()?;
            self.skip_ws();
            self.take(b',');
        }
    }

    fn skip_scalar(&mut self) -> Option<()> {
        while let Some(byte) = self.bytes.get(self.pos) {
            if matches!(byte, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                break;
            }
            self.pos += 1;
        }
        Some(())
    }

    fn skip_string(&mut self) -> Option<()> {
        self.take(b'"')?;
        loop {
            let byte = *self.bytes.get(self.pos)?;
            self.pos += 1;
            match byte {
                b'"' => return Some(()),
                b'\\' => self.pos += 1,
                _ => {}
            }
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        self.take(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let byte = *self.bytes.get(self.pos)?;
            self.pos += 1;
            match byte {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let escape = *self.bytes.get(self.pos)?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = self.bytes.get(self.pos..self.pos + 4)?;
                            self.pos += 4;
                            let code =
                                u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            let mut buffer = [0u8; 4];
                            let encoded = char::from_u32(code)
                                .unwrap_or('\u{fffd}')
                                .encode_utf8(&mut buffer);
                            out.extend_from_slice(encoded.as_bytes());
                        }
                        _ => return None,
                    }
                }
                _ => out.push(byte),
            }
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn take(&mut self, byte: u8) -> Option<()> {
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_scanner_locates_nested_paths() {
        let source = r#"{"name":"x","interface":{"defaultPrompt":["a","bb"]}}"#;
        // Locations point at the offending value, not the key/member.
        let name = JsonScanner::locate(source, &[Seg::Key("name")]).unwrap();
        assert_eq!(&source[name], "\"x\"");

        let prompt = JsonScanner::locate(
            source,
            &[
                Seg::Key("interface"),
                Seg::Key("defaultPrompt"),
                Seg::Index(1),
            ],
        )
        .unwrap();
        assert_eq!(&source[prompt], "\"bb\"");

        assert!(JsonScanner::locate(source, &[Seg::Key("missing")]).is_none());
    }

    #[test]
    fn json_scanner_locates_repeated_equal_values_by_path() {
        // The same serialized value appears in two fields; each path resolves
        // to its own occurrence, not the first document-wide match.
        let source = "{\n  \"commands\": \"../same\",\n  \"skills\": \"../same\"\n}";
        let commands = JsonScanner::locate(source, &[Seg::Key("commands")]).unwrap();
        let skills = JsonScanner::locate(source, &[Seg::Key("skills")]).unwrap();
        assert_eq!(&source[commands.clone()], "\"../same\"");
        assert_eq!(&source[skills.clone()], "\"../same\"");
        assert!(commands.end <= skills.start);
    }

    #[test]
    fn json_scanner_locates_key_tokens() {
        let source = r#"{"hooks":{"beforeShellExecution":[{"command":""}],"unknown":[]}}"#;
        let event = JsonScanner::locate_key(
            source,
            &[Seg::Key("hooks"), Seg::Key("unknown")],
        )
        .unwrap();
        assert_eq!(&source[event], "\"unknown\"");

        let top = JsonScanner::locate_key(source, &[Seg::Key("hooks")]).unwrap();
        assert_eq!(&source[top], "\"hooks\"");

        // Escaped key spellings decode before comparison; the located token is
        // the raw escaped spelling from the source.
        let escaped = "{\"\\u0068ooks\": 1}";
        let decoded = JsonScanner::locate_key(escaped, &[Seg::Key("hooks")]).unwrap();
        assert_eq!(&escaped[decoded], "\"\\u0068ooks\"");

        assert!(JsonScanner::locate_key(source, &[]).is_none());
        assert!(JsonScanner::locate_key(source, &[Seg::Index(0)]).is_none());
        assert!(JsonScanner::locate_key(source, &[Seg::Key("missing")]).is_none());
    }
}
