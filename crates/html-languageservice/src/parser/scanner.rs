//! Port of `htmlScanner.ts`. Offsets are UTF-8 bytes.

use css_languageservice::utils::is_js_whitespace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    StartCommentTag,
    Comment,
    EndCommentTag,
    StartTagOpen,
    StartTagClose,
    StartTagSelfClose,
    StartTag,
    EndTagOpen,
    EndTagClose,
    EndTag,
    DelimiterAssign,
    AttributeName,
    AttributeValue,
    StartDoctypeTag,
    Doctype,
    EndDoctypeTag,
    Content,
    Whitespace,
    Unknown,
    Script,
    Styles,
    Eos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScannerState {
    WithinContent,
    AfterOpeningStartTag,
    AfterOpeningEndTag,
    WithinDoctype,
    WithinTag,
    WithinEndTag,
    WithinComment,
    WithinScriptContent,
    WithinStyleContent,
    AfterAttributeName,
    BeforeAttributeValue,
}

/// `text/x-handlebars-template` and `text/html` scripts stay in HTML.
fn is_html_script_type(value: &str) -> bool {
    matches!(value, "text/x-handlebars-template" | "text/html")
}

fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

pub struct Scanner<'a> {
    source: &'a str,
    position: usize,
    state: ScannerState,
    emit_pseudo_close_tags: bool,
    token_offset: usize,
    token_type: TokenType,
    token_error: Option<&'static str>,
    has_space_after_tag: bool,
    last_tag: &'a str,
    last_attribute_name: Option<&'a str>,
    last_type_value: Option<&'a str>,
}

impl<'a> Scanner<'a> {
    /// `createScanner`.
    pub fn new(
        input: &'a str,
        initial_offset: usize,
        initial_state: ScannerState,
        emit_pseudo_close_tags: bool,
    ) -> Self {
        Scanner {
            source: input,
            position: initial_offset,
            state: initial_state,
            emit_pseudo_close_tags,
            token_offset: 0,
            token_type: TokenType::Unknown,
            token_error: None,
            has_space_after_tag: false,
            last_tag: "",
            last_attribute_name: None,
            last_type_value: None,
        }
    }

    /// `createScanner(input, offset)` with the default state.
    pub fn at(input: &'a str, offset: usize) -> Self {
        Self::new(input, offset, ScannerState::WithinContent, false)
    }

    pub fn token_type(&self) -> TokenType {
        self.token_type
    }

    pub fn token_offset(&self) -> usize {
        self.token_offset
    }

    pub fn token_length(&self) -> usize {
        self.position - self.token_offset
    }

    pub fn token_end(&self) -> usize {
        self.position
    }

    pub fn token_text(&self) -> &'a str {
        &self.source[self.token_offset..self.position]
    }

    pub fn scanner_state(&self) -> ScannerState {
        self.state
    }

    pub fn token_error(&self) -> Option<&'static str> {
        self.token_error
    }

    pub fn scan(&mut self) -> TokenType {
        let offset = self.position;
        let token = self.internal_scan();
        if token != TokenType::Eos
            && offset == self.position
            && !(self.emit_pseudo_close_tags
                && matches!(token, TokenType::StartTagClose | TokenType::EndTagClose))
        {
            self.advance_char();
            return self.finish_token(offset, TokenType::Unknown, None);
        }
        token
    }

    fn rest(&self) -> &'a str {
        &self.source[self.position..]
    }

    fn eos(&self) -> bool {
        self.position >= self.source.len()
    }

    fn peek(&self) -> u8 {
        self.source
            .as_bytes()
            .get(self.position)
            .copied()
            .unwrap_or(0)
    }

    /// `stream.advance(1)`: one UTF-16 unit upstream, one character here.
    fn advance_char(&mut self) {
        if let Some(c) = self.rest().chars().next() {
            self.position += c.len_utf8();
        }
    }

    fn advance_if_char(&mut self, ch: u8) -> bool {
        if self.peek() == ch && !self.eos() {
            self.position += 1;
            return true;
        }
        false
    }

    fn advance_if_str(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.position += s.len();
            return true;
        }
        false
    }

    fn advance_until_char(&mut self, ch: u8) -> bool {
        match memchr(ch, self.rest()) {
            Some(i) => {
                self.position += i;
                true
            }
            None => {
                self.position = self.source.len();
                false
            }
        }
    }

    fn advance_until_str(&mut self, s: &str) -> bool {
        match self.rest().find(s) {
            Some(i) => {
                self.position += i;
                true
            }
            None => {
                self.position = self.source.len();
                false
            }
        }
    }

    fn skip_whitespace(&mut self) -> bool {
        let n = self
            .rest()
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\x0C' | b'\r'))
            .count();
        self.position += n;
        n > 0
    }

    /// `/^[_:\w][_:\w.\d-]*/`. Upstream lowercases it.
    fn next_element_name(&mut self) -> &'a str {
        let rest = self.rest();
        let b = rest.as_bytes();
        let first = |c: &u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b':');
        if !b.first().is_some_and(first) {
            return "";
        }
        let n = 1 + b[1..]
            .iter()
            .take_while(|c| first(c) || matches!(c, b'.' | b'-'))
            .count();
        self.position += n;
        &rest[..n]
    }

    /// `/^[^\s"'></=\x00-\x0F\x7F\x80-\x9F]*/`. Upstream lowercases it.
    fn next_attribute_name(&mut self) -> &'a str {
        let rest = self.rest();
        let n = rest
            .char_indices()
            .find(|&(_, c)| {
                is_js_whitespace(c)
                    || matches!(c, '"' | '\'' | '>' | '<' | '/' | '=' | '\0'..='\x0F' | '\x7F'..='\u{9F}')
            })
            .map_or(rest.len(), |(i, _)| i);
        self.position += n;
        &rest[..n]
    }

    /// No other character lowercases to an ASCII letter of `type`.
    fn last_attribute_is_type(&self) -> bool {
        self.last_attribute_name
            .is_some_and(|n| n.eq_ignore_ascii_case("type"))
    }

    /// `/^[^\s"'`=<>]+/`.
    fn next_unquoted_value(&mut self) -> &'a str {
        let rest = self.rest();
        let n = rest
            .char_indices()
            .find(|&(_, c)| is_js_whitespace(c) || matches!(c, '"' | '\'' | '`' | '=' | '<' | '>'))
            .map_or(rest.len(), |(i, _)| i);
        self.position += n;
        &rest[..n]
    }

    fn finish_token(
        &mut self,
        offset: usize,
        token_type: TokenType,
        error: Option<&'static str>,
    ) -> TokenType {
        self.token_type = token_type;
        self.token_offset = offset;
        self.token_error = error;
        token_type
    }

    fn internal_scan(&mut self) -> TokenType {
        let offset = self.position;
        if self.eos() {
            return self.finish_token(offset, TokenType::Eos, None);
        }
        let error = match self.state {
            ScannerState::WithinComment => {
                if self.advance_if_str("-->") {
                    self.state = ScannerState::WithinContent;
                    return self.finish_token(offset, TokenType::EndCommentTag, None);
                }
                self.advance_until_str("-->");
                return self.finish_token(offset, TokenType::Comment, None);
            }
            ScannerState::WithinDoctype => {
                if self.advance_if_char(b'>') {
                    self.state = ScannerState::WithinContent;
                    return self.finish_token(offset, TokenType::EndDoctypeTag, None);
                }
                self.advance_until_char(b'>');
                return self.finish_token(offset, TokenType::Doctype, None);
            }
            ScannerState::WithinContent => {
                if self.advance_if_char(b'<') {
                    if self.peek() == b'!' {
                        if self.advance_if_str("!--") {
                            self.state = ScannerState::WithinComment;
                            return self.finish_token(offset, TokenType::StartCommentTag, None);
                        }
                        if starts_with_ignore_ascii_case(self.rest(), "!doctype") {
                            self.position += "!doctype".len();
                            self.state = ScannerState::WithinDoctype;
                            return self.finish_token(offset, TokenType::StartDoctypeTag, None);
                        }
                    }
                    if self.advance_if_char(b'/') {
                        self.state = ScannerState::AfterOpeningEndTag;
                        return self.finish_token(offset, TokenType::EndTagOpen, None);
                    }
                    self.state = ScannerState::AfterOpeningStartTag;
                    return self.finish_token(offset, TokenType::StartTagOpen, None);
                }
                self.advance_until_char(b'<');
                return self.finish_token(offset, TokenType::Content, None);
            }
            ScannerState::AfterOpeningEndTag => {
                if !self.next_element_name().is_empty() {
                    self.state = ScannerState::WithinEndTag;
                    return self.finish_token(offset, TokenType::EndTag, None);
                }
                if self.skip_whitespace() {
                    return self.finish_token(
                        offset,
                        TokenType::Whitespace,
                        Some("Tag name must directly follow the open bracket."),
                    );
                }
                self.state = ScannerState::WithinEndTag;
                self.advance_until_char(b'>');
                if offset < self.position {
                    return self.finish_token(
                        offset,
                        TokenType::Unknown,
                        Some("End tag name expected."),
                    );
                }
                return self.internal_scan();
            }
            ScannerState::WithinEndTag => {
                if self.skip_whitespace() {
                    return self.finish_token(offset, TokenType::Whitespace, None);
                }
                if self.advance_if_char(b'>') {
                    self.state = ScannerState::WithinContent;
                    return self.finish_token(offset, TokenType::EndTagClose, None);
                }
                if self.emit_pseudo_close_tags && self.peek() == b'<' {
                    self.state = ScannerState::WithinContent;
                    return self.finish_token(
                        offset,
                        TokenType::EndTagClose,
                        Some("Closing bracket missing."),
                    );
                }
                Some("Closing bracket expected.")
            }
            ScannerState::AfterOpeningStartTag => {
                self.last_tag = self.next_element_name();
                self.last_type_value = None;
                self.last_attribute_name = None;
                if !self.last_tag.is_empty() {
                    self.has_space_after_tag = false;
                    self.state = ScannerState::WithinTag;
                    return self.finish_token(offset, TokenType::StartTag, None);
                }
                if self.skip_whitespace() {
                    return self.finish_token(
                        offset,
                        TokenType::Whitespace,
                        Some("Tag name must directly follow the open bracket."),
                    );
                }
                self.state = ScannerState::WithinTag;
                self.advance_until_char(b'>');
                if offset < self.position {
                    return self.finish_token(
                        offset,
                        TokenType::Unknown,
                        Some("Start tag name expected."),
                    );
                }
                return self.internal_scan();
            }
            ScannerState::WithinTag => {
                if self.skip_whitespace() {
                    self.has_space_after_tag = true;
                    return self.finish_token(offset, TokenType::Whitespace, None);
                }
                if self.has_space_after_tag {
                    let name = self.next_attribute_name();
                    let found = !name.is_empty();
                    self.last_attribute_name = Some(name);
                    if found {
                        self.state = ScannerState::AfterAttributeName;
                        self.has_space_after_tag = false;
                        return self.finish_token(offset, TokenType::AttributeName, None);
                    }
                }
                if self.advance_if_str("/>") {
                    self.state = ScannerState::WithinContent;
                    return self.finish_token(offset, TokenType::StartTagSelfClose, None);
                }
                if self.advance_if_char(b'>') {
                    let tag = self.last_tag;
                    self.state = if tag.eq_ignore_ascii_case("script") {
                        if self.last_type_value.is_some_and(is_html_script_type) {
                            ScannerState::WithinContent
                        } else {
                            ScannerState::WithinScriptContent
                        }
                    } else if tag.eq_ignore_ascii_case("style") {
                        ScannerState::WithinStyleContent
                    } else {
                        ScannerState::WithinContent
                    };
                    return self.finish_token(offset, TokenType::StartTagClose, None);
                }
                if self.emit_pseudo_close_tags && self.peek() == b'<' {
                    self.state = ScannerState::WithinContent;
                    return self.finish_token(
                        offset,
                        TokenType::StartTagClose,
                        Some("Closing bracket missing."),
                    );
                }
                self.advance_char();
                return self.finish_token(
                    offset,
                    TokenType::Unknown,
                    Some("Unexpected character in tag."),
                );
            }
            ScannerState::AfterAttributeName => {
                if self.skip_whitespace() {
                    self.has_space_after_tag = true;
                    return self.finish_token(offset, TokenType::Whitespace, None);
                }
                if self.advance_if_char(b'=') {
                    self.state = ScannerState::BeforeAttributeValue;
                    return self.finish_token(offset, TokenType::DelimiterAssign, None);
                }
                self.state = ScannerState::WithinTag;
                return self.internal_scan();
            }
            ScannerState::BeforeAttributeValue => {
                if self.skip_whitespace() {
                    return self.finish_token(offset, TokenType::Whitespace, None);
                }
                let mut value = self.next_unquoted_value();
                if !value.is_empty() {
                    // `<foo bar=http://foo/>`
                    if self.peek() == b'>' && self.source.as_bytes()[self.position - 1] == b'/' {
                        self.position -= 1;
                        value = &value[..value.len() - 1];
                    }
                    if self.last_attribute_is_type() {
                        self.last_type_value = Some(value);
                    }
                    if !value.is_empty() {
                        self.state = ScannerState::WithinTag;
                        self.has_space_after_tag = false;
                        return self.finish_token(offset, TokenType::AttributeValue, None);
                    }
                }
                let ch = self.peek();
                if ch == b'\'' || ch == b'"' {
                    self.position += 1;
                    if self.advance_until_char(ch) {
                        self.position += 1;
                    }
                    if self.last_attribute_is_type() {
                        // Upstream `substring(offset + 1, pos - 1)`: without a
                        // closing quote, the last character is cut.
                        let end = self.source[..self.position]
                            .char_indices()
                            .next_back()
                            .map_or(0, |(i, _)| i);
                        let start = offset + 1;
                        self.last_type_value = Some(if start <= end {
                            &self.source[start..end]
                        } else {
                            ""
                        });
                    }
                    self.state = ScannerState::WithinTag;
                    self.has_space_after_tag = false;
                    return self.finish_token(offset, TokenType::AttributeValue, None);
                }
                self.state = ScannerState::WithinTag;
                self.has_space_after_tag = false;
                return self.internal_scan();
            }
            ScannerState::WithinScriptContent => {
                // See http://stackoverflow.com/questions/14574471/how-do-browsers-parse-a-script-tag-exactly
                let mut script_state = 1;
                while !self.eos() {
                    let Some((index, len, kind)) = find_script_marker(self.rest()) else {
                        self.position = self.source.len();
                        return self.finish_token(offset, TokenType::Script, None);
                    };
                    self.position += index + len;
                    match kind {
                        ScriptMarker::CommentStart => {
                            if script_state == 1 {
                                script_state = 2;
                            }
                        }
                        ScriptMarker::CommentEnd => script_state = 1,
                        ScriptMarker::ScriptStart => {
                            if script_state == 2 {
                                script_state = 3;
                            }
                        }
                        ScriptMarker::ScriptEnd => {
                            if script_state == 3 {
                                script_state = 2;
                            } else {
                                self.position -= len;
                                break;
                            }
                        }
                    }
                }
                self.state = ScannerState::WithinContent;
                if offset < self.position {
                    return self.finish_token(offset, TokenType::Script, None);
                }
                return self.internal_scan();
            }
            ScannerState::WithinStyleContent => {
                let rest = self.rest();
                self.position += find_ignore_ascii_case(rest, "</style").unwrap_or(rest.len());
                self.state = ScannerState::WithinContent;
                if offset < self.position {
                    return self.finish_token(offset, TokenType::Styles, None);
                }
                return self.internal_scan();
            }
        };
        self.advance_char();
        self.state = ScannerState::WithinContent;
        self.finish_token(offset, TokenType::Unknown, error)
    }
}

fn memchr(ch: u8, s: &str) -> Option<usize> {
    s.bytes().position(|b| b == ch)
}

/// The needle starts with an ASCII byte, so each match is at a character
/// boundary.
fn find_ignore_ascii_case(s: &str, needle: &str) -> Option<usize> {
    let first = needle.as_bytes()[0];
    s.bytes()
        .enumerate()
        .find(|&(i, b)| b == first && starts_with_ignore_ascii_case(&s[i..], needle))
        .map(|(i, _)| i)
}

enum ScriptMarker {
    CommentStart,
    CommentEnd,
    ScriptStart,
    ScriptEnd,
}

/// The first match of `/<!--|-->|<\/?script\s*\/?>?/i`: index, length and kind.
fn find_script_marker(s: &str) -> Option<(usize, usize, ScriptMarker)> {
    let b = s.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'<' => {
                let rest = &s[i..];
                if rest.starts_with("<!--") {
                    return Some((i, 4, ScriptMarker::CommentStart));
                }
                let slash = rest.as_bytes().get(1) == Some(&b'/');
                let name = 1 + usize::from(slash);
                if starts_with_ignore_ascii_case(&rest[name..], "script") {
                    let mut len = name + 6;
                    len += rest[len..]
                        .char_indices()
                        .find(|&(_, c)| !is_js_whitespace(c))
                        .map_or(rest.len() - len, |(j, _)| j);
                    if rest.as_bytes().get(len) == Some(&b'/') {
                        len += 1;
                    }
                    if rest.as_bytes().get(len) == Some(&b'>') {
                        len += 1;
                    }
                    let kind = if slash {
                        ScriptMarker::ScriptEnd
                    } else {
                        ScriptMarker::ScriptStart
                    };
                    return Some((i, len, kind));
                }
            }
            b'-' if s[i..].starts_with("-->") => return Some((i, 3, ScriptMarker::CommentEnd)),
            _ => {}
        }
    }
    None
}

/// `convertScriptContentToJavaScript`: matched `<!--` and `-->` pairs become
/// `/* ` and ` */`.
pub fn convert_script_content_to_javascript(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut depth = 0;
    let mut last = 0;
    let mut i = 0;
    let b = content.as_bytes();
    while i < b.len() {
        if content[i..].starts_with("<!--") {
            out.push_str(&content[last..i]);
            out.push_str("/* ");
            depth += 1;
            i += 4;
            last = i;
        } else if content[i..].starts_with("-->") {
            if depth > 0 {
                out.push_str(&content[last..i]);
                out.push_str(" */");
                depth -= 1;
                i += 3;
                last = i;
            } else {
                i += 3;
            }
        } else {
            i += 1;
        }
    }
    out.push_str(&content[last..]);
    out
}
