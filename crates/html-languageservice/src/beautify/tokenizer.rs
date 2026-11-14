//! The HTML `Tokenizer` of js-beautify with its `InputScanner`, `Pattern`,
//! `TemplatablePattern` and `WhitespacePattern`. The regular expressions are
//! matched by hand.

use css_languageservice::beautify::{find_ignore_end, is_ignore_start};
use css_languageservice::utils::is_js_whitespace;

use super::{BeautifyOptions, Templating};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TokenType {
    Start,
    TagOpen,
    TagClose,
    ControlFlowOpen,
    ControlFlowClose,
    Attribute,
    Equals,
    Value,
    Comment,
    Text,
    Unknown,
    Eof,
    /// `TK_CONTENT`: the parser token of a text token.
    Content,
}

#[derive(Clone, Debug)]
pub(super) struct Token<'a> {
    pub kind: TokenType,
    pub text: &'a str,
    pub newlines: usize,
    pub whitespace_before: &'a str,
    pub opened: Option<usize>,
    pub closed: Option<usize>,
}

/// The end patterns of the `TemplatablePattern` readers.
#[derive(Clone, Copy)]
enum Until {
    /// `[\n\r\t <]`
    Word,
    /// `[\n\r\t <}]`
    WordControlFlow,
    /// `'`
    SingleQuote,
    /// `"`
    DoubleQuote,
    /// `[\n\r\t =>]|\/>`
    Attribute,
    /// `[\n\r\t >\/]`
    ElementName,
}

impl Until {
    fn matches_at(self, rest: &str) -> bool {
        let Some(c) = rest.chars().next() else {
            return false;
        };
        match self {
            Until::Word => matches!(c, '\n' | '\r' | '\t' | ' ' | '<'),
            Until::WordControlFlow => matches!(c, '\n' | '\r' | '\t' | ' ' | '<' | '}'),
            Until::SingleQuote => c == '\'',
            Until::DoubleQuote => c == '"',
            Until::Attribute => {
                matches!(c, '\n' | '\r' | '\t' | ' ' | '=' | '>') || rest.starts_with("/>")
            }
            Until::ElementName => matches!(c, '\n' | '\r' | '\t' | ' ' | '>' | '/'),
        }
    }
}

/// The `_disabled` flags of `TemplatablePattern`.
struct Disabled {
    php: bool,
    handlebars: bool,
    angular: bool,
    erb: bool,
    django: bool,
    smarty: bool,
}

pub(super) struct Tokenizer<'a, 'o> {
    text: &'a str,
    pos: usize,
    options: &'o BeautifyOptions,
    disabled: Disabled,
    newline_count: usize,
    whitespace_before: &'a str,
}

/// The start of the regular expression `</tag_name[\n\r\t ]*?>` with the
/// `i` flag in `text`, from `from`.
fn find_end_tag(text: &str, from: usize, tag_name: &str) -> Option<usize> {
    text[from..].match_indices("</").find_map(|(i, _)| {
        let start = from + i;
        let mut rest = text[start + 2..].chars();
        for t in tag_name.chars() {
            let c = rest.next()?;
            if c != t && !c.to_lowercase().eq(t.to_lowercase()) {
                return None;
            }
        }
        let rest = rest.as_str().trim_start_matches(['\n', '\r', '\t', ' ']);
        rest.starts_with('>').then_some(start)
    })
}

/// The length of the match of `<\?(?:[= ]|php)`.
fn php_start(rest: &str) -> Option<usize> {
    let r = rest.strip_prefix("<?")?;
    if r.starts_with(['=', ' ']) {
        Some(3)
    } else {
        r.starts_with("php").then_some(5)
    }
}

/// The length of the match of `<%[^%]`.
fn erb_start(rest: &str) -> Option<usize> {
    let c = rest.strip_prefix("<%")?.chars().next()?;
    (c != '%').then_some(2 + c.len_utf8())
}

/// `{(?=[^}{\s\n])`.
fn is_smarty_start(rest: &str) -> bool {
    rest.strip_prefix('{')
        .and_then(|r| r.chars().next())
        .is_some_and(|c| c != '}' && c != '{' && !is_js_whitespace(c))
}

impl<'a, 'o> Tokenizer<'a, 'o> {
    pub fn new(text: &'a str, options: &'o BeautifyOptions) -> Self {
        let off = |t: Templating| !options.templating.contains(&t);
        Tokenizer {
            text,
            pos: 0,
            options,
            disabled: Disabled {
                php: off(Templating::Php),
                handlebars: off(Templating::Handlebars),
                angular: off(Templating::Angular),
                erb: off(Templating::Erb),
                django: off(Templating::Django),
                smarty: off(Templating::Smarty),
            },
            newline_count: 0,
            whitespace_before: "",
        }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn peek(&self, index: usize) -> Option<char> {
        self.rest().chars().nth(index)
    }

    fn next_char(&mut self) -> &'a str {
        let start = self.pos;
        if let Some(c) = self.peek(0) {
            self.pos += c.len_utf8();
        }
        &self.text[start..self.pos]
    }

    fn slice(&self, start: usize) -> &'a str {
        &self.text[start..self.pos]
    }

    /// `readUntil` with a literal pattern.
    fn read_until(&mut self, pattern: &str, until_after: bool) {
        self.pos = match self.rest().find(pattern) {
            Some(i) => self.pos + i + if until_after { pattern.len() } else { 0 },
            None => self.text.len(),
        };
    }

    /// `Pattern.starting_with(open).until_after(close).read()` with literal
    /// patterns.
    fn read_pair(&mut self, open: &str, close: &str) -> &'a str {
        let start = self.pos;
        if self.rest().starts_with(open) {
            self.pos += open.len();
            self.read_until(close, true);
        }
        self.slice(start)
    }

    fn template_starts_at(&self, rest: &str) -> bool {
        let d = &self.disabled;
        (!d.php && php_start(rest).is_some())
            || ((!d.handlebars || !d.angular) && rest.starts_with("{{"))
            || (!d.erb && erb_start(rest).is_some())
            || (!d.django
                && (rest.starts_with("{%") || rest.starts_with("{{") || rest.starts_with("{#")))
            || (!d.smarty && is_smarty_start(rest))
    }

    /// `readUntil(__template_pattern)`.
    fn read_until_template(&mut self, until: Until) {
        let rest = self.rest();
        self.pos += rest
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| until.matches_at(&rest[i..]) || self.template_starts_at(&rest[i..]))
            .unwrap_or(rest.len());
    }

    /// `TemplatablePattern._read_template`: whether it read a template.
    fn read_template(&mut self, handlebars_excluded: bool) -> bool {
        let start = self.pos;
        let d = &self.disabled;
        let (php, erb, handlebars, django, smarty) = (
            !d.php,
            !d.erb,
            !d.handlebars && !handlebars_excluded,
            !d.django,
            !d.smarty && d.django && d.handlebars,
        );
        let rest = self.rest();
        if rest.starts_with('<') {
            if php && let Some(n) = php_start(rest) {
                self.pos += n;
                self.read_until("?>", true);
            } else if erb && let Some(n) = erb_start(rest) {
                self.pos += n;
                // `[^%]%>`
                let rest = self.rest();
                self.pos = rest
                    .match_indices("%>")
                    .find(|&(i, _)| rest[..i].chars().next_back().is_some_and(|c| c != '%'))
                    .map_or(self.text.len(), |(i, _)| self.pos + i + 2);
            }
        } else if rest.starts_with('{') {
            let pairs: &[(&str, &str)] = &[("{{!--", "--}}"), ("{{{", "}}}"), ("{{", "}}")];
            if handlebars {
                for (open, close) in pairs {
                    if !self.read_pair(open, close).is_empty() {
                        return true;
                    }
                }
            }
            if django {
                let value = !handlebars_excluded && !self.read_pair("{{", "}}").is_empty();
                if value
                    || !self.read_pair("{#", "#}").is_empty()
                    || !self.read_pair("{%", "%}").is_empty()
                {
                    return true;
                }
            }
            if smarty
                && (!self.read_pair("{*", "*}").is_empty()
                    || !self.read_pair("{literal}", "{/literal}").is_empty())
            {
                return true;
            }
            if smarty && is_smarty_start(rest) {
                self.pos += 1;
                // `[^\s\n]}`
                let rest = self.rest();
                self.pos = rest
                    .match_indices('}')
                    .find(|&(i, _)| {
                        rest[..i]
                            .chars()
                            .next_back()
                            .is_some_and(|c| !is_js_whitespace(c))
                    })
                    .map_or(self.text.len(), |(i, _)| self.pos + i + 1);
            }
        }
        self.pos > start
    }

    /// `TemplatablePattern.read` of `until(pattern)` or
    /// `until_after(pattern)`.
    fn read_templatable(&mut self, until: Until, handlebars_excluded: bool) -> &'a str {
        let start = self.pos;
        self.read_until_template(until);
        while self.read_template(handlebars_excluded) {
            self.read_until_template(until);
        }
        match until {
            Until::SingleQuote => self.read_until("'", true),
            Until::DoubleQuote => self.read_until("\"", true),
            _ => {}
        }
        self.slice(start)
    }

    fn read_word(&mut self, until: Until) -> &'a str {
        self.read_templatable(until, self.options.indent_handlebars)
    }

    /// `WhitespacePattern.read`.
    fn read_whitespace(&mut self) {
        let rest = self.rest();
        let n = rest.len() - rest.trim_start_matches(['\t', ' ', '\n', '\r']).len();
        self.pos += n;
        let ws = &rest[..n];
        self.newline_count = 0;
        self.whitespace_before = "";
        if ws == " " {
            self.whitespace_before = " ";
        } else if !ws.is_empty() {
            let mut last = 0;
            let mut i = 0;
            let b = ws.as_bytes();
            while i < b.len() {
                match b[i] {
                    b'\r' if b.get(i + 1) == Some(&b'\n') => {
                        self.newline_count += 1;
                        i += 2;
                        last = i;
                    }
                    b'\r' | b'\n' => {
                        self.newline_count += 1;
                        i += 1;
                        last = i;
                    }
                    _ => i += 1,
                }
            }
            self.whitespace_before = &ws[last..];
        }
    }

    fn token(&self, kind: TokenType, text: &'a str) -> Token<'a> {
        Token {
            kind,
            text,
            newlines: self.newline_count,
            whitespace_before: self.whitespace_before,
            opened: None,
            closed: None,
        }
    }

    pub fn tokenize(mut self) -> Vec<Token<'a>> {
        // About 3.5 tokens for each `<` in markup and in style sheets.
        let tag_count = self.text.bytes().filter(|&b| b == b'<').count();
        let mut tokens: Vec<Token<'a>> = Vec::with_capacity(4 * tag_count + 1);
        let mut open: Option<usize> = None;
        let mut open_stack: Vec<Option<usize>> = Vec::new();
        loop {
            let previous = tokens.last();
            let open_token = open.map(|i| &tokens[i]);
            let opened = previous.and_then(|p| p.opened).map(|i| &tokens[i]);
            let mut current = self.next_token(previous, opened, open_token);
            let index = tokens.len();
            if matches!(
                current.kind,
                TokenType::TagOpen | TokenType::ControlFlowOpen
            ) {
                open_stack.push(open);
                open = Some(index);
            } else if let Some(o) = open
                && is_closing(&current, &tokens[o])
            {
                current.opened = Some(o);
                tokens[o].closed = Some(index);
                open = open_stack.pop().flatten();
            }
            let eof = current.kind == TokenType::Eof;
            tokens.push(current);
            if eof {
                return tokens;
            }
        }
    }

    fn next_token(
        &mut self,
        previous: Option<&Token<'a>>,
        previous_opened: Option<&Token<'a>>,
        open: Option<&Token<'a>>,
    ) -> Token<'a> {
        self.read_whitespace();
        let Some(c) = self.peek(0) else {
            return self.token(TokenType::Eof, "");
        };
        let previous_kind = previous.map_or(TokenType::Start, |p| p.kind);
        // The tag name of the previous token if it closes a start tag.
        let closed_tag = previous
            .filter(|p| p.kind == TokenType::TagClose && !p.text.starts_with('/'))
            .and(previous_opened)
            .and_then(|o| o.text.strip_prefix('<'))
            .map(str::to_lowercase);
        self.read_open_handlebars(c, open)
            .or_else(|| self.read_attribute(c, previous_kind, open))
            .or_else(|| self.read_close(c, open))
            .or_else(|| self.read_script_and_style(c, closed_tag.as_deref()))
            .or_else(|| self.read_control_flows(c, open))
            .or_else(|| self.read_raw_content(open, closed_tag.as_deref()))
            .or_else(|| self.read_content_word(c, open))
            .or_else(|| self.read_comment_or_cdata(c))
            .or_else(|| self.read_processing(c))
            .or_else(|| self.read_open(c, open))
            .unwrap_or_else(|| {
                let text = self.next_char();
                self.token(TokenType::Unknown, text)
            })
    }

    fn read_comment_or_cdata(&mut self, c: char) -> Option<Token<'a>> {
        if c != '<' || self.peek(1) != Some('!') {
            return None;
        }
        let start = self.pos;
        if !self.read_pair("<!--", "-->").is_empty() {
            if is_ignore_start(self.slice(start), "<!--", "-->", true) {
                self.pos = find_ignore_end(self.rest(), "<!--", "-->")
                    .map_or(self.text.len(), |i| self.pos + i);
            }
        } else {
            self.read_pair("<![CDATA[", "]]>");
        }
        let text = self.slice(start);
        (!text.is_empty()).then(|| self.token(TokenType::Comment, text))
    }

    fn read_processing(&mut self, c: char) -> Option<Token<'a>> {
        if c != '<' || !matches!(self.peek(1), Some('!' | '?')) {
            return None;
        }
        let mut text = self.read_pair("<![", "]>");
        if text.is_empty() {
            text = self.read_pair("<?", "?>");
        }
        (!text.is_empty()).then(|| self.token(TokenType::Comment, text))
    }

    fn read_open(&mut self, c: char, open: Option<&Token>) -> Option<Token<'a>> {
        if open.is_some_and(|o| o.kind != TokenType::ControlFlowOpen) || c != '<' {
            return None;
        }
        let start = self.pos;
        self.pos += 1;
        if self.peek(0) == Some('/') {
            self.pos += 1;
        }
        self.read_templatable(Until::ElementName, false);
        Some(self.token(TokenType::TagOpen, self.slice(start)))
    }

    fn read_open_handlebars(&mut self, c: char, open: Option<&Token>) -> Option<Token<'a>> {
        if open.is_some_and(|o| o.kind != TokenType::ControlFlowOpen)
            || !(self.options.templating.contains(&Templating::Angular)
                || self.options.indent_handlebars)
            || c != '{'
            || self.peek(1) != Some('{')
        {
            return None;
        }
        if self.options.indent_handlebars && self.peek(2) == Some('!') {
            let mut text = self.read_pair("{{!--", "--}}");
            if text.is_empty() {
                text = self.read_pair("{{", "}}");
            }
            Some(self.token(TokenType::Comment, text))
        } else {
            let start = self.pos;
            let rest = self.rest();
            self.pos += rest
                .find(['\n', '\r', '\t', ' ', '}'])
                .unwrap_or(rest.len());
            Some(self.token(TokenType::TagOpen, self.slice(start)))
        }
    }

    fn read_control_flows(&mut self, c: char, open: Option<&Token>) -> Option<Token<'a>> {
        if !self.options.templating.contains(&Templating::Angular) {
            return None;
        }
        if c == '@' {
            // `\@[a-zA-Z]+[^({]*[({]`
            let rest = self.rest();
            let letters = rest[1..]
                .bytes()
                .take_while(u8::is_ascii_alphabetic)
                .count();
            if letters == 0 {
                return None;
            }
            let end = 1 + letters + rest[1 + letters..].find(['(', '{'])? + 1;
            let start = self.pos;
            self.pos += end;
            let mut text = self.slice(start);
            let mut opening = usize::from(text.ends_with('('));
            let mut closing = 0;
            while !(text.ends_with('{') && opening == closing) {
                let next = self.next_char();
                if next.is_empty() {
                    break;
                } else if next == "(" {
                    opening += 1;
                } else if next == ")" {
                    closing += 1;
                }
                text = self.slice(start);
            }
            Some(self.token(TokenType::ControlFlowOpen, text))
        } else if c == '}' && open.is_some_and(|o| o.kind == TokenType::ControlFlowOpen) {
            let text = self.next_char();
            Some(self.token(TokenType::ControlFlowClose, text))
        } else {
            None
        }
    }

    fn read_close(&mut self, c: char, open: Option<&Token>) -> Option<Token<'a>> {
        let open = open.filter(|o| o.kind == TokenType::TagOpen)?;
        let start = self.pos;
        if open.text.starts_with('<') && (c == '>' || (c == '/' && self.peek(1) == Some('>'))) {
            self.pos += if c == '/' { 2 } else { 1 };
        } else if open.text.starts_with('{') && c == '}' && self.peek(1) == Some('}') {
            self.pos += 2;
        } else {
            return None;
        }
        Some(self.token(TokenType::TagClose, self.slice(start)))
    }

    fn read_attribute(
        &mut self,
        c: char,
        previous: TokenType,
        open: Option<&Token>,
    ) -> Option<Token<'a>> {
        if !open.is_some_and(|o| o.text.starts_with('<')) {
            return None;
        }
        let start = self.pos;
        match c {
            '=' => {
                let text = self.next_char();
                Some(self.token(TokenType::Equals, text))
            }
            '"' | '\'' => {
                self.pos += 1;
                let until = if c == '"' {
                    Until::DoubleQuote
                } else {
                    Until::SingleQuote
                };
                self.read_templatable(until, false);
                Some(self.token(TokenType::Value, self.slice(start)))
            }
            _ => {
                let text = self.read_templatable(Until::Attribute, false);
                if text.is_empty() {
                    None
                } else if previous == TokenType::Equals {
                    Some(self.token(TokenType::Value, text))
                } else {
                    Some(self.token(TokenType::Attribute, text))
                }
            }
        }
    }

    fn is_content_unformatted(&self, tag_name: &str) -> bool {
        let o = self.options;
        !o.void_elements.iter().any(|t| t == tag_name)
            && (o.content_unformatted.iter().any(|t| t == tag_name)
                || o.unformatted.iter().any(|t| t == tag_name))
    }

    fn read_raw_content(
        &mut self,
        open: Option<&Token>,
        closed_tag: Option<&str>,
    ) -> Option<Token<'a>> {
        let start = self.pos;
        if open.is_some_and(|o| o.text.starts_with('{')) {
            self.read_until("}}", false);
        } else if let Some(tag_name) = closed_tag
            && self.is_content_unformatted(tag_name)
        {
            self.pos = find_end_tag(self.text, self.pos, tag_name).unwrap_or(self.text.len());
        }
        let text = self.slice(start);
        (!text.is_empty()).then(|| self.token(TokenType::Text, text))
    }

    fn read_script_and_style(&mut self, c: char, closed_tag: Option<&str>) -> Option<Token<'a>> {
        let tag_name = closed_tag.filter(|t| matches!(*t, "script" | "style"))?;
        if let Some(mut token) = self.read_comment_or_cdata(c) {
            token.kind = TokenType::Text;
            return Some(token);
        }
        let start = self.pos;
        self.pos = find_end_tag(self.text, self.pos, tag_name).unwrap_or(self.text.len());
        let text = self.slice(start);
        (!text.is_empty()).then(|| self.token(TokenType::Text, text))
    }

    fn read_content_word(&mut self, c: char, open: Option<&Token>) -> Option<Token<'a>> {
        let start = self.pos;
        let delimiter = &self.options.unformatted_content_delimiter;
        if !delimiter.is_empty()
            && delimiter.starts_with(c)
            && self.rest().starts_with(delimiter.as_str())
        {
            self.pos += delimiter.len();
            self.read_until(delimiter, true);
        }
        if self.pos == start {
            let until = if open.is_some_and(|o| o.kind == TokenType::ControlFlowOpen) {
                Until::WordControlFlow
            } else {
                Until::Word
            };
            self.read_word(until);
        }
        let text = self.slice(start);
        (!text.is_empty()).then(|| self.token(TokenType::Text, text))
    }
}

fn is_closing(current: &Token, open: &Token) -> bool {
    (current.kind == TokenType::TagClose
        && (((current.text == ">" || current.text == "/>") && open.text.starts_with('<'))
            || (current.text == "}}" && open.text.starts_with("{{"))))
        || (current.kind == TokenType::ControlFlowClose
            && current.text == "}"
            && open.text.ends_with('{'))
}
