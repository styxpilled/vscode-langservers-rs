use crate::{ScanError, SyntaxKind};

pub struct Scanner<'a> {
    text: &'a str,
    ignore_trivia: bool,
    pos: usize,
    /// The token value is this range of `text` if set, else `value`.
    value_span: Option<(usize, usize)>,
    value: String,
    token_offset: usize,
    token: SyntaxKind,
    line_number: usize,
    token_start_line: usize,
    token_line_start_offset: usize,
    prev_token_line_start_offset: usize,
    scan_error: ScanError,
}

impl<'a> Scanner<'a> {
    /// If `ignore_trivia` is set, [`Scanner::scan`] skips whitespace, line breaks and comments.
    pub fn new(text: &'a str, ignore_trivia: bool) -> Self {
        Self {
            text,
            ignore_trivia,
            pos: 0,
            value_span: None,
            value: String::new(),
            token_offset: 0,
            token: SyntaxKind::Unknown,
            line_number: 0,
            token_start_line: 0,
            token_line_start_offset: 0,
            prev_token_line_start_offset: 0,
            scan_error: ScanError::None,
        }
    }

    /// A call to [`Scanner::scan`] is necessary to read the first token at the new position.
    pub fn set_position(&mut self, pos: usize) {
        self.pos = pos;
        self.value_span = None;
        self.value.clear();
        self.token_offset = 0;
        self.token = SyntaxKind::Unknown;
        self.scan_error = ScanError::None;
    }

    pub fn scan(&mut self) -> SyntaxKind {
        loop {
            let token = self.scan_next();
            if !self.ignore_trivia || !is_trivia(token) {
                return token;
            }
        }
    }

    /// The position after the last token.
    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn token(&self) -> SyntaxKind {
        self.token
    }

    /// For strings, the decoded content.
    pub fn token_value(&self) -> &str {
        match self.value_span {
            Some((start, end)) => &self.text[start..end],
            None => &self.value,
        }
    }

    fn append(&mut self, start: usize, end: usize) {
        if self.value_span.is_none() && self.value.is_empty() {
            self.value_span = Some((start, end));
        } else {
            self.materialize();
            self.value.push_str(&self.text[start..end]);
        }
    }

    fn push_char(&mut self, c: char) {
        self.materialize();
        self.value.push(c);
    }

    fn materialize(&mut self) {
        if let Some((start, end)) = self.value_span.take() {
            self.value.push_str(&self.text[start..end]);
        }
    }

    pub fn token_offset(&self) -> usize {
        self.token_offset
    }

    pub fn token_length(&self) -> usize {
        self.pos - self.token_offset
    }

    pub fn token_start_line(&self) -> usize {
        self.token_start_line
    }

    pub fn token_start_character(&self) -> usize {
        self.token_offset
            .saturating_sub(self.prev_token_line_start_offset)
    }

    pub fn token_error(&self) -> ScanError {
        self.scan_error
    }

    fn byte(&self, index: usize) -> Option<u8> {
        self.text.as_bytes().get(index).copied()
    }

    fn scan_next(&mut self) -> SyntaxKind {
        self.token = self.read_token();
        self.token
    }

    fn read_token(&mut self) -> SyntaxKind {
        self.value_span = None;
        self.value.clear();
        self.scan_error = ScanError::None;
        self.token_offset = self.pos;
        self.token_start_line = self.line_number;
        self.prev_token_line_start_offset = self.token_line_start_offset;

        let text = self.text;
        let bytes = text.as_bytes();
        let len = text.len();
        if self.pos >= len {
            self.token_offset = len;
            return SyntaxKind::EOF;
        }

        let code = bytes[self.pos];
        if is_white_space(code) {
            while self.byte(self.pos).is_some_and(is_white_space) {
                self.pos += 1;
            }
            self.append(self.token_offset, self.pos);
            return SyntaxKind::Trivia;
        }

        if is_line_break(code) {
            self.pos += 1;
            if code == b'\r' && self.byte(self.pos) == Some(b'\n') {
                self.pos += 1;
            }
            self.append(self.token_offset, self.pos);
            self.line_number += 1;
            self.token_line_start_offset = self.pos;
            return SyntaxKind::LineBreakTrivia;
        }

        match code {
            b'{' => self.single(SyntaxKind::OpenBraceToken),
            b'}' => self.single(SyntaxKind::CloseBraceToken),
            b'[' => self.single(SyntaxKind::OpenBracketToken),
            b']' => self.single(SyntaxKind::CloseBracketToken),
            b':' => self.single(SyntaxKind::ColonToken),
            b',' => self.single(SyntaxKind::CommaToken),
            b'"' => {
                self.pos += 1;
                self.scan_string();
                SyntaxKind::StringLiteral
            }
            b'/' => {
                // Upstream starts the value one character before the slash (a bug).
                let start = self.pos;
                if self.byte(self.pos + 1) == Some(b'/') {
                    self.pos += 2;
                    while self.pos < len && !is_line_break(bytes[self.pos]) {
                        self.pos += 1;
                    }
                    self.append(start, self.pos);
                    return SyntaxKind::LineCommentTrivia;
                }

                if self.byte(self.pos + 1) == Some(b'*') {
                    self.pos += 2;
                    let safe_length = len - 1;
                    let mut closed = false;
                    while self.pos < safe_length {
                        let ch = bytes[self.pos];
                        if ch == b'*' && bytes[self.pos + 1] == b'/' {
                            self.pos += 2;
                            closed = true;
                            break;
                        }
                        self.pos += 1;
                        if is_line_break(ch) {
                            if ch == b'\r' && self.byte(self.pos) == Some(b'\n') {
                                self.pos += 1;
                            }
                            self.line_number += 1;
                            self.token_line_start_offset = self.pos;
                        }
                    }
                    if !closed {
                        // Upstream can move past the end of the text here.
                        self.pos = (self.pos + 1).min(len);
                        self.scan_error = ScanError::UnexpectedEndOfComment;
                    }
                    self.append(start, self.pos);
                    return SyntaxKind::BlockCommentTrivia;
                }

                self.push_char('/');
                self.pos += 1;
                SyntaxKind::Unknown
            }
            b'-' => {
                self.pos += 1;
                if !self.byte(self.pos).is_some_and(|b| b.is_ascii_digit()) {
                    self.push_char('-');
                    return SyntaxKind::Unknown;
                }
                self.scan_number();
                SyntaxKind::NumericLiteral
            }
            b'0'..=b'9' => {
                self.scan_number();
                SyntaxKind::NumericLiteral
            }
            _ => {
                while self
                    .byte(self.pos)
                    .is_some_and(is_unknown_content_character)
                {
                    self.pos += 1;
                }
                if self.token_offset != self.pos {
                    self.append(self.token_offset, self.pos);
                    return match self.token_value() {
                        "true" => SyntaxKind::TrueKeyword,
                        "false" => SyntaxKind::FalseKeyword,
                        "null" => SyntaxKind::NullKeyword,
                        _ => SyntaxKind::Unknown,
                    };
                }
                let ch = text[self.pos..].chars().next().unwrap_or_default();
                self.push_char(ch);
                self.pos += ch.len_utf8();
                SyntaxKind::Unknown
            }
        }
    }

    fn single(&mut self, kind: SyntaxKind) -> SyntaxKind {
        self.pos += 1;
        kind
    }

    fn scan_number(&mut self) {
        let text = self.text;
        let is_digit = |b: Option<u8>| b.is_some_and(|b| b.is_ascii_digit());
        let start = self.pos;
        self.pos += 1;
        if text.as_bytes()[start] != b'0' {
            while is_digit(self.byte(self.pos)) {
                self.pos += 1;
            }
        }
        if self.byte(self.pos) == Some(b'.') {
            self.pos += 1;
            if !is_digit(self.byte(self.pos)) {
                self.scan_error = ScanError::UnexpectedEndOfNumber;
                self.append(self.token_offset, self.pos);
                return;
            }
            while is_digit(self.byte(self.pos)) {
                self.pos += 1;
            }
        }
        let mut end = self.pos;
        if matches!(self.byte(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.byte(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if is_digit(self.byte(self.pos)) {
                while is_digit(self.byte(self.pos)) {
                    self.pos += 1;
                }
                end = self.pos;
            } else {
                self.scan_error = ScanError::UnexpectedEndOfNumber;
            }
        }
        self.append(self.token_offset, end);
    }

    fn scan_string(&mut self) {
        let text = self.text;
        let bytes = text.as_bytes();
        let len = text.len();
        let mut start = self.pos;
        loop {
            if self.pos >= len {
                self.append(start, self.pos);
                self.scan_error = ScanError::UnexpectedEndOfString;
                break;
            }
            let ch = bytes[self.pos];
            if ch == b'"' {
                self.append(start, self.pos);
                self.pos += 1;
                break;
            }
            if ch == b'\\' {
                self.append(start, self.pos);
                self.pos += 1;
                if self.pos >= len {
                    self.scan_error = ScanError::UnexpectedEndOfString;
                    break;
                }
                let escaped = bytes[self.pos];
                self.pos += 1;
                match escaped {
                    b'"' => self.push_char('"'),
                    b'\\' => self.push_char('\\'),
                    b'/' => self.push_char('/'),
                    b'b' => self.push_char('\u{8}'),
                    b'f' => self.push_char('\u{c}'),
                    b'n' => self.push_char('\n'),
                    b'r' => self.push_char('\r'),
                    b't' => self.push_char('\t'),
                    b'u' => match self.scan_unicode_escape() {
                        Some(c) => self.push_char(c),
                        None => self.scan_error = ScanError::InvalidUnicode,
                    },
                    _ => {
                        let escaped_len = text[self.pos - 1..]
                            .chars()
                            .next()
                            .map_or(1, char::len_utf8);
                        self.pos += escaped_len - 1;
                        self.scan_error = ScanError::InvalidEscapeCharacter;
                    }
                }
                start = self.pos;
                continue;
            }
            if ch <= 0x1f {
                if is_line_break(ch) {
                    self.append(start, self.pos);
                    self.scan_error = ScanError::UnexpectedEndOfString;
                    break;
                }
                self.scan_error = ScanError::InvalidCharacter;
            }
            self.pos += 1;
        }
    }

    /// Upstream keeps unpaired surrogates in the string. Rust strings cannot hold them,
    /// so a surrogate pair becomes one character and an unpaired surrogate becomes U+FFFD.
    fn scan_unicode_escape(&mut self) -> Option<char> {
        let high = self.scan_hex_digits()?;
        if !(0xD800..0xDC00).contains(&high) {
            return Some(char::from_u32(high).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        let saved = self.pos;
        if self.text[self.pos..].starts_with("\\u") {
            self.pos += 2;
            if let Some(low @ 0xDC00..0xE000) = self.scan_hex_digits() {
                let c = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                return char::from_u32(c);
            }
        }
        self.pos = saved;
        Some(char::REPLACEMENT_CHARACTER)
    }

    fn scan_hex_digits(&mut self) -> Option<u32> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = self.byte(self.pos).and_then(|b| (b as char).to_digit(16))?;
            value = value * 16 + digit;
            self.pos += 1;
        }
        Some(value)
    }
}

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LineCommentTrivia
            | SyntaxKind::BlockCommentTrivia
            | SyntaxKind::LineBreakTrivia
            | SyntaxKind::Trivia
    )
}

fn is_white_space(ch: u8) -> bool {
    ch == b' ' || ch == b'\t'
}

pub(crate) fn is_line_break(ch: u8) -> bool {
    ch == b'\n' || ch == b'\r'
}

fn is_unknown_content_character(ch: u8) -> bool {
    !is_white_space(ch)
        && !is_line_break(ch)
        && !matches!(ch, b'}' | b']' | b'{' | b'[' | b'"' | b':' | b',' | b'/')
}
