//! Port of `src/beautify/beautify-css.js` (js-beautify 2.0.3, MIT license,
//! copyright (c) 2007-2018 Einar Lielmanis, Liam Newman, and contributors).
//!
//! Only the parts that the CSS beautifier uses: no alignment, no raw output,
//! `eol` is always `\n`. No line wrapping: `print_string` sets
//! `non_breaking_space` before each token, so upstream never sets a wrap point
//! and `wrap_line_length` has no effect.

use std::borrow::Cow;

use crate::utils::is_js_whitespace;

/// The options of `Beautifier`, after the conversions of `Options`.
#[derive(Clone, Debug)]
pub struct BeautifyOptions {
    pub indent_size: i64,
    pub indent_with_tabs: bool,
    pub end_with_newline: bool,
    pub selector_separator_newline: bool,
    pub newline_between_rules: bool,
    pub space_around_combinator: bool,
    pub brace_style_expand: bool,
    pub indent_empty_lines: bool,
    pub max_preserve_newlines: i64,
    /// The directive pattern of the HTML service's copy of js-beautify.
    pub loose_directives: bool,
}

/// An output line. Its items are in `Output::items`, from `start` to the
/// start of the next line.
struct Line {
    start: usize,
    /// -1 for no indent.
    indent_count: i64,
}

struct Output<'a> {
    base_indent: &'a str,
    options: &'a BeautifyOptions,
    /// The items of all lines. Only the last line gets and loses items.
    items: Vec<&'a str>,
    lines: Vec<Line>,
    next_indent: i64,
    space_before_token: bool,
}

impl<'a> Output<'a> {
    fn new(options: &'a BeautifyOptions, base_indent: &'a str) -> Self {
        Output {
            base_indent,
            options,
            items: Vec::new(),
            lines: vec![Line {
                start: 0,
                indent_count: -1,
            }],
            next_indent: -1,
            space_before_token: false,
        }
    }

    fn write_indent(&self, out: &mut String, indent: i64) {
        if indent < 0 {
            return;
        }
        out.push_str(self.base_indent);
        if self.options.indent_with_tabs {
            out.extend(std::iter::repeat_n('\t', indent as usize));
        } else {
            let n = indent * self.options.indent_size;
            out.extend(std::iter::repeat_n(' ', n as usize));
        }
    }

    fn line_items(&self, index: usize) -> &[&'a str] {
        let end = self
            .lines
            .get(index + 1)
            .map_or(self.items.len(), |l| l.start);
        &self.items[self.lines[index].start..end]
    }

    /// The start of the items of the current line.
    fn current_start(&self) -> usize {
        self.lines.last().map_or(0, |l| l.start)
    }

    fn add_output_line(&mut self) {
        self.lines.push(Line {
            start: self.items.len(),
            indent_count: self.next_indent,
        });
    }

    fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.items.is_empty()
    }

    /// The items of the line before the current line.
    fn previous_line(&self) -> Option<&[&'a str]> {
        let n = self.lines.len();
        (n > 1).then(|| self.line_items(n - 2))
    }

    fn add_new_line(&mut self, force_newline: bool) {
        if !(self.is_empty() || (!force_newline && self.just_added_newline())) {
            self.add_output_line();
        }
    }

    fn just_added_newline(&self) -> bool {
        self.items.len() == self.current_start()
    }

    fn just_added_blankline(&self) -> bool {
        self.is_empty()
            || (self.just_added_newline() && self.previous_line().is_some_and(<[_]>::is_empty))
    }

    fn set_indent(&mut self, indent: i64) {
        self.next_indent = indent;
        // Never indent the first output line.
        let indent = if self.lines.len() > 1 { indent } else { 0 };
        if self.just_added_newline()
            && let Some(line) = self.lines.last_mut()
        {
            line.indent_count = indent;
        }
    }

    fn add_token(&mut self, token: &'a str) {
        if self.space_before_token && !self.just_added_newline() {
            self.items.push(" ");
        }
        self.items.push(token);
        self.space_before_token = false;
    }

    /// Removes the spaces at the end of the current line.
    fn trim_line(&mut self) {
        let start = self.current_start();
        while self.items.len() > start && self.items.last() == Some(&" ") {
            self.items.pop();
        }
    }

    fn trim(&mut self, eat_newlines: bool) {
        self.trim_line();
        while eat_newlines && self.lines.len() > 1 && self.just_added_newline() {
            self.lines.pop();
            self.trim_line();
        }
    }

    fn ensure_empty_line_above(&mut self, starts_with: &str, ends_with: &str) {
        for index in (0..self.lines.len().saturating_sub(1)).rev() {
            let items = self.line_items(index);
            let (Some(first), Some(last)) = (items.first(), items.last()) else {
                break;
            };
            if !first.starts_with(starts_with) && *last != ends_with {
                let start = self.lines[index + 1].start;
                self.lines.insert(
                    index + 1,
                    Line {
                        start,
                        indent_count: -1,
                    },
                );
                break;
            }
        }
    }

    fn get_code(mut self) -> String {
        self.trim(true);
        // `if (last_item)`: an empty last item is dropped.
        if !self.just_added_newline()
            && let Some(last) = self.items.pop().filter(|s| !s.is_empty())
        {
            self.items.push(last.trim_end_matches('\n'));
        }
        if self.options.end_with_newline {
            self.add_output_line();
        }
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let items = self.line_items(i);
            if !items.is_empty() || self.options.indent_empty_lines {
                self.write_indent(&mut out, line.indent_count);
            }
            out.extend(items.iter().copied());
        }
        out
    }
}

struct Input<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Input<'a> {
    fn has_next(&self) -> bool {
        self.pos < self.text.len()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.text[self.pos..].chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn back(&mut self) {
        if let Some(c) = self.text[..self.pos].chars().next_back() {
            self.pos -= c.len_utf8();
        }
    }

    fn is_whitespace_next(&self) -> bool {
        self.peek().is_some_and(is_js_whitespace)
    }

    fn slice(&self, start: usize) -> &'a str {
        &self.text[start..self.pos]
    }

    /// `lookBack`: the text before the last read character, ignoring ASCII
    /// case. Callers only use it after an ASCII character.
    fn look_back(&self, test: &str) -> bool {
        let start = self.pos - 1;
        start >= test.len()
            && self.text.as_bytes()[start - test.len()..start].eq_ignore_ascii_case(test.as_bytes())
    }

    /// `peekUntilAfter(/[: ,;{}()[\]\/='"]/g)`.
    fn peek_until_after_delimiter(&self) -> &'a str {
        let rest = &self.text[self.pos..];
        let end = rest
            .find(|c| {
                matches!(
                    c,
                    ':' | ' '
                        | ','
                        | ';'
                        | '{'
                        | '}'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '/'
                        | '='
                        | '\''
                        | '"'
                )
            })
            .map_or(rest.len(), |i| i + 1);
        &rest[..end]
    }
}

/// `Directives.__directives_block_pattern`. The CSS service's copy of
/// js-beautify has `start beautify( \w+[:]\w+)+ end`, the HTML service's copy
/// has `start\s*beautify(\s+\w+[:]\w+)+\s*end` (`loose`).
fn has_directives_block(text: &str, start: &str, end: &str, loose: bool) -> bool {
    let b = text.as_bytes();
    let ws = |i: usize| {
        if loose {
            text[i..].len() - text[i..].trim_start_matches(is_js_whitespace).len()
        } else {
            usize::from(b.get(i) == Some(&b' '))
        }
    };
    text.match_indices(start).any(|(i, _)| {
        let mut j = i + start.len();
        let w = ws(j);
        if !(loose || w == 1) || !text[j + w..].starts_with("beautify") {
            return false;
        }
        j += w + "beautify".len();
        let mut groups = 0;
        loop {
            let w = ws(j);
            if groups > 0 && (loose || w == 1) && text[j + w..].starts_with(end) {
                return true;
            }
            if w == 0 {
                return false;
            }
            let name = j + w;
            let name_end = word_end(b, name);
            if name_end == name || b.get(name_end) != Some(&b':') {
                return false;
            }
            let value_end = word_end(b, name_end + 1);
            if value_end == name_end + 1 {
                return false;
            }
            j = value_end;
            groups += 1;
        }
    })
}

fn word_end(b: &[u8], mut i: usize) -> usize {
    while b
        .get(i)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        i += 1;
    }
    i
}

/// `get_directives(text).ignore === 'start'`. The directives are all the
/// matches of `/ (\w+)[:](\w+)/g` in the text; the last one wins.
pub fn is_ignore_start(text: &str, start: &str, end: &str, loose: bool) -> bool {
    if !has_directives_block(text, start, end, loose) {
        return false;
    }
    let b = text.as_bytes();
    let mut ignore = None;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b' ' {
            let name_end = word_end(b, i + 1);
            if name_end > i + 1 && b.get(name_end) == Some(&b':') {
                let value_end = word_end(b, name_end + 1);
                if value_end > name_end + 1 {
                    if &b[i + 1..name_end] == b"ignore" {
                        ignore = Some(&b[name_end + 1..value_end]);
                    }
                    i = value_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    ignore == Some(b"start")
}

/// The end of the first match of `start\sbeautify\signore:end\send` in
/// `text`.
pub fn find_ignore_end(text: &str, start: &str, end: &str) -> Option<usize> {
    text.match_indices(start).find_map(|(i, _)| {
        let mut rest = &text[i + start.len()..];
        for part in ["beautify", "ignore:end", end] {
            let mut chars = rest.chars();
            if !chars.next().is_some_and(is_js_whitespace) {
                return None;
            }
            rest = chars.as_str().strip_prefix(part)?;
        }
        Some(text.len() - rest.len())
    })
}

const NESTED_AT_RULE: [&str; 6] = [
    "page",
    "font-face",
    "keyframes",
    "media",
    "supports",
    "document",
];
const CONDITIONAL_GROUP_RULE: [&str; 3] = ["media", "supports", "document"];
const NON_SEMICOLON_NEWLINE_PROPERTY: [&str; 2] = ["grid-template-areas", "grid-template"];

struct Beautifier<'a> {
    input: Input<'a>,
    output: Output<'a>,
    options: &'a BeautifyOptions,
    indent_level: i64,
    nested_level: i64,
}

impl<'a> Beautifier<'a> {
    fn eat_string(&mut self, end_chars: &str) -> &'a str {
        let start = self.input.pos;
        while let Some(c) = self.input.next() {
            if c == '\\' {
                // Upstream appends `null` to the result at the end of the
                // input (see `docs/differences/css-formatter.md`).
                self.input.next();
            } else if end_chars.contains(c) || c == '\n' {
                break;
            }
        }
        self.input.slice(start)
    }

    fn eat_whitespace(&mut self, allow_at_least_one_new_line: bool) {
        let mut newline_count = 0;
        while self.input.is_whitespace_next() {
            let c = self.input.next();
            if allow_at_least_one_new_line
                && c == Some('\n')
                && (newline_count == 0 || newline_count < self.options.max_preserve_newlines)
            {
                newline_count += 1;
                self.output.add_new_line(true);
            }
        }
    }

    fn found_nested_pseudo_class(&self) -> bool {
        let mut open_paren = 0;
        for c in self.input.text[self.input.pos..].chars().skip(1) {
            match c {
                '{' => return true,
                '(' => open_paren += 1,
                ')' => {
                    if open_paren == 0 {
                        return false;
                    }
                    open_paren -= 1;
                }
                ';' | '}' => return false,
                _ => {}
            }
        }
        false
    }

    fn print_string(&mut self, s: &'a str) {
        self.output.set_indent(self.indent_level);
        self.output.add_token(s);
    }

    fn preserve_single_space(&mut self, is_after_space: bool) {
        if is_after_space {
            self.output.space_before_token = true;
        }
    }

    fn indent(&mut self) {
        self.indent_level += 1;
    }

    fn outdent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    /// The `$` and `@` variables: prints the name if a space or a colon
    /// follows. Returns the name, or the text up to the next delimiter.
    fn variable_or_rule(&mut self) -> &'a str {
        let peeked = self.input.peek_until_after_delimiter();
        if !(peeked.ends_with(' ') || peeked.ends_with(':')) {
            return peeked;
        }
        let variable = self.eat_string(": ").trim_end_matches(is_js_whitespace);
        self.print_string(variable);
        self.output.space_before_token = true;
        variable
    }

    fn beautify(&mut self) {
        let mut paren_level = 0;
        let mut inside_rule = false;
        let mut inside_property_value = false;
        let mut entering_conditional_group = false;
        let mut inside_non_nested_at_rule = false;
        let mut inside_scss_map = false;
        let mut top_character: Option<&str> = None;
        let mut inside_non_semicolon_values = false;

        loop {
            let ws_start = self.input.pos;
            while self.input.is_whitespace_next() {
                self.input.next();
            }
            let is_after_space = self.input.pos != ws_start;
            let previous_ch = top_character;
            let start = self.input.pos;
            let Some(c) = self.input.next() else {
                break;
            };
            if c == '\\' && self.input.has_next() {
                self.input.next();
            }
            let ch = self.input.slice(start);
            top_character = Some(ch);
            let peek = self.input.peek();
            let previous_is_quote = matches!(previous_ch, Some("\"" | "'"));

            match ch {
                "/" if peek == Some('*') => {
                    // Block comments start on a new line.
                    self.output.add_new_line(false);
                    let end = self.input.text[start + 2..]
                        .find("*/")
                        .map_or(self.input.text.len(), |i| start + 4 + i);
                    self.input.pos = end;
                    if is_ignore_start(
                        self.input.slice(start),
                        "/*",
                        "*/",
                        self.options.loose_directives,
                    ) {
                        let rest = &self.input.text[end..];
                        self.input.pos = find_ignore_end(rest, "/*", "*/")
                            .map_or(self.input.text.len(), |i| end + i);
                    }
                    let comment = self.input.slice(start);
                    self.print_string(comment);
                    self.eat_whitespace(true);
                    self.output.add_new_line(false);
                }
                "/" if peek == Some('/') => {
                    self.output.space_before_token = true;
                    let rest = &self.input.text[start..];
                    let end = rest
                        .find(['\n', '\r', '\u{2028}', '\u{2029}'])
                        .map_or(self.input.text.len(), |i| start + i);
                    self.input.pos = end;
                    let comment = self.input.slice(start);
                    self.print_string(comment);
                    self.eat_whitespace(true);
                }
                "$" => {
                    self.preserve_single_space(is_after_space);
                    self.print_string(ch);
                    let variable = self.variable_or_rule();
                    // Might be a Sass variable.
                    if paren_level == 0 && variable.contains(':') {
                        inside_property_value = true;
                        self.indent();
                    }
                }
                "@" => {
                    self.preserve_single_space(is_after_space);
                    if peek == Some('{') {
                        // Less property mixins `@{...}`.
                        self.eat_string("}");
                        let s = self.input.slice(start);
                        self.print_string(s);
                    } else {
                        self.print_string(ch);
                        let variable_or_rule = self.variable_or_rule();
                        if paren_level == 0 && variable_or_rule.contains(':') {
                            // Might be a Less variable.
                            inside_property_value = true;
                            self.indent();
                        } else if NESTED_AT_RULE.contains(&variable_or_rule) {
                            self.nested_level += 1;
                            if CONDITIONAL_GROUP_RULE.contains(&variable_or_rule) {
                                entering_conditional_group = true;
                            }
                        } else if paren_level == 0 && !inside_property_value {
                            inside_non_nested_at_rule = true;
                        }
                    }
                }
                "#" if peek == Some('{') => {
                    self.preserve_single_space(is_after_space);
                    self.eat_string("}");
                    let s = self.input.slice(start);
                    self.print_string(s);
                }
                "{" => {
                    if inside_property_value {
                        inside_property_value = false;
                        self.outdent();
                    }
                    inside_non_nested_at_rule = false;
                    // In conditional groups, only rule sets are allowed.
                    let nested_level = self.nested_level;
                    if entering_conditional_group {
                        entering_conditional_group = false;
                        inside_rule = self.indent_level >= nested_level;
                    } else {
                        inside_rule = self.indent_level >= nested_level - 1;
                    }
                    if self.options.newline_between_rules
                        && inside_rule
                        && self
                            .output
                            .previous_line()
                            .is_some_and(|l| l.last() != Some(&"{"))
                    {
                        self.output.ensure_empty_line_above("/", ",");
                    }
                    self.output.space_before_token = true;
                    if self.options.brace_style_expand {
                        self.output.add_new_line(false);
                        self.print_string(ch);
                        self.indent();
                        self.output.set_indent(self.indent_level);
                    } else {
                        // In a mixin with an object as the first parameter.
                        if previous_ch == Some("(") {
                            self.output.space_before_token = false;
                        } else if previous_ch != Some(",") {
                            self.indent();
                        }
                        self.print_string(ch);
                    }
                    self.eat_whitespace(true);
                    self.output.add_new_line(false);
                }
                "}" => {
                    self.outdent();
                    self.output.add_new_line(false);
                    if previous_ch == Some("{") {
                        self.output.trim(true);
                    }
                    if inside_property_value {
                        self.outdent();
                        inside_property_value = false;
                    }
                    self.print_string(ch);
                    inside_rule = false;
                    if self.nested_level != 0 {
                        self.nested_level -= 1;
                    }
                    self.eat_whitespace(true);
                    self.output.add_new_line(false);
                    if self.options.newline_between_rules
                        && !self.output.just_added_blankline()
                        && self.input.peek() != Some('}')
                    {
                        self.output.add_new_line(true);
                    }
                    if self.input.peek() == Some(')') {
                        self.output.trim(true);
                        if self.options.brace_style_expand {
                            self.output.add_new_line(true);
                        }
                    }
                }
                ":" => {
                    if NON_SEMICOLON_NEWLINE_PROPERTY
                        .iter()
                        .any(|p| self.input.look_back(p))
                    {
                        inside_non_semicolon_values = true;
                    }
                    if (inside_rule || entering_conditional_group)
                        && !(self.input.look_back("&") || self.found_nested_pseudo_class())
                        && !self.input.look_back("(")
                        && !inside_non_nested_at_rule
                        && paren_level == 0
                    {
                        // `property: value`, also in a conditional group query.
                        self.print_string(":");
                        if !inside_property_value {
                            inside_property_value = true;
                            self.output.space_before_token = true;
                            self.eat_whitespace(true);
                            self.indent();
                        }
                    } else {
                        // A space before a pseudo-class or pseudo-element
                        // means "in any child".
                        if self.input.look_back(" ") {
                            self.output.space_before_token = true;
                        }
                        if peek == Some(':') {
                            self.input.next();
                            self.print_string("::");
                        } else {
                            self.print_string(":");
                        }
                    }
                }
                "\"" | "'" => {
                    self.preserve_single_space(previous_is_quote || is_after_space);
                    self.eat_string(ch);
                    let s = self.input.slice(start);
                    self.print_string(s);
                    self.eat_whitespace(true);
                }
                ";" => {
                    inside_non_semicolon_values = false;
                    if paren_level == 0 {
                        if inside_property_value {
                            self.outdent();
                            inside_property_value = false;
                        }
                        inside_non_nested_at_rule = false;
                        self.print_string(ch);
                        self.eat_whitespace(true);
                        // Keeps comments on the same line.
                        if self.input.peek() != Some('/') {
                            self.output.add_new_line(false);
                        }
                    } else {
                        self.print_string(ch);
                        self.eat_whitespace(true);
                        self.output.space_before_token = true;
                    }
                }
                "(" if self.input.look_back("url") => {
                    self.print_string(ch);
                    self.eat_whitespace(false);
                    paren_level += 1;
                    self.indent();
                    let url_start = self.input.pos;
                    match self.input.next() {
                        Some(')' | '"' | '\'') => self.input.back(),
                        Some(_) => {
                            self.eat_string(")");
                            let s = self.input.slice(url_start);
                            self.print_string(s);
                            if paren_level != 0 {
                                paren_level -= 1;
                                self.outdent();
                            }
                        }
                        None => {}
                    }
                }
                "(" => {
                    let space_needed = self.input.look_back("with");
                    self.preserve_single_space(is_after_space || space_needed);
                    self.print_string(ch);
                    // A Sass map.
                    if inside_property_value
                        && previous_ch == Some("$")
                        && self.options.selector_separator_newline
                    {
                        self.output.add_new_line(false);
                        inside_scss_map = true;
                    } else {
                        self.eat_whitespace(false);
                        paren_level += 1;
                        self.indent();
                    }
                }
                ")" => {
                    if paren_level != 0 {
                        paren_level -= 1;
                        self.outdent();
                    }
                    if inside_scss_map
                        && peek == Some(';')
                        && self.options.selector_separator_newline
                    {
                        inside_scss_map = false;
                        self.outdent();
                        self.output.add_new_line(false);
                    }
                    self.print_string(ch);
                }
                "," => {
                    self.print_string(ch);
                    self.eat_whitespace(true);
                    if self.options.selector_separator_newline
                        && (!inside_property_value || inside_scss_map)
                        && paren_level == 0
                        && !inside_non_nested_at_rule
                    {
                        self.output.add_new_line(false);
                    } else {
                        self.output.space_before_token = true;
                    }
                }
                ">" | "+" | "~" if !inside_property_value && paren_level == 0 => {
                    if self.options.space_around_combinator {
                        self.output.space_before_token = true;
                        self.print_string(ch);
                        self.output.space_before_token = true;
                    } else {
                        self.print_string(ch);
                        self.eat_whitespace(false);
                    }
                }
                "]" => self.print_string(ch),
                "[" => {
                    self.preserve_single_space(is_after_space);
                    self.print_string(ch);
                }
                "=" => {
                    self.eat_whitespace(false);
                    self.print_string(ch);
                }
                "!" if !self.input.look_back("\\") => {
                    self.output.space_before_token = true;
                    self.print_string(ch);
                }
                _ => {
                    self.preserve_single_space(previous_is_quote || is_after_space);
                    self.print_string(ch);
                    if !self.output.just_added_newline()
                        && self.input.peek() == Some('\n')
                        && inside_non_semicolon_values
                    {
                        self.output.add_new_line(false);
                    }
                }
            }
        }
    }
}

/// `css_beautify`.
pub fn css_beautify(source_text: &str, options: &BeautifyOptions) -> String {
    let source: Cow<str> = if source_text.contains('\r') {
        Cow::Owned(source_text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(source_text)
    };
    let base_indent = &source[..source.len() - source.trim_start_matches([' ', '\t']).len()];
    let mut beautifier = Beautifier {
        input: Input {
            text: &source,
            pos: 0,
        },
        output: Output::new(options, base_indent),
        options,
        indent_level: 0,
        nested_level: 0,
    };
    beautifier.beautify();
    beautifier.output.get_code()
}
