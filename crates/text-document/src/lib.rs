//! Port of `vscode-languageserver-textdocument`.

use lsp_types::{Position, Range, TextDocumentContentChangeEvent, TextEdit, Uri};

/// Offsets are UTF-8 byte offsets. Positions use
/// UTF-16 characters, as LSP does.
#[derive(Clone, Debug)]
pub struct TextDocument {
    uri: Uri,
    language_id: String,
    version: i32,
    text: String,
    line_offsets: Vec<usize>,
    wide_chars: Vec<WideChar>,
}

/// A character with a different length in UTF-8 and UTF-16. These make the conversion between
/// byte offsets and UTF-16 offsets a binary search.
#[derive(Clone, Copy, Debug)]
struct WideChar {
    offset: usize,
    utf16_offset: usize,
    len_utf16: usize,
    /// UTF-8 length minus UTF-16 length of this and all earlier wide characters.
    excess: usize,
}

impl TextDocument {
    pub fn new(uri: Uri, language_id: impl Into<String>, version: i32, text: String) -> Self {
        let (line_offsets, wide_chars) = index(&text);
        Self {
            uri,
            language_id: language_id.into(),
            version,
            text,
            line_offsets,
            wide_chars,
        }
    }

    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    pub fn language_id(&self) -> &str {
        &self.language_id
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn text_in(&self, range: Range) -> &str {
        let start = self.offset_at(range.start);
        &self.text[start..self.offset_at(range.end).max(start)]
    }

    pub fn line_count(&self) -> usize {
        self.line_offsets.len()
    }

    /// `offset` must be at a character boundary.
    fn utf16_offset(&self, offset: usize) -> usize {
        let i = self.wide_chars.partition_point(|w| w.offset < offset);
        offset - i.checked_sub(1).map_or(0, |i| self.wide_chars[i].excess)
    }

    /// An offset in a surrogate pair gives the start of the character.
    fn byte_offset(&self, utf16_offset: usize) -> usize {
        let i = self
            .wide_chars
            .partition_point(|w| w.utf16_offset < utf16_offset);
        let Some(w) = i.checked_sub(1).map(|i| self.wide_chars[i]) else {
            return utf16_offset;
        };
        if w.utf16_offset + w.len_utf16 > utf16_offset {
            w.offset
        } else {
            utf16_offset + w.excess
        }
    }

    pub fn position_at(&self, offset: usize) -> Position {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.line_offsets.partition_point(|&o| o <= offset) - 1;
        let line_offset = self.line_offsets[line];
        let offset = self.ensure_before_eol(offset, line_offset);
        let character = self.utf16_offset(offset) - self.utf16_offset(line_offset);
        Position::new(line as u32, character as u32)
    }

    pub fn offset_at(&self, position: Position) -> usize {
        let line = position.line as usize;
        let Some(&line_offset) = self.line_offsets.get(line) else {
            return self.text.len();
        };
        let next_line_offset = self
            .line_offsets
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        let target = self.utf16_offset(line_offset) + position.character as usize;
        let offset = self.byte_offset(target).min(next_line_offset);
        self.ensure_before_eol(offset, line_offset)
    }

    pub fn range(&self, start: usize, end: usize) -> Range {
        Range::new(self.position_at(start), self.position_at(end))
    }

    pub fn full_range(&self) -> Range {
        self.range(0, self.text.len())
    }

    /// `TextDocument.update`: applies the content changes in order.
    pub fn update(self, changes: Vec<TextDocumentContentChangeEvent>, version: i32) -> Self {
        let mut document = self;
        for change in changes {
            let text = match change.range {
                None => change.text,
                Some(range) => {
                    let a = document.offset_at(range.start);
                    let b = document.offset_at(range.end);
                    let (start, end) = (a.min(b), a.max(b));
                    let old = &document.text;
                    let mut text =
                        String::with_capacity(old.len() - (end - start) + change.text.len());
                    text.push_str(&old[..start]);
                    text.push_str(&change.text);
                    text.push_str(&old[end..]);
                    text
                }
            };
            document = TextDocument::new(document.uri, document.language_id, version, text);
        }
        document.version = version;
        document
    }

    fn ensure_before_eol(&self, mut offset: usize, line_offset: usize) -> usize {
        let bytes = self.text.as_bytes();
        while offset > line_offset && matches!(bytes[offset - 1], b'\r' | b'\n') {
            offset -= 1;
        }
        offset
    }

    /// Applies edits that refer to this document. Overlapping edits are ignored.
    pub fn apply_edits(&self, edits: &[TextEdit]) -> String {
        let mut sorted: Vec<(usize, usize, &str)> = edits
            .iter()
            .map(|e| {
                let start = self.offset_at(e.range.start);
                let end = self.offset_at(e.range.end).max(start);
                (start, end, e.new_text.as_str())
            })
            .collect();
        sorted.sort_by_key(|&(start, _, _)| start);
        let mut result = String::with_capacity(self.text.len());
        let mut last_modified_offset = 0;
        for (start, end, new_text) in sorted {
            if start < last_modified_offset {
                continue;
            }
            result.push_str(&self.text[last_modified_offset..start]);
            result.push_str(new_text);
            last_modified_offset = end;
        }
        result.push_str(&self.text[last_modified_offset..]);
        result
    }
}

fn index(text: &str) -> (Vec<usize>, Vec<WideChar>) {
    let bytes = text.as_bytes();
    let mut line_offsets = vec![0];
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                i += 1;
                line_offsets.push(i + 1);
            }
            b'\r' | b'\n' => line_offsets.push(i + 1),
            _ => {}
        }
        i += 1;
    }

    let mut wide_chars = Vec::new();
    if !text.is_ascii() {
        let mut excess = 0;
        for (offset, c) in text.char_indices().filter(|(_, c)| !c.is_ascii()) {
            let utf16_offset = offset - excess;
            excess += c.len_utf8() - c.len_utf16();
            wide_chars.push(WideChar {
                offset,
                utf16_offset,
                len_utf16: c.len_utf16(),
                excess,
            });
        }
    }
    (line_offsets, wide_chars)
}

#[cfg(test)]
mod tests {
    use super::TextDocument;
    use lsp_types::Position;

    #[test]
    fn utf16_positions() {
        let text = "aé😀b\r\nc€\nd";
        let doc = TextDocument::new("test://a".parse().unwrap(), "json", 0, text.to_owned());
        let cases = [
            (0, (0, 0)),
            (1, (0, 1)),
            (3, (0, 2)),
            (7, (0, 4)),
            (8, (0, 5)),
            // Between `\r` and `\n`.
            (9, (0, 5)),
            (10, (1, 0)),
            (11, (1, 1)),
            (14, (1, 2)),
            (15, (2, 0)),
            (16, (2, 1)),
        ];
        for (offset, (line, character)) in cases {
            assert_eq!(
                doc.position_at(offset),
                Position::new(line, character),
                "{offset}"
            );
        }
        assert_eq!(
            doc.offset_at(Position::new(0, 3)),
            3,
            "inside the surrogate pair"
        );
        assert_eq!(doc.offset_at(Position::new(0, 4)), 7);
        assert_eq!(doc.offset_at(Position::new(0, 99)), 8);
        assert_eq!(doc.offset_at(Position::new(1, 2)), 14);
        assert_eq!(doc.offset_at(Position::new(9, 0)), text.len());
        for (offset, _) in text.char_indices() {
            let back = doc.offset_at(doc.position_at(offset));
            assert!(back == offset || text[back..offset].chars().all(|c| c == '\r'));
        }
    }
}
