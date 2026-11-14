//! Port of `src/beautify/beautify-html.js` (js-beautify 2.0.3, MIT license,
//! copyright (c) 2007-2018 Einar Lielmanis, Liam Newman, and contributors).
//!
//! `eol` is always `\n` and `indent_level` is 0: `htmlFormatter.ts` sets no
//! other value. `js_beautify` returns its input, as upstream.

mod output;
mod tokenizer;

use std::borrow::Cow;

use css_languageservice::beautify::{BeautifyOptions as CssBeautifyOptions, css_beautify};
use css_languageservice::utils::{is_js_whitespace, utf16_len};
use serde::Deserialize;

use output::{Output, OutputOptions};
use tokenizer::{Token, TokenType, Tokenizer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Templating {
    Auto,
    None,
    Angular,
    Django,
    Erb,
    Handlebars,
    Php,
    Smarty,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WrapAttributes {
    #[default]
    Auto,
    Force,
    ForceAligned,
    ForceExpandMultiline,
    AlignedMultiple,
    Preserve,
    PreserveAligned,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndentScripts {
    #[default]
    Normal,
    Keep,
    Separate,
}

/// The options of `Beautifier`, after the conversions of `Options`.
#[derive(Clone, Debug)]
pub struct BeautifyOptions {
    pub indent_size: i64,
    pub indent_with_tabs: bool,
    pub end_with_newline: bool,
    pub preserve_newlines: bool,
    pub max_preserve_newlines: i64,
    pub wrap_line_length: i64,
    pub indent_empty_lines: bool,
    /// After the replacement of `['auto']`.
    pub templating: Vec<Templating>,
    pub indent_inner_html: bool,
    pub indent_handlebars: bool,
    pub wrap_attributes: WrapAttributes,
    pub wrap_attributes_indent_size: i64,
    pub extra_liners: Vec<String>,
    pub unformatted: Vec<String>,
    pub content_unformatted: Vec<String>,
    pub unformatted_content_delimiter: String,
    pub indent_scripts: IndentScripts,
    pub void_elements: Vec<String>,
    /// The options of `css_beautify` for `<style>` content.
    pub css: CssBeautifyOptions,
}

const INLINE: [&str; 57] = [
    "a", "abbr", "area", "audio", "b", "bdi", "bdo", "br", "button", "canvas", "cite", "code",
    "data", "datalist", "del", "dfn", "em", "embed", "i", "iframe", "img", "input", "ins", "kbd",
    "keygen", "label", "map", "mark", "math", "meter", "noscript", "object", "output", "progress",
    "q", "ruby", "s", "samp", "select", "small", "span", "strong", "sub", "sup", "svg", "template",
    "textarea", "time", "u", "var", "video", "wbr", "text", "acronym", "big", "strike", "tt",
];

pub const VOID_ELEMENTS: [&str; 20] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "menuitem",
    "meta", "param", "source", "track", "wbr", "!doctype", "?xml", "basefont", "isindex",
];

const P_CLOSERS: [&str; 29] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];
const P_PARENT_EXCLUDES: [&str; 7] = ["a", "audio", "del", "ins", "map", "noscript", "video"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum CustomBeautifier {
    Css,
    Javascript,
    Html,
    Null,
}

/// `TagOpenParserToken`.
#[derive(Default)]
struct TagToken<'a> {
    parent: Option<usize>,
    text: &'a str,
    tag_name: Cow<'a, str>,
    tag_check: Cow<'a, str>,
    /// `None` for the initial token.
    tag_start_char: Option<char>,
    is_inline_element: bool,
    is_unformatted: bool,
    is_content_unformatted: bool,
    is_empty_element: bool,
    is_start_tag: bool,
    is_end_tag: bool,
    indent_content: bool,
    multiline_content: bool,
    custom_beautifier_name: Option<CustomBeautifier>,
    start_tag_token: Option<usize>,
    attr_count: i64,
    has_wrapped_attrs: bool,
    alignment_size: i64,
    tag_complete: bool,
}

/// `TagFrame`.
struct Frame<'a> {
    tag: Cow<'a, str>,
    indent_level: i64,
    parser_token: usize,
}

struct Beautifier<'a, 'o> {
    options: &'o BeautifyOptions,
    tokens: Vec<Token<'a>>,
    tags: Vec<TagToken<'a>>,
    frames: Vec<Frame<'a>>,
    output: Output<'a>,
    indent_level: i64,
    alignment_size: i64,
}

/// `html_beautify`.
pub fn html_beautify(source_text: &str, options: &BeautifyOptions) -> String {
    let source = source_text.replace("\r\n", "\n").replace('\r', "\n");
    let base_indent = &source[..source.len() - source.trim_start_matches(['\t', ' ']).len()];
    let tokens = Tokenizer::new(&source, options).tokenize();
    let tag_count = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenType::TagOpen | TokenType::Comment))
        .count();
    let mut tags = Vec::with_capacity(1 + tag_count);
    tags.push(TagToken {
        tag_complete: true,
        ..Default::default()
    });
    let item_count = tokens.len();
    let mut b = Beautifier {
        options,
        tokens,
        tags,
        frames: Vec::new(),
        output: Output::new(
            &OutputOptions {
                indent_size: options.indent_size,
                indent_with_tabs: options.indent_with_tabs,
                indent_char: if options.indent_with_tabs { '\t' } else { ' ' },
                end_with_newline: options.end_with_newline,
                wrap_line_length: options.wrap_line_length,
                indent_empty_lines: options.indent_empty_lines,
            },
            base_indent,
            item_count,
        ),
        indent_level: 0,
        alignment_size: 0,
    };
    b.beautify();
    b.output.get_code()
}

/// `get_custom_beautifier_name` for a type attribute value.
fn custom_beautifier_of(type_attribute: &str) -> Option<CustomBeautifier> {
    // `(text|application|dojo)\/(x-)?` followed by one of `names`.
    let has_type = |names: &[&str]| {
        ["text/", "application/", "dojo/"].iter().any(|prefix| {
            type_attribute.match_indices(prefix).any(|(i, _)| {
                let rest = &type_attribute[i + prefix.len()..];
                let rest = rest.strip_prefix("x-").unwrap_or(rest);
                names.iter().any(|n| rest.starts_with(n))
            })
        })
    };
    if type_attribute.contains("text/css") {
        Some(CustomBeautifier::Css)
    } else if type_attribute.contains("module")
        || type_attribute.contains("importmap")
        || has_type(&[
            "javascript",
            "ecmascript",
            "jscript",
            "livescript",
            "ld+json",
            "json",
            "method",
            "aspect",
        ])
    {
        Some(CustomBeautifier::Javascript)
    } else if has_type(&["html"]) {
        Some(CustomBeautifier::Html)
    } else if type_attribute.contains("test/null") {
        Some(CustomBeautifier::Null)
    } else {
        None
    }
}

/// `toLowerCase`, without a copy for lowercase ASCII.
fn to_lowercase(s: &str) -> Cow<'_, str> {
    if s.bytes().all(|b| b.is_ascii() && !b.is_ascii_uppercase()) {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.to_lowercase())
    }
}

/// `/^{{~?(?:[\^]|#\*?)?([^\s}]+)/`, group 1.
fn handlebars_tag_check(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("{{") else {
        return "";
    };
    fn name(s: &str) -> Option<&str> {
        let n = s
            .find(|c: char| c == '}' || is_js_whitespace(c))
            .unwrap_or(s.len());
        (n > 0).then(|| &s[..n])
    }
    let tildes = match rest.strip_prefix('~') {
        Some(r) => vec![r, rest],
        None => vec![rest],
    };
    for r in tildes {
        let mut options = Vec::new();
        if let Some(s) = r.strip_prefix('^') {
            options.push(s);
        }
        if let Some(s) = r.strip_prefix('#') {
            if let Some(t) = s.strip_prefix('*') {
                options.push(t);
            }
            options.push(s);
        }
        options.push(r);
        if let Some(n) = options.into_iter().find_map(name) {
            return n;
        }
    }
    ""
}

/// `text.replace(/\n[ \t]*$/, '')`.
fn strip_last_empty_line(text: &str) -> &str {
    let trimmed = text.trim_end_matches([' ', '\t']);
    trimmed.strip_suffix('\n').unwrap_or(text)
}

/// `/^(<!--[^\n]*|<!\[CDATA\[)(\n?)([ \t\n]*)([\s\S]*)(-->|]]>)$/`.
fn split_wrapped(text: &str) -> Option<[&str; 5]> {
    let end = text.len().checked_sub(3)?;
    if !(text.ends_with("-->") || text.ends_with("]]>")) {
        return None;
    }
    let first = if let Some(comment) = text.strip_prefix("<!--") {
        let line = comment.find('\n').map_or(text.len(), |i| 4 + i);
        if end < 4 {
            return None;
        }
        line.min(end)
    } else if text.starts_with("<![CDATA[") && end >= 9 {
        9
    } else {
        return None;
    };
    let newline = if text[first..end].starts_with('\n') {
        first + 1
    } else {
        first
    };
    let space = newline + text[newline..end].len()
        - text[newline..end]
            .trim_start_matches([' ', '\t', '\n'])
            .len();
    Some([
        &text[..first],
        &text[first..newline],
        &text[newline..space],
        &text[space..end],
        &text[end..],
    ])
}

impl<'a> Beautifier<'a, '_> {
    fn is_wrap_force(&self) -> bool {
        matches!(
            self.options.wrap_attributes,
            WrapAttributes::Force
                | WrapAttributes::ForceAligned
                | WrapAttributes::ForceExpandMultiline
        )
    }

    fn beautify(&mut self) {
        let mut last_token = TokenType::Start;
        let mut last_tag = 0;
        let mut i = 0;
        while self.tokens[i].kind != TokenType::Eof {
            let kind = self.tokens[i].kind;
            last_token = match kind {
                TokenType::TagOpen | TokenType::Comment => {
                    last_tag = self.handle_tag_open(i, last_tag, last_token);
                    TokenType::TagOpen
                }
                TokenType::Attribute | TokenType::Equals | TokenType::Value => {
                    self.handle_inside_tag(i, last_tag, last_token)
                }
                TokenType::Text if !self.tags[last_tag].tag_complete => {
                    self.handle_inside_tag(i, last_tag, last_token)
                }
                TokenType::TagClose => self.handle_tag_close(i, last_tag),
                TokenType::Text => self.handle_text(i, last_tag),
                TokenType::ControlFlowOpen => self.handle_control_flow_open(i),
                TokenType::ControlFlowClose => self.handle_control_flow_close(i),
                _ => {
                    let t = self.tokens[i].clone();
                    self.add_raw_token(&t);
                    last_token
                }
            };
            i += 1;
        }
    }

    // Printer

    fn set_space_before_token(&mut self, value: bool, non_breaking: bool) {
        self.output.space_before_token = value;
        self.output.non_breaking_space = non_breaking;
    }

    fn set_wrap_point(&mut self) {
        self.output
            .set_indent(self.indent_level, self.alignment_size);
        self.output.set_wrap_point();
    }

    fn add_raw_token(&mut self, token: &Token<'a>) {
        self.output
            .add_raw_token(token.newlines, token.whitespace_before, token.text.into());
    }

    fn print_preserved_newlines(&mut self, i: usize) -> bool {
        let token = &self.tokens[i];
        let previous = if i == 0 {
            TokenType::Start
        } else {
            self.tokens[i - 1].kind
        };
        let mut newlines = 0;
        if token.kind != TokenType::Text && previous != TokenType::Text {
            newlines = usize::from(token.newlines > 0);
        }
        if self.options.preserve_newlines {
            let max = self.options.max_preserve_newlines + 1;
            newlines = if (token.newlines as i64) < max {
                token.newlines
            } else {
                max.max(0) as usize
            };
        }
        for n in 0..newlines {
            self.output.add_new_line(n > 0);
        }
        newlines != 0
    }

    fn traverse_whitespace(&mut self, i: usize) -> bool {
        let token = &self.tokens[i];
        if !token.whitespace_before.is_empty() || token.newlines > 0 {
            if !self.print_preserved_newlines(i) {
                self.output.space_before_token = true;
            }
            return true;
        }
        false
    }

    fn print_newline(&mut self, force: bool) {
        self.output.add_new_line(force);
    }

    fn print_token(&mut self, text: &'a str) {
        if !text.is_empty() {
            self.output
                .set_indent(self.indent_level, self.alignment_size);
            self.output.add_token(text);
        }
    }

    fn deindent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
            self.output
                .set_indent(self.indent_level, self.alignment_size);
        }
    }

    fn get_full_indent(&self, level: i64) -> String {
        let level = self.indent_level + level;
        if level < 1 {
            return String::new();
        }
        self.output.indent_string(level, 0)
    }

    // TagStack

    fn record_tag(&mut self, parser_token: usize) {
        self.frames.push(Frame {
            tag: self.tags[parser_token].tag_name.clone(),
            indent_level: self.indent_level,
            parser_token,
        });
    }

    fn get_frame(&self, tag_list: &[&str], stop_list: &[&str]) -> Option<usize> {
        for (i, frame) in self.frames.iter().enumerate().rev() {
            if tag_list.contains(&&*frame.tag) {
                return Some(i);
            } else if stop_list.contains(&&*frame.tag) {
                return None;
            }
        }
        None
    }

    fn try_pop(&mut self, tag: &str, stop_list: &[&str]) -> Option<usize> {
        let i = self.get_frame(&[tag], stop_list)?;
        let frame = &self.frames[i];
        let parser_token = frame.parser_token;
        self.indent_level = frame.indent_level;
        self.frames.truncate(i);
        Some(parser_token)
    }

    fn indent_to_tag(&mut self, tag_list: &[&str]) {
        if let Some(i) = self.get_frame(tag_list, &[]) {
            self.indent_level = self.frames[i].indent_level;
        }
    }

    // Beautifier

    fn handle_control_flow_open(&mut self, i: usize) -> TokenType {
        let t = self.tokens[i].clone();
        let space = t.newlines > 0 || !t.whitespace_before.is_empty();
        self.set_space_before_token(space, true);
        if t.newlines > 0 {
            self.print_preserved_newlines(i);
        } else {
            self.set_space_before_token(space, true);
        }
        self.print_token(t.text);
        self.indent_level += 1;
        t.kind
    }

    fn handle_control_flow_close(&mut self, i: usize) -> TokenType {
        let t = self.tokens[i].clone();
        self.deindent();
        if t.newlines > 0 {
            self.print_preserved_newlines(i);
        } else {
            let space = !t.whitespace_before.is_empty();
            self.set_space_before_token(space, true);
        }
        self.print_token(t.text);
        t.kind
    }

    fn handle_tag_close(&mut self, i: usize, last_tag: usize) -> TokenType {
        let t = self.tokens[i].clone();
        self.alignment_size = 0;
        self.tags[last_tag].tag_complete = true;
        self.set_space_before_token(t.newlines > 0 || !t.whitespace_before.is_empty(), true);
        let tag = &self.tags[last_tag];
        let (is_unformatted, start_char, has_wrapped_attrs) = (
            tag.is_unformatted,
            tag.tag_start_char,
            tag.has_wrapped_attrs,
        );
        if is_unformatted {
            self.add_raw_token(&t);
        } else {
            if start_char == Some('<') {
                self.set_space_before_token(t.text.starts_with('/'), true);
                if self.options.wrap_attributes == WrapAttributes::ForceExpandMultiline
                    && has_wrapped_attrs
                {
                    self.print_newline(false);
                }
            }
            self.print_token(t.text);
        }
        let tag = &self.tags[last_tag];
        let unformatted = tag.is_unformatted || tag.is_content_unformatted;
        if tag.indent_content && !unformatted {
            self.indent_level += 1;
            self.tags[last_tag].indent_content = false;
        }
        if !self.tags[last_tag].is_inline_element && !unformatted {
            self.set_wrap_point();
        }
        t.kind
    }

    fn handle_inside_tag(&mut self, i: usize, last_tag: usize, last_token: TokenType) -> TokenType {
        let mut t = self.tokens[i].clone();
        let mut wrapped = self.tags[last_tag].has_wrapped_attrs;
        self.set_space_before_token(t.newlines > 0 || !t.whitespace_before.is_empty(), true);
        let tag = &self.tags[last_tag];
        if tag.is_unformatted {
            self.add_raw_token(&t);
        } else if tag.tag_start_char == Some('{') && t.kind == TokenType::Text {
            if self.print_preserved_newlines(i) {
                t.newlines = 0;
                self.add_raw_token(&t);
            } else {
                self.print_token(t.text);
            }
        } else {
            let previous = if i == 0 {
                TokenType::Start
            } else {
                self.tokens[i - 1].kind
            };
            match t.kind {
                TokenType::Attribute => self.set_space_before_token(true, false),
                TokenType::Equals => self.set_space_before_token(false, false),
                TokenType::Value if previous == TokenType::Equals => {
                    self.set_space_before_token(false, false)
                }
                _ => {}
            }
            if t.kind == TokenType::Attribute && self.tags[last_tag].tag_start_char == Some('<') {
                if matches!(
                    self.options.wrap_attributes,
                    WrapAttributes::Preserve | WrapAttributes::PreserveAligned
                ) {
                    self.traverse_whitespace(i);
                    wrapped = wrapped || t.newlines != 0;
                }
                if self.is_wrap_force()
                    && self.tags[last_tag].attr_count >= 2
                    && (last_token != TokenType::TagOpen
                        || self.options.wrap_attributes == WrapAttributes::ForceExpandMultiline)
                {
                    self.print_newline(false);
                    wrapped = true;
                }
            }
            self.print_token(t.text);
            wrapped = wrapped || self.output.previous_token_wrapped;
            self.tags[last_tag].has_wrapped_attrs = wrapped;
        }
        t.kind
    }

    fn handle_text(&mut self, i: usize, last_tag: usize) -> TokenType {
        let t = self.tokens[i].clone();
        let tag = &self.tags[last_tag];
        if let Some(name) = tag.custom_beautifier_name {
            self.print_custom_beautifier_text(t, name);
        } else if tag.is_unformatted || tag.is_content_unformatted {
            self.add_raw_token(&t);
        } else {
            self.traverse_whitespace(i);
            self.print_token(t.text);
        }
        TokenType::Content
    }

    fn print_custom_beautifier_text(&mut self, raw: Token<'a>, name: CustomBeautifier) {
        if raw.text.is_empty() {
            return;
        }
        let script_indent_level = match self.options.indent_scripts {
            IndentScripts::Keep => 0,
            IndentScripts::Separate => -self.indent_level,
            IndentScripts::Normal => 1,
        };
        let indentation = self.get_full_indent(script_indent_level);
        let mut text = strip_last_empty_line(raw.text);
        let mut whitespace_before = raw.whitespace_before;
        let mut pre = String::new();
        let mut post = String::new();
        if name != CustomBeautifier::Html
            && (text.starts_with("<!--") || text.starts_with("<![CDATA["))
        {
            let Some([open, newline, space, body, close]) = split_wrapped(text) else {
                self.add_raw_token(&raw);
                return;
            };
            pre = format!("{indentation}{open}\n");
            text = strip_last_empty_line(body);
            post = format!("{indentation}{close}");
            if !newline.is_empty() || space.contains('\n') {
                let trailing = space.trim_end_matches([' ', '\t']);
                if trailing.len() < space.len() {
                    whitespace_before = &space[trailing.len()..];
                }
            }
        }
        let mut text = text.to_owned();
        if !text.is_empty() {
            let source = format!("{indentation}{text}");
            text = match name {
                CustomBeautifier::Javascript => source,
                CustomBeautifier::Css => css_beautify(&source, &self.options.css),
                CustomBeautifier::Html => html_beautify(&source, self.options),
                CustomBeautifier::Null => {
                    if !whitespace_before.is_empty() {
                        text = text.replace(&format!("\n{whitespace_before}"), "\n");
                    }
                    format!(
                        "{indentation}{}",
                        text.replace('\n', &format!("\n{indentation}"))
                    )
                }
            };
        }
        if !pre.is_empty() {
            text = if text.is_empty() {
                pre + &post
            } else {
                format!("{pre}{text}\n{post}")
            };
        }
        self.print_newline(false);
        if !text.is_empty() {
            self.output.add_raw_token(0, "", text.into());
            self.print_newline(true);
        }
    }

    fn handle_tag_open(&mut self, i: usize, last_tag: usize, last_token: TokenType) -> usize {
        let pt = self.get_tag_open_token(i);
        let raw = self.tokens[i].clone();
        let last = &self.tags[last_tag];
        if (last.is_unformatted || last.is_content_unformatted)
            && !last.is_empty_element
            && raw.kind == TokenType::TagOpen
            && !self.tags[pt].is_start_tag
        {
            self.add_raw_token(&raw);
            let tag_name = self.tags[pt].tag_name.clone();
            self.tags[pt].start_tag_token = self.try_pop(&tag_name, &[]);
        } else {
            self.traverse_whitespace(i);
            self.set_tag_position(i, pt, last_tag, last_token);
            if !self.tags[pt].is_inline_element {
                self.set_wrap_point();
            }
            self.print_token(raw.text);
        }
        if self.tags[pt].is_start_tag && self.is_wrap_force() {
            let count = self.tokens[i + 1..]
                .iter()
                .take_while(|t| !matches!(t.kind, TokenType::Eof | TokenType::TagClose))
                .filter(|t| t.kind == TokenType::Attribute)
                .count();
            self.tags[pt].attr_count += count as i64;
        }
        if matches!(
            self.options.wrap_attributes,
            WrapAttributes::ForceAligned
                | WrapAttributes::AlignedMultiple
                | WrapAttributes::PreserveAligned
        ) {
            self.tags[pt].alignment_size = utf16_len(raw.text) as i64 + 1;
        }
        let tag = &self.tags[pt];
        if !tag.tag_complete && !tag.is_unformatted {
            self.alignment_size = tag.alignment_size;
        }
        pt
    }

    /// `_get_tag_open_token` with the `TagOpenParserToken` constructor.
    fn get_tag_open_token(&mut self, i: usize) -> usize {
        let raw = &self.tokens[i];
        let options = self.options;
        let tag_start_char = raw.text.chars().next();
        let tag_check = if tag_start_char == Some('<') {
            raw.text[1..]
                .split(|c: char| c == '>' || is_js_whitespace(c))
                .next()
                .unwrap_or("")
        } else {
            let mut check = handlebars_tag_check(raw.text);
            if (raw.text.starts_with("{{#>") || raw.text.starts_with("{{~#>"))
                && check.starts_with('>')
            {
                check = match self.tokens.get(i + 1) {
                    Some(next) if check == ">" => next.text.split(' ').next().unwrap_or(""),
                    _ => raw.text.split('>').nth(1).unwrap_or(""),
                };
            }
            check
        };
        let tag_check = to_lowercase(tag_check);
        let tag_complete = raw.kind == TokenType::Comment;
        let is_start_tag = !tag_check.starts_with('/');
        let tag_name = match &tag_check {
            _ if is_start_tag => tag_check.clone(),
            Cow::Borrowed(t) => Cow::Borrowed(&t[1..]),
            Cow::Owned(t) => Cow::Owned(t[1..].to_owned()),
        };
        let mut is_end_tag =
            !is_start_tag || raw.closed.is_some_and(|c| self.tokens[c].text == "/>");
        let handlebar_starts =
            if tag_start_char == Some('{') && raw.text.chars().nth(2) == Some('~') {
                3
            } else {
                2
            };
        is_end_tag = is_end_tag
            || (tag_start_char == Some('{')
                && (!options.indent_handlebars
                    || utf16_len(raw.text) < 3
                    || raw
                        .text
                        .chars()
                        .nth(handlebar_starts)
                        .is_some_and(|c| c != '#' && c != '^')));
        is_end_tag = is_end_tag || options.void_elements.iter().any(|t| *t == tag_check);
        let is_empty_element = tag_complete || (is_start_tag && is_end_tag);
        let is_inline_element =
            INLINE.contains(&&*tag_name) || tag_name.contains('-') || tag_start_char == Some('{');
        let token = TagToken {
            parent: self.frames.last().map(|f| f.parser_token),
            text: raw.text,
            is_unformatted: !tag_complete && options.unformatted.iter().any(|t| *t == tag_check),
            is_content_unformatted: !is_empty_element
                && options.content_unformatted.iter().any(|t| *t == tag_check),
            tag_name,
            tag_check,
            tag_start_char,
            is_inline_element,
            is_empty_element,
            is_start_tag,
            is_end_tag,
            alignment_size: options.wrap_attributes_indent_size,
            tag_complete,
            ..Default::default()
        };
        self.tags.push(token);
        self.tags.len() - 1
    }

    fn set_tag_position(&mut self, i: usize, pt: usize, last_tag: usize, last_token: TokenType) {
        if !self.tags[pt].is_empty_element {
            if self.tags[pt].is_end_tag {
                let tag_name = self.tags[pt].tag_name.clone();
                self.tags[pt].start_tag_token = self.try_pop(&tag_name, &[]);
            } else {
                if self.do_optional_end_element(pt) && !self.tags[pt].is_inline_element {
                    self.print_newline(false);
                }
                self.record_tag(pt);
                let tag = &self.tags[pt];
                if (tag.tag_name == "script" || tag.tag_name == "style")
                    && !(tag.is_unformatted || tag.is_content_unformatted)
                {
                    self.tags[pt].custom_beautifier_name = self.get_custom_beautifier_name(pt, i);
                }
            }
        }
        if self
            .options
            .extra_liners
            .iter()
            .any(|t| *t == self.tags[pt].tag_check)
        {
            self.print_newline(false);
            if !self.output.just_added_blankline() {
                self.print_newline(true);
            }
        }
        let tag = &self.tags[pt];
        if tag.is_empty_element {
            if tag.tag_start_char == Some('{') && tag.tag_check == "else" {
                self.indent_to_tag(&["if", "unless", "each"]);
                self.tags[pt].indent_content = true;
                if !self.output.current_line_contains("{{#if") {
                    self.print_newline(false);
                }
            }
            let tag = &self.tags[pt];
            let last = &self.tags[last_tag];
            if !(tag.tag_name == "!--"
                && last_token == TokenType::TagClose
                && last.is_end_tag
                && !tag.text.contains('\n'))
            {
                if !(tag.is_inline_element || tag.is_unformatted) {
                    self.print_newline(false);
                }
                self.calculate_parent_multiline(pt);
            }
        } else if tag.is_end_tag {
            let last = &self.tags[last_tag];
            let mut do_end_expand = tag
                .start_tag_token
                .is_some_and(|s| self.tags[s].multiline_content)
                || (!tag.is_inline_element
                    && !(last.is_inline_element || last.is_unformatted)
                    && !(last_token == TokenType::TagClose
                        && tag.start_tag_token == Some(last_tag))
                    && last_token != TokenType::Content);
            if tag.is_content_unformatted || tag.is_unformatted {
                do_end_expand = false;
            }
            if do_end_expand {
                self.print_newline(false);
            }
        } else {
            let mut indent_content = tag.custom_beautifier_name.is_none();
            if tag.tag_start_char == Some('<') {
                match &*tag.tag_name {
                    "html" => indent_content = self.options.indent_inner_html,
                    "head" | "body" => indent_content = true,
                    _ => {}
                }
            }
            self.tags[pt].indent_content = indent_content;
            let tag = &self.tags[pt];
            if !(tag.is_inline_element || tag.is_unformatted)
                && (last_token != TokenType::Content || tag.is_content_unformatted)
            {
                self.print_newline(false);
            }
            self.calculate_parent_multiline(pt);
        }
    }

    fn get_custom_beautifier_name(&self, pt: usize, i: usize) -> Option<CustomBeautifier> {
        let closed = self.tokens[i].closed?;
        let default = if self.tags[pt].tag_check == "script" {
            "text/javascript"
        } else {
            "text/css"
        };
        // `get_type_attribute`
        let mut type_attribute = None;
        for (j, t) in self.tokens.iter().enumerate().skip(i + 1) {
            if t.kind == TokenType::Eof || j == closed {
                break;
            }
            if t.kind == TokenType::Attribute && t.text == "type" {
                if let (Some(eq), Some(value)) = (self.tokens.get(j + 1), self.tokens.get(j + 2))
                    && eq.kind == TokenType::Equals
                    && value.kind == TokenType::Value
                {
                    type_attribute = Some(value.text);
                }
                break;
            }
        }
        custom_beautifier_of(type_attribute.filter(|t| !t.is_empty()).unwrap_or(default))
    }

    fn calculate_parent_multiline(&mut self, pt: usize) {
        let tag = &self.tags[pt];
        if let Some(parent) = tag.parent
            && self.output.just_added_newline()
            && !((tag.is_inline_element || tag.is_unformatted)
                && self.tags[parent].is_inline_element)
        {
            self.tags[parent].multiline_content = true;
        }
    }

    fn do_optional_end_element(&mut self, pt: usize) -> bool {
        let tag = &self.tags[pt];
        let Some(parent) = tag.parent else {
            return false;
        };
        if tag.is_empty_element || !tag.is_start_tag {
            return false;
        }
        const TABLE: &[&str] = &["table"];
        let mut result = None;
        let mut pop = |s: &mut Self, tag: &str, stop: &[&str]| {
            if result.is_none() {
                result = s.try_pop(tag, stop);
            }
        };
        match &*tag.tag_name.clone() {
            "body" => pop(self, "head", &[]),
            "li" => pop(self, "li", &["ol", "ul", "menu"]),
            "dd" | "dt" => {
                pop(self, "dt", &["dl"]);
                pop(self, "dd", &["dl"]);
            }
            name if self.tags[parent].tag_name == "p" && P_CLOSERS.contains(&name) => {
                let p_parent = self.tags[parent].parent;
                if p_parent.is_none_or(|p| !P_PARENT_EXCLUDES.contains(&&*self.tags[p].tag_name)) {
                    pop(self, "p", &[]);
                }
            }
            "rp" | "rt" => {
                pop(self, "rt", &["ruby", "rtc"]);
                pop(self, "rp", &["ruby", "rtc"]);
            }
            "optgroup" => pop(self, "optgroup", &["select"]),
            "option" => pop(self, "option", &["select", "datalist", "optgroup"]),
            "colgroup" => pop(self, "caption", TABLE),
            "thead" => {
                pop(self, "caption", TABLE);
                pop(self, "colgroup", TABLE);
            }
            "tbody" | "tfoot" => {
                pop(self, "caption", TABLE);
                pop(self, "colgroup", TABLE);
                pop(self, "thead", TABLE);
                pop(self, "tbody", TABLE);
            }
            "tr" => {
                pop(self, "caption", TABLE);
                pop(self, "colgroup", TABLE);
                pop(self, "tr", &["table", "thead", "tbody", "tfoot"]);
            }
            "th" | "td" => {
                pop(self, "td", &["table", "thead", "tbody", "tfoot", "tr"]);
                pop(self, "th", &["table", "thead", "tbody", "tfoot", "tr"]);
            }
            _ => {}
        }
        self.tags[pt].parent = self.frames.last().map(|f| f.parser_token);
        result.is_some()
    }
}
