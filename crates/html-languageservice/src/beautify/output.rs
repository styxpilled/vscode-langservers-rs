//! `Output` and `OutputLine` of js-beautify. Character counts are UTF-16
//! lengths, as upstream.

use std::borrow::Cow;

use css_languageservice::utils::utf16_len;

/// The items of all lines are in `Output::items`: a line starts at `start`
/// and ends at the start of the next line. Only the last two lines change.
#[derive(Clone, Default)]
struct Line {
    start: usize,
    character_count: i64,
    /// -1 marks a line with preserved indentation.
    indent_count: i64,
    alignment_count: i64,
    wrap_point_index: usize,
    wrap_point_character_count: i64,
    wrap_point_indent_count: i64,
    wrap_point_alignment_count: i64,
}

impl Line {
    fn new(start: usize) -> Self {
        Line {
            start,
            indent_count: -1,
            wrap_point_indent_count: -1,
            ..Default::default()
        }
    }

    /// `set_indent`: only an empty line takes the indent.
    fn set_indent(&mut self, empty: bool, indent: i64, alignment: i64, size: i64) {
        if empty {
            self.indent_count = indent;
            self.alignment_count = alignment;
            self.character_count = size;
        }
    }
}

pub(super) struct OutputOptions {
    pub indent_size: i64,
    pub indent_with_tabs: bool,
    pub indent_char: char,
    pub end_with_newline: bool,
    pub wrap_line_length: i64,
    pub indent_empty_lines: bool,
}

pub(super) struct Output<'a> {
    indent_size: i64,
    indent_string: String,
    base_string: String,
    end_with_newline: bool,
    wrap_line_length: i64,
    indent_empty_lines: bool,
    lines: Vec<Line>,
    items: Vec<Cow<'a, str>>,
    /// Holds no items.
    next_line: Line,
    pub space_before_token: bool,
    pub non_breaking_space: bool,
    pub previous_token_wrapped: bool,
}

impl<'a> Output<'a> {
    pub fn new(options: &OutputOptions, base_indent: &str, item_count: usize) -> Self {
        let indent_string = if options.indent_with_tabs {
            options.indent_char.to_string()
        } else {
            options
                .indent_char
                .to_string()
                .repeat(options.indent_size.max(0) as usize)
        };
        let mut output = Output {
            indent_size: options.indent_size,
            indent_string,
            base_string: base_indent.to_owned(),
            end_with_newline: options.end_with_newline,
            wrap_line_length: options.wrap_line_length,
            indent_empty_lines: options.indent_empty_lines,
            lines: Vec::new(),
            items: Vec::with_capacity(item_count),
            next_line: Line::new(0),
            space_before_token: false,
            non_breaking_space: false,
            previous_token_wrapped: false,
        };
        output.add_output_line();
        output
    }

    fn current(&mut self) -> &mut Line {
        self.lines.last_mut().unwrap()
    }

    fn current_items(&self) -> &[Cow<'a, str>] {
        &self.items[self.lines.last().unwrap().start..]
    }

    fn push(&mut self, item: Cow<'a, str>) {
        let line = self.lines.last_mut().unwrap();
        match item.rfind('\n') {
            Some(i) => line.character_count = utf16_len(&item[i..]) as i64,
            None => line.character_count += utf16_len(&item) as i64,
        }
        self.items.push(item);
    }

    fn trim_current(&mut self) {
        while self.current_items().last().is_some_and(|i| i == " ") {
            self.items.pop();
            self.current().character_count -= 1;
        }
    }

    fn indent_size_of(&self, indent: i64, column: i64) -> i64 {
        let base = if indent < 0 {
            0
        } else {
            utf16_len(&self.base_string) as i64
        };
        base + indent * self.indent_size + column
    }

    pub fn indent_string(&self, indent_level: i64, column: i64) -> String {
        let mut result = String::new();
        self.push_indent(&mut result, indent_level, column);
        result
    }

    fn push_indent(&self, out: &mut String, indent_level: i64, column: i64) {
        let indent_level = if indent_level < 0 {
            0
        } else {
            out.push_str(&self.base_string);
            indent_level
        };
        let mut column = column + indent_level * self.indent_size;
        if self.indent_size > 0 && column >= self.indent_size {
            let n = column / self.indent_size;
            column -= n * self.indent_size;
            for _ in 0..n {
                out.push_str(&self.indent_string);
            }
        }
        for _ in 0..column.max(0) {
            out.push(' ');
        }
    }

    fn set_current_indent(&mut self, indent: i64, alignment: i64) {
        let size = self.indent_size_of(indent, alignment);
        let empty = self.just_added_newline();
        self.current().set_indent(empty, indent, alignment, size);
    }

    fn add_output_line(&mut self) {
        let mut line = Line::new(self.items.len());
        let (indent, alignment) = (self.next_line.indent_count, self.next_line.alignment_count);
        line.set_indent(
            true,
            indent,
            alignment,
            self.indent_size_of(indent, alignment),
        );
        self.lines.push(line);
    }

    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.items.is_empty()
    }

    pub fn add_new_line(&mut self, force_newline: bool) -> bool {
        if self.is_empty() || (!force_newline && self.just_added_newline()) {
            return false;
        }
        self.add_output_line();
        true
    }

    pub fn get_code(mut self) -> String {
        self.trim(true);
        if !self.just_added_newline() {
            let last = self.items.pop().unwrap();
            self.current().character_count -= utf16_len(&last) as i64;
            if !last.is_empty() {
                self.push(match last {
                    Cow::Borrowed(s) => Cow::Borrowed(s.trim_end_matches('\n')),
                    Cow::Owned(s) => Cow::Owned(s.trim_end_matches('\n').to_owned()),
                });
            }
        }
        if self.end_with_newline {
            self.add_output_line();
        }
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let end = self.lines.get(i + 1).map_or(self.items.len(), |l| l.start);
            let items = &self.items[line.start..end];
            if items.is_empty() {
                if self.indent_empty_lines {
                    self.push_indent(&mut out, line.indent_count, 0);
                }
            } else {
                self.push_indent(&mut out, line.indent_count, line.alignment_count);
                for item in items {
                    out += item;
                }
            }
        }
        out
    }

    pub fn set_wrap_point(&mut self) {
        if self.wrap_line_length != 0 {
            let (indent, alignment) = (self.next_line.indent_count, self.next_line.alignment_count);
            let len = self.items.len();
            let line = self.current();
            line.wrap_point_index = len - line.start;
            line.wrap_point_character_count = line.character_count;
            line.wrap_point_indent_count = indent;
            line.wrap_point_alignment_count = alignment;
        }
    }

    pub fn set_indent(&mut self, indent: i64, alignment: i64) {
        let size = self.indent_size_of(indent, alignment);
        self.next_line.set_indent(true, indent, alignment, size);
        if self.lines.len() > 1 {
            self.set_current_indent(indent, alignment);
        } else {
            self.set_current_indent(0, 0);
        }
    }

    pub fn add_raw_token(
        &mut self,
        newlines: usize,
        whitespace_before: &'a str,
        text: Cow<'a, str>,
    ) {
        for _ in 0..newlines {
            self.add_output_line();
        }
        self.set_current_indent(-1, 0);
        self.push(Cow::Borrowed(whitespace_before));
        self.push(text);
        self.space_before_token = false;
        self.non_breaking_space = false;
        self.previous_token_wrapped = false;
    }

    pub fn add_token(&mut self, text: &'a str) {
        if self.space_before_token && !self.just_added_newline() {
            if !self.non_breaking_space {
                self.set_wrap_point();
            }
            self.push(Cow::Borrowed(" "));
        }
        self.push(Cow::Borrowed(text));
        self.space_before_token = false;
        self.non_breaking_space = false;
        self.previous_token_wrapped = self.allow_wrap();
    }

    fn allow_wrap(&mut self) -> bool {
        let line = self.lines.last().unwrap();
        let should_wrap = line.wrap_point_index != 0
            && line.character_count > self.wrap_line_length
            && line.wrap_point_character_count > self.next_line.character_count;
        if !should_wrap {
            return false;
        }
        let (indent, alignment) = (
            line.wrap_point_indent_count,
            line.wrap_point_alignment_count,
        );
        let size = self.indent_size_of(indent, alignment);
        self.add_new_line(false);
        let n = self.lines.len();
        let (head, tail) = self.lines.split_at_mut(n - 1);
        let (line, next) = (&mut head[n - 2], &mut tail[0]);
        next.set_indent(true, indent, alignment, size);
        next.start = line.start + line.wrap_point_index;
        next.character_count += line.character_count - line.wrap_point_character_count;
        line.character_count = line.wrap_point_character_count;
        if self.items.get(next.start).is_some_and(|i| i == " ") {
            self.items.remove(next.start);
            next.character_count -= 1;
        }
        true
    }

    pub fn trim(&mut self, eat_newlines: bool) {
        self.trim_current();
        while eat_newlines && self.lines.len() > 1 && self.just_added_newline() {
            self.lines.pop();
            self.trim_current();
        }
    }

    pub fn just_added_newline(&self) -> bool {
        self.current_items().is_empty()
    }

    pub fn just_added_blankline(&self) -> bool {
        let n = self.lines.len();
        self.is_empty()
            || (self.just_added_newline() && self.lines[n - 2].start == self.lines[n - 1].start)
    }

    /// `current_line.has_match` with a literal pattern.
    pub fn current_line_contains(&self, pattern: &str) -> bool {
        self.current_items().iter().any(|i| i.contains(pattern))
    }
}
