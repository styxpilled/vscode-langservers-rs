//! Port of `cssScanner.ts`, `scssScanner.ts` and `lessScanner.ts`.

use std::borrow::Cow;

use super::Dialect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenType {
    Ident,
    AtKeyword,
    String,
    BadString,
    UnquotedString,
    Hash,
    Num,
    Percentage,
    Dimension,
    UnicodeRange,
    /// `<!--`
    Cdo,
    /// `-->`
    Cdc,
    Colon,
    SemiColon,
    CurlyL,
    CurlyR,
    ParenthesisL,
    ParenthesisR,
    BracketL,
    BracketR,
    Whitespace,
    Includes,
    Dashmatch,
    SubstringOperator,
    PrefixOperator,
    SuffixOperator,
    Delim,
    Ems,
    Exs,
    Length,
    Angle,
    Time,
    Freq,
    Exclamation,
    Resolution,
    Comma,
    Charset,
    EscapedJavaScript,
    BadEscapedJavaScript,
    Comment,
    SingleLineComment,
    Eof,
    ContainerQueryLength,
    // SCSS
    VariableName,
    InterpolationFunction,
    Default,
    EqualsOperator,
    NotEqualsOperator,
    GreaterEqualsOperator,
    SmallerEqualsOperator,
    Module,
    // SCSS and Less
    Ellipsis,
}

/// The text is read with `Scanner::text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenType,
    pub offset: usize,
    pub len: usize,
    /// The text differs from the source: an escape was read.
    escaped: bool,
}

impl Token {
    pub fn end(&self) -> usize {
        self.offset + self.len
    }
}

/// `unit.toLowerCase()` is the key of the upstream unit table.
fn dimension_type(unit: &str) -> TokenType {
    let mut ascii = [0; 8];
    let kind = match ascii.get_mut(..unit.len()) {
        Some(lower) if unit.is_ascii() => {
            lower.copy_from_slice(unit.as_bytes());
            lower.make_ascii_lowercase();
            str::from_utf8(lower).ok().and_then(unit_type)
        }
        _ => unit_type(&unit.to_lowercase()),
    };
    // Upstream looks the unit up in a plain JS object, so `constructor` and
    // `__proto__` give a non-token type there.
    kind.unwrap_or(TokenType::Dimension)
}

fn unit_type(unit: &str) -> Option<TokenType> {
    use TokenType::*;
    Some(match unit {
        "em" => Ems,
        "ex" => Exs,
        "px" | "cm" | "mm" | "in" | "pt" | "pc" => Length,
        "deg" | "rad" | "grad" => Angle,
        "ms" | "s" => Time,
        "hz" | "khz" => Freq,
        "%" | "fr" => Percentage,
        "dpi" | "dpcm" => Resolution,
        "cqw" | "cqh" | "cqi" | "cqb" | "cqmin" | "cqmax" => ContainerQueryLength,
        _ => return None,
    })
}

fn static_token(ch: u8) -> Option<TokenType> {
    use TokenType::*;
    Some(match ch {
        b';' => SemiColon,
        b':' => Colon,
        b'{' => CurlyL,
        b'}' => CurlyR,
        b']' => BracketR,
        b'[' => BracketL,
        b'(' => ParenthesisL,
        b')' => ParenthesisR,
        b',' => Comma,
        _ => return None,
    })
}

fn is_hex(ch: u8) -> bool {
    ch.is_ascii_hexdigit()
}

/// Upstream reads UTF-16 code units; `0x80..=0xFFFF` matches each unit of every
/// non-ASCII character. Here the position always stays on a character
/// boundary, and a non-ASCII character is read whole.
pub struct Scanner<'a> {
    src: &'a str,
    /// Can be one past the end: upstream reads a `\` at the end of the text as
    /// an escape of a NUL character.
    pos: usize,
    pub ignore_comment: bool,
    pub ignore_whitespace: bool,
    pub in_url: bool,
    dialect: Dialect,
    /// Set when an escape is read.
    escaped: bool,
    /// The token text. Only `text` collects it, for tokens with escapes.
    buf: Option<String>,
}

impl<'a> Scanner<'a> {
    pub fn new(dialect: Dialect) -> Self {
        Scanner {
            src: "",
            pos: 0,
            ignore_comment: true,
            ignore_whitespace: true,
            in_url: false,
            dialect,
            escaped: false,
            buf: None,
        }
    }

    pub fn set_source(&mut self, input: &'a str) {
        self.src = input;
        self.pos = 0;
    }

    /// Ends the source at `end`: later scans give `Eof` there.
    pub fn end_at(&mut self, end: usize) {
        self.src = &self.src[..end];
        self.pos = self.pos.min(end);
    }

    pub fn source(&self) -> &'a str {
        self.src
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn go_back_to(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Empty if a bound is inside a character. Callers compare the result with
    /// ASCII text, which cannot match there.
    pub fn substring(&self, offset: usize, len: usize) -> &'a str {
        let n = self.src.len();
        self.src
            .get(offset.min(n)..(offset + len).min(n))
            .unwrap_or("")
    }

    fn slice(&self, from: usize, to: usize) -> &'a str {
        let n = self.src.len();
        &self.src[from.min(n)..to.min(n)]
    }

    fn eos(&self) -> bool {
        self.src.len() <= self.pos
    }

    fn peek(&self, n: usize) -> u8 {
        self.src.as_bytes().get(self.pos + n).copied().unwrap_or(0)
    }

    fn char_len(&self) -> usize {
        self.src[self.pos..]
            .chars()
            .next()
            .map_or(1, char::len_utf8)
    }

    fn advance_if(&mut self, ch: u8) -> bool {
        if self.src.len() > self.pos && self.peek(0) == ch {
            self.pos += 1;
            return true;
        }
        false
    }

    fn advance_if_chars(&mut self, chars: &[u8]) -> bool {
        if self
            .src
            .as_bytes()
            .get(self.pos..)
            .is_some_and(|s| s.starts_with(chars))
        {
            self.pos += chars.len();
            return true;
        }
        false
    }

    fn advance_while(&mut self, mut condition: impl FnMut(u8) -> bool) -> usize {
        let start = self.pos;
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() && condition(bytes[self.pos]) {
            self.pos += 1;
        }
        self.pos - start
    }

    /// Moves over one character and appends it to the token text.
    fn take_char(&mut self) {
        let len = if self.peek(0) < 0x80 {
            1
        } else {
            self.char_len()
        };
        if let Some(buf) = &mut self.buf {
            buf.push_str(&self.src[self.pos..self.pos + len]);
        }
        self.pos += len;
    }

    fn push(&mut self, ch: char) {
        if let Some(buf) = &mut self.buf {
            buf.push(ch);
        }
    }

    /// Starts the text of a token.
    fn begin_text(&mut self) {
        self.escaped = false;
        if let Some(buf) = &mut self.buf {
            buf.clear();
        }
    }

    /// Upstream `finishToken`: the token from `offset` to the position.
    fn token(&self, offset: usize, kind: TokenType) -> Token {
        Token {
            kind,
            offset,
            len: self.pos - offset,
            escaped: self.escaped,
        }
    }

    /// The type of a token whose text is the source text.
    fn plain(&mut self, kind: TokenType) -> TokenType {
        self.escaped = false;
        kind
    }

    /// The token text: the source text without the `\` of escapes. An empty
    /// text falls back to the source text, as `text || substring` does
    /// upstream.
    pub fn text(&self, token: &Token) -> Cow<'a, str> {
        let source = self.slice(token.offset, token.end());
        if !token.escaped {
            return Cow::Borrowed(source);
        }
        // Scan the token again and collect the text.
        let mut scanner = Scanner {
            pos: token.offset,
            buf: Some(String::new()),
            ..*self
        };
        if token.kind == TokenType::UnquotedString {
            scanner.unquoted_string();
        } else {
            scanner.scan_next(token.offset);
        }
        match scanner.buf {
            Some(text) if !text.is_empty() => Cow::Owned(text),
            _ => Cow::Borrowed(source),
        }
    }

    pub fn scan_unquoted_string(&mut self) -> Option<Token> {
        let offset = self.pos;
        self.begin_text();
        if self.unquoted_string() {
            return Some(self.token(offset, TokenType::UnquotedString));
        }
        None
    }

    #[inline]
    pub fn scan(&mut self) -> Token {
        let (offset, kind) = self.scan_type();
        self.token(offset, kind)
    }

    /// The start and the type of the next token; the position is its end.
    fn scan_type(&mut self) -> (usize, TokenType) {
        if let Some(trivia) = self.trivia() {
            return trivia;
        }
        let offset = self.pos;
        if self.eos() {
            return (offset, self.plain(TokenType::Eof));
        }
        (offset, self.scan_next(offset))
    }

    /// Reads a unicode range. The `u` is already consumed.
    pub fn try_scan_unicode(&mut self) -> Option<Token> {
        let offset = self.pos;
        if !self.eos() && self.unicode_range() {
            let kind = self.plain(TokenType::UnicodeRange);
            return Some(self.token(offset, kind));
        }
        self.pos = offset;
        None
    }

    fn scan_next(&mut self, offset: usize) -> TokenType {
        let kind = match self.dialect {
            Dialect::Scss => self.scan_next_scss(offset),
            Dialect::Less => self.scan_next_less(),
            Dialect::Css => None,
        };
        kind.unwrap_or_else(|| self.scan_next_css(offset))
    }

    fn scan_next_scss(&mut self, offset: usize) -> Option<TokenType> {
        use TokenType::*;
        if self.advance_if(b'$') {
            self.begin_text();
            self.push('$');
            if self.ident() {
                return Some(VariableName);
            }
            self.pos = offset;
        }
        let kind = if self.advance_if_chars(b"#{") {
            InterpolationFunction
        } else if self.advance_if_chars(b"==") {
            EqualsOperator
        } else if self.advance_if_chars(b"!=") {
            NotEqualsOperator
        } else if self.advance_if(b'<') {
            if self.advance_if(b'=') {
                SmallerEqualsOperator
            } else {
                Delim
            }
        } else if self.advance_if(b'>') {
            if self.advance_if(b'=') {
                GreaterEqualsOperator
            } else {
                Delim
            }
        } else if self.advance_if_chars(b"...") {
            Ellipsis
        } else {
            return None;
        };
        Some(self.plain(kind))
    }

    fn scan_next_less(&mut self) -> Option<TokenType> {
        if self.advance_if(b'`') {
            self.advance_while(|ch| ch != b'`');
            let kind = if self.advance_if(b'`') {
                TokenType::EscapedJavaScript
            } else {
                TokenType::BadEscapedJavaScript
            };
            return Some(self.plain(kind));
        }
        if self.advance_if_chars(b"...") {
            return Some(self.plain(TokenType::Ellipsis));
        }
        None
    }

    fn scan_next_css(&mut self, offset: usize) -> TokenType {
        use TokenType::*;
        if self.advance_if_chars(b"<!--") {
            return self.plain(Cdo);
        }
        if self.advance_if_chars(b"-->") {
            return self.plain(Cdc);
        }

        self.begin_text();
        if self.ident() {
            return Ident;
        }

        if self.advance_if(b'@') {
            self.begin_text();
            self.push('@');
            if self.name() {
                // `text` scans again when it collects; the type has no use then.
                if self.buf.is_none() && self.text(&self.token(offset, AtKeyword)) == "@charset" {
                    return Charset;
                }
                return AtKeyword;
            }
            return self.plain(Delim);
        }

        if self.advance_if(b'#') {
            self.begin_text();
            self.push('#');
            if self.name() {
                return Hash;
            }
            return self.plain(Delim);
        }

        if self.advance_if(b'!') {
            return self.plain(Exclamation);
        }

        if self.number() {
            let pos = self.pos;
            self.begin_text();
            if let Some(buf) = &mut self.buf {
                buf.push_str(&self.src[offset..pos]);
            }
            if self.advance_if(b'%') {
                return self.plain(Percentage);
            } else if self.ident() {
                return dimension_type(self.slice(pos, self.pos));
            }
            return self.plain(Num);
        }

        self.begin_text();
        if let Some(kind) = self.string() {
            return kind;
        }

        if let Some(kind) = static_token(self.peek(0)) {
            self.pos += 1;
            return self.plain(kind);
        }

        let kind = match (self.peek(0), self.peek(1)) {
            (b'~', b'=') => Some(Includes),
            (b'|', b'=') => Some(Dashmatch),
            (b'*', b'=') => Some(SubstringOperator),
            (b'^', b'=') => Some(PrefixOperator),
            (b'$', b'=') => Some(SuffixOperator),
            _ => None,
        };
        if let Some(kind) = kind {
            self.pos += 2;
            return self.plain(kind);
        }

        self.pos += self.char_len();
        self.plain(Delim)
    }

    fn trivia(&mut self) -> Option<(usize, TokenType)> {
        loop {
            let offset = self.pos;
            if self.whitespace() {
                if !self.ignore_whitespace {
                    return Some((offset, self.plain(TokenType::Whitespace)));
                }
            } else if self.comment() {
                if !self.ignore_comment {
                    return Some((offset, self.plain(TokenType::Comment)));
                }
            } else {
                return None;
            }
        }
    }

    fn comment(&mut self) -> bool {
        if self.advance_if_chars(b"/*") {
            let mut success = false;
            let mut hot = false;
            self.advance_while(|ch| {
                if hot && ch == b'/' {
                    success = true;
                    return false;
                }
                hot = ch == b'*';
                true
            });
            if success {
                self.pos += 1;
            }
            return true;
        }
        if self.dialect != Dialect::Css && !self.in_url && self.advance_if_chars(b"//") {
            self.advance_while(|ch| !matches!(ch, b'\n' | b'\r' | 0x0c));
            return true;
        }
        false
    }

    fn number(&mut self) -> bool {
        let mut npeek = 0;
        let mut has_dot = false;
        if matches!(self.peek(0), b'+' | b'-') {
            npeek += 1;
        }
        if self.peek(npeek) == b'.' {
            npeek += 1;
            has_dot = true;
        }
        if self.peek(npeek).is_ascii_digit() {
            self.pos += npeek + 1;
            // Upstream accepts more than one dot after the first digit.
            self.advance_while(|ch| ch.is_ascii_digit() || !has_dot && ch == b'.');
            return true;
        }
        false
    }

    fn newline(&mut self, push: bool) -> bool {
        let ch = self.peek(0);
        if matches!(ch, b'\r' | 0x0c | b'\n') {
            self.pos += 1;
            if push {
                self.push(ch as char);
            }
            if ch == b'\r' && self.advance_if(b'\n') && push {
                self.push('\n');
            }
            return true;
        }
        false
    }

    fn escape(&mut self, include_new_lines: bool) -> bool {
        if self.peek(0) != b'\\' {
            return false;
        }
        self.pos += 1;
        let mut hex_count = 0;
        while hex_count < 6 && is_hex(self.peek(0)) {
            self.pos += 1;
            hex_count += 1;
        }
        if hex_count > 0 {
            let value =
                u32::from_str_radix(self.slice(self.pos - hex_count, self.pos), 16).unwrap();
            if value != 0 {
                // `String.fromCharCode` keeps the low 16 bits. A lone surrogate
                // cannot be in a Rust string.
                self.push(char::from_u32(value & 0xffff).unwrap_or('\u{fffd}'));
            }
            self.escaped = true;
            if matches!(self.peek(0), b' ' | b'\t') {
                self.pos += 1;
            } else {
                self.newline(false);
            }
            return true;
        }
        let ch = self.peek(0);
        if !matches!(ch, b'\r' | 0x0c | b'\n') {
            if self.eos() {
                self.pos += 1;
                self.push('\0');
            } else {
                self.take_char();
            }
            self.escaped = true;
            return true;
        } else if include_new_lines {
            self.escaped = true;
            return self.newline(true);
        }
        false
    }

    fn string_char(&mut self, close_quote: u8) -> bool {
        let ch = self.peek(0);
        if ch != 0 && ch != close_quote && !matches!(ch, b'\\' | b'\r' | 0x0c | b'\n') {
            self.take_char();
            return true;
        }
        false
    }

    fn string(&mut self) -> Option<TokenType> {
        let close_quote = self.peek(0);
        if close_quote != b'\'' && close_quote != b'"' {
            return None;
        }
        self.pos += 1;
        self.push(close_quote as char);
        while self.string_char(close_quote) || self.escape(true) {}
        if self.advance_if(close_quote) {
            self.push(close_quote as char);
            return Some(TokenType::String);
        }
        Some(TokenType::BadString)
    }

    fn unquoted_char(&mut self) -> bool {
        let ch = self.peek(0);
        if ch != 0
            && !matches!(
                ch,
                b'\\' | b'\'' | b'"' | b'(' | b')' | b' ' | b'\t' | b'\n' | 0x0c | b'\r'
            )
        {
            self.take_char();
            return true;
        }
        false
    }

    fn unquoted_string(&mut self) -> bool {
        let mut has_content = false;
        while self.unquoted_char() || self.escape(false) {
            has_content = true;
        }
        has_content
    }

    fn whitespace(&mut self) -> bool {
        self.advance_while(|ch| matches!(ch, b' ' | b'\t' | b'\n' | 0x0c | b'\r')) > 0
    }

    fn name(&mut self) -> bool {
        let mut matched = false;
        while self.ident_char() || self.escape(false) {
            matched = true;
        }
        matched
    }

    fn ident(&mut self) -> bool {
        let pos = self.pos;
        let len = self.buf.as_ref().map_or(0, String::len);
        if self.minus() {
            if self.minus() || self.ident_first_char() || self.escape(false) {
                while self.ident_char() || self.escape(false) {}
                return true;
            }
        } else if self.ident_first_char() || self.escape(false) {
            while self.ident_char() || self.escape(false) {}
            return true;
        }
        // Upstream leaves the partial text in `result`; no caller reads it.
        if let Some(buf) = &mut self.buf {
            buf.truncate(len);
        }
        self.pos = pos;
        false
    }

    fn ident_first_char(&mut self) -> bool {
        let ch = self.peek(0);
        if ch == b'_' || ch.is_ascii_alphabetic() || ch >= 0x80 {
            self.take_char();
            return true;
        }
        false
    }

    fn minus(&mut self) -> bool {
        if self.peek(0) == b'-' {
            self.pos += 1;
            self.push('-');
            return true;
        }
        false
    }

    fn ident_char(&mut self) -> bool {
        let ch = self.peek(0);
        if ch == b'_' || ch == b'-' || ch.is_ascii_alphanumeric() || ch >= 0x80 {
            self.take_char();
            return true;
        }
        false
    }

    fn unicode_range(&mut self) -> bool {
        if self.advance_if(b'+') {
            let code_points = self.advance_while(is_hex) + self.advance_while(|ch| ch == b'?');
            if (1..=6).contains(&code_points) {
                if self.advance_if(b'-') {
                    let digits = self.advance_while(is_hex);
                    if (1..=6).contains(&digits) {
                        return true;
                    }
                } else {
                    return true;
                }
            }
        }
        false
    }
}
