use crate::{Edit, Range, ScanError, Scanner, SyntaxKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattingOptions {
    /// The number of spaces in one indent when `insert_spaces` is set. 0 means 4.
    pub tab_size: usize,
    pub insert_spaces: bool,
    /// Used only if the document has no line break.
    pub eol: String,
    pub insert_final_newline: bool,
    /// Keep the line breaks of the document.
    pub keep_lines: bool,
}

impl Default for FormattingOptions {
    fn default() -> Self {
        Self {
            tab_size: 4,
            insert_spaces: false,
            eol: "\n".to_owned(),
            insert_final_newline: false,
            keep_lines: false,
        }
    }
}

impl FormattingOptions {
    fn tab_size(&self) -> usize {
        if self.tab_size == 0 { 4 } else { self.tab_size }
    }
}

/// Computes the edits that format the document, or only the lines of `range`.
pub fn format(document_text: &str, range: Option<Range>, options: &FormattingOptions) -> Vec<Edit> {
    let (format_text_start, format_text_end, initial_indent_level, range) = match range {
        Some(range) => {
            let range_start = range.offset;
            let range_end = range_start + range.length;
            let mut start = range_start.min(document_text.len());
            while start > 0 && !is_eol(document_text, start - 1) {
                start -= 1;
            }
            let mut end = range_end.min(document_text.len());
            while end < document_text.len() && !is_eol(document_text, end) {
                end += 1;
            }
            let indent_level = compute_indent_level(&document_text[start..end], options);
            (start, end, indent_level, Some((range_start, range_end)))
        }
        None => (0, document_text.len(), 0, None),
    };

    let indent_value = if options.insert_spaces {
        " ".repeat(options.tab_size())
    } else {
        "\t".to_owned()
    };

    let mut formatter = Formatter {
        document_text,
        scanner: Scanner::new(&document_text[format_text_start..format_text_end], false),
        keep_lines: options.keep_lines,
        eol: get_eol(options, document_text),
        indent_value,
        initial_indent_level,
        indent_level: 0,
        number_line_breaks: 0,
        has_error: false,
        range,
        format_text_start,
        edits: Vec::new(),
    };
    formatter.run(options.insert_final_newline);
    formatter.edits
}

struct Formatter<'a> {
    document_text: &'a str,
    scanner: Scanner<'a>,
    keep_lines: bool,
    eol: &'a str,
    indent_value: String,
    initial_indent_level: usize,
    indent_level: isize,
    number_line_breaks: usize,
    has_error: bool,
    range: Option<(usize, usize)>,
    format_text_start: usize,
    edits: Vec<Edit>,
}

impl Formatter<'_> {
    fn new_lines_and_indent(&self) -> String {
        let level = (self.initial_indent_level as isize + self.indent_level).max(0) as usize;
        let mut result = self.eol.repeat(self.number_line_breaks.max(1));
        result.push_str(&self.indent_value.repeat(level));
        result
    }

    fn scan_next(&mut self) -> SyntaxKind {
        let mut token = self.scanner.scan();
        self.number_line_breaks = 0;
        while matches!(token, SyntaxKind::Trivia | SyntaxKind::LineBreakTrivia) {
            if token == SyntaxKind::LineBreakTrivia {
                if self.keep_lines {
                    self.number_line_breaks += 1;
                } else {
                    self.number_line_breaks = 1;
                }
            }
            token = self.scanner.scan();
        }
        self.has_error =
            token == SyntaxKind::Unknown || self.scanner.token_error() != ScanError::None;
        token
    }

    fn add_edit(&mut self, text: String, start: usize, end: usize) {
        let in_range = self
            .range
            .is_none_or(|(range_start, range_end)| start < range_end && end > range_start);
        if !self.has_error && in_range && self.document_text[start..end] != text {
            self.edits.push(Edit {
                offset: start,
                length: end - start,
                content: text,
            });
        }
    }

    fn token_start(&self) -> usize {
        self.scanner.token_offset() + self.format_text_start
    }

    fn token_end(&self) -> usize {
        self.token_start() + self.scanner.token_length()
    }

    fn run(&mut self, insert_final_newline: bool) {
        use SyntaxKind::*;

        let keep_lines = self.keep_lines;
        let mut first_token = self.scan_next();
        if keep_lines && self.number_line_breaks > 0 {
            self.add_edit(self.eol.repeat(self.number_line_breaks), 0, 0);
        }

        if first_token != EOF {
            let initial_indent = self.indent_value.repeat(self.initial_indent_level);
            self.add_edit(initial_indent, self.format_text_start, self.token_start());
        }

        while first_token != EOF {
            let mut first_token_end = self.token_end();
            let mut second_token = self.scan_next();
            let mut replace_content = String::new();
            let mut needs_line_break = false;

            while self.number_line_breaks == 0
                && matches!(second_token, LineCommentTrivia | BlockCommentTrivia)
            {
                self.add_edit(" ".to_owned(), first_token_end, self.token_start());
                first_token_end = self.token_end();
                needs_line_break = second_token == LineCommentTrivia;
                replace_content = if needs_line_break {
                    self.new_lines_and_indent()
                } else {
                    String::new()
                };
                second_token = self.scan_next();
            }

            let line_breaks = self.number_line_breaks > 0;
            if let Some(open) = match second_token {
                CloseBraceToken => Some(OpenBraceToken),
                CloseBracketToken => Some(OpenBracketToken),
                _ => None,
            } {
                if first_token != open {
                    self.indent_level -= 1;
                }
                if (keep_lines && line_breaks) || (!keep_lines && first_token != open) {
                    replace_content = self.new_lines_and_indent();
                } else if keep_lines {
                    replace_content = " ".to_owned();
                }
            } else {
                match first_token {
                    OpenBracketToken | OpenBraceToken | CommaToken => {
                        if matches!(first_token, OpenBracketToken | OpenBraceToken) {
                            self.indent_level += 1;
                        }
                        if !keep_lines || line_breaks {
                            replace_content = self.new_lines_and_indent();
                        } else {
                            replace_content = " ".to_owned();
                        }
                    }
                    LineCommentTrivia => replace_content = self.new_lines_and_indent(),
                    BlockCommentTrivia => {
                        if line_breaks {
                            replace_content = self.new_lines_and_indent();
                        } else if !needs_line_break {
                            replace_content = " ".to_owned();
                        }
                    }
                    ColonToken => {
                        if keep_lines && line_breaks {
                            replace_content = self.new_lines_and_indent();
                        } else if !needs_line_break {
                            replace_content = " ".to_owned();
                        }
                    }
                    StringLiteral => {
                        if keep_lines && line_breaks {
                            replace_content = self.new_lines_and_indent();
                        } else if second_token == ColonToken && !needs_line_break {
                            replace_content = String::new();
                        }
                    }
                    NullKeyword | TrueKeyword | FalseKeyword | NumericLiteral | CloseBraceToken
                    | CloseBracketToken => {
                        if keep_lines && line_breaks {
                            replace_content = self.new_lines_and_indent();
                        } else if matches!(second_token, LineCommentTrivia | BlockCommentTrivia)
                            && !needs_line_break
                        {
                            replace_content = " ".to_owned();
                        } else if !matches!(second_token, CommaToken | EOF) {
                            self.has_error = true;
                        }
                    }
                    Unknown => self.has_error = true,
                    _ => {}
                }
                if line_breaks && matches!(second_token, LineCommentTrivia | BlockCommentTrivia) {
                    replace_content = self.new_lines_and_indent();
                }
            }
            if second_token == EOF {
                replace_content = if keep_lines && line_breaks {
                    self.new_lines_and_indent()
                } else if insert_final_newline {
                    self.eol.to_owned()
                } else {
                    String::new()
                };
            }
            self.add_edit(replace_content, first_token_end, self.token_start());
            first_token = second_token;
        }
    }
}

fn compute_indent_level(content: &str, options: &FormattingOptions) -> usize {
    let tab_size = options.tab_size();
    let mut n_chars = 0;
    for b in content.bytes() {
        match b {
            b' ' => n_chars += 1,
            b'\t' => n_chars += tab_size,
            _ => break,
        }
    }
    n_chars / tab_size
}

/// The first line break of the text, else the line break of the options.
fn get_eol<'a>(options: &'a FormattingOptions, text: &'a str) -> &'a str {
    let bytes = text.as_bytes();
    match bytes.iter().position(|&b| b == b'\r' || b == b'\n') {
        Some(i) if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') => "\r\n",
        Some(i) if bytes[i] == b'\r' => "\r",
        Some(_) => "\n",
        None if options.eol.is_empty() => "\n",
        None => &options.eol,
    }
}

pub(crate) fn is_eol(text: &str, offset: usize) -> bool {
    text.as_bytes()
        .get(offset)
        .is_some_and(|&b| crate::scanner::is_line_break(b))
}
