//! The style sheet printer of the formatter. The parse tree gives the
//! structure; the scanner gives the tokens, so each token of the source is
//! printed. A statement with a parse error keeps its source text.

use std::ops::Range;

use crate::parser::nodes::{Ast, Field, Kind, NodeId, NodeType};
use crate::parser::scanner::{Scanner, TokenType};
use crate::parser::{Dialect, Stylesheet, errors, parse_stylesheet};

#[derive(Clone, Debug)]
pub struct PrintOptions {
    pub indent_size: usize,
    pub indent_with_tabs: bool,
    pub end_with_newline: bool,
    pub selector_separator_newline: bool,
    pub newline_between_rules: bool,
    pub space_around_combinator: bool,
    pub brace_style_expand: bool,
    pub indent_empty_lines: bool,
    /// The most line breaks kept between two statements. 0 and 1 keep no
    /// empty line.
    pub max_preserve_newlines: usize,
}

/// Formats a whole style sheet. Each line starts with the indentation of the
/// first line of `source`.
pub fn format_text(source: &str, dialect: Dialect, options: &PrintOptions) -> String {
    let base_indent = &source[..source.len() - source.trim_start_matches([' ', '\t']).len()];
    let stylesheet = parse_stylesheet(dialect, source);
    let printer = Printer::new(&stylesheet, dialect, options, base_indent);
    match printer.print(None) {
        Some((_, text)) => text,
        None => source.to_owned(),
    }
}

/// Formats the style sheet, or the statements of one block that `range`
/// touches. Returns the replaced byte range and its new text; `None` when
/// there is nothing to format.
pub fn print(
    stylesheet: &Stylesheet,
    dialect: Dialect,
    options: &PrintOptions,
    range: Option<Range<usize>>,
) -> Option<(Range<usize>, String)> {
    Printer::new(stylesheet, dialect, options, "").print(range)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Statement(NodeId),
    Comment,
    Semicolon,
    /// Tokens that no statement covers.
    Other,
}

/// A statement, or a token between the statements of a block.
#[derive(Clone, Copy)]
struct Item {
    kind: ItemKind,
    start: usize,
    end: usize,
    /// Line breaks between the item before and this one.
    newlines: usize,
}

/// What the tree says about the tokens of one statement, outside its blocks.
#[derive(Default)]
struct Marks {
    dirty: bool,
    blocks: Vec<NodeId>,
    /// Printed as they are: the parser scans them in other modes.
    atomic: Vec<Range<usize>>,
    selectors: Vec<Range<usize>>,
    /// The ranges between the selectors of a rule set.
    selector_separators: Vec<Range<usize>>,
    /// Ranges that contain a `:` between a name and a value.
    colon_ranges: Vec<Range<usize>>,
    colons: Vec<usize>,
    combinators: Vec<usize>,
    values: Vec<Range<usize>>,
}

impl Marks {
    fn clear(&mut self) {
        self.dirty = false;
        self.blocks.clear();
        self.atomic.clear();
        self.selectors.clear();
        self.selector_separators.clear();
        self.colon_ranges.clear();
        self.colons.clear();
        self.combinators.clear();
        self.values.clear();
    }
}

/// The value of the last `ignore:` of a `/* beautify ... */` comment.
fn ignore_directive(comment: &str) -> Option<&str> {
    let body = comment.strip_prefix("/*")?.strip_suffix("*/")?;
    let rest = body.trim_start().strip_prefix("beautify")?;
    rest.split_whitespace()
        .filter_map(|w| w.strip_prefix("ignore:"))
        .last()
}

fn contains(ranges: &[Range<usize>], offset: usize) -> bool {
    let i = ranges.partition_point(|r| r.end <= offset);
    ranges.get(i).is_some_and(|r| r.start <= offset)
}

enum Printed {
    /// The source text, for a statement with a parse error.
    Source,
    Closed,
    /// With a `(` or `[` that is not closed.
    Open,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sep {
    None,
    Space,
    Line(usize),
}

/// The previous token of a statement.
#[derive(Clone, Copy)]
struct Prev<'a> {
    text: &'a str,
    end: usize,
    line_comment: bool,
    block_end: bool,
    colon: bool,
    selector_separator: bool,
    combinator: bool,
}

struct Printer<'a> {
    ast: &'a Ast,
    root: NodeId,
    text: &'a str,
    dialect: Dialect,
    options: &'a PrintOptions,
    base_indent: &'a str,
    out: String,
    /// The marks of the next statement, from the check of a `;` before it.
    next_marks: Option<(NodeId, Marks)>,
    // Buffers for reuse.
    spare_marks: Vec<Marks>,
    spare_items: Vec<Vec<Item>>,
    stack: Vec<NodeId>,
    spans: Vec<(usize, usize)>,
}

impl<'a> Printer<'a> {
    fn new(
        stylesheet: &'a Stylesheet,
        dialect: Dialect,
        options: &'a PrintOptions,
        base_indent: &'a str,
    ) -> Self {
        Printer {
            ast: &stylesheet.ast,
            root: stylesheet.root,
            text: stylesheet.ast.text(),
            dialect,
            options,
            base_indent,
            out: String::with_capacity(stylesheet.ast.text().len()),
            next_marks: None,
            spare_marks: Vec::new(),
            spare_items: Vec::new(),
            stack: Vec::new(),
            spans: Vec::new(),
        }
    }

    fn print(mut self, range: Option<Range<usize>>) -> Option<(Range<usize>, String)> {
        if self.ast.has_issue(self.root, &errors::NESTING_TOO_DEEP) {
            return None;
        }
        let len = self.text.len();
        let range = range.filter(|r| r.start > 0 || r.end < len);
        let Some(range) = range else {
            let items = self.items(self.root, 0..len);
            self.out.push_str(self.base_indent);
            self.print_items(self.root, &items, 0..items.len(), 0, false);
            let end = self.out.trim_end().len();
            self.out.truncate(end);
            if self.out.len() == self.base_indent.len() {
                self.out.clear();
            }
            if self.options.end_with_newline {
                self.out.push('\n');
            }
            return Some((0..len, self.out));
        };

        let (mut parent, mut interior, mut level) = (self.root, 0..len, 0);
        loop {
            let mut touched = self.ast.children(parent).filter(|&c| {
                let (start, end) = self.span(c);
                start < range.end.max(range.start + 1) && end > range.start
            });
            let (Some(only), None) = (touched.next(), touched.next()) else {
                break;
            };
            let marks = self.marks(only);
            let inner = marks.blocks.iter().copied().find(|&b| {
                let (start, end) = self.span(b);
                start < range.start && range.end < end
            });
            let dirty = marks.dirty;
            self.spare_marks.push(marks);
            match inner {
                Some(block) if !dirty => {
                    let (start, end) = self.span(block);
                    (parent, interior, level) = (block, start + 1..end - 1, level + 1);
                }
                _ => break,
            }
        }
        let items = self.items(parent, interior);
        let first = items.iter().position(|i| i.end > range.start)?;
        let last = items
            .iter()
            .rposition(|i| i.start < range.end.max(range.start + 1))?;
        if last < first {
            return None;
        }
        self.print_items(parent, &items, first..last + 1, level, false);
        Some((items[first].start..items[last].end, self.out))
    }

    /// Some nodes of an unclosed block end after the text.
    fn span(&self, id: NodeId) -> (usize, usize) {
        let len = self.text.len();
        let start = (self.ast.offset(id) as usize).min(len);
        (start, (self.ast.end(id) as usize).clamp(start, len))
    }

    fn scanner(&self, from: usize) -> Scanner<'a> {
        let mut scanner = Scanner::new(self.dialect);
        scanner.set_source(self.text);
        scanner.go_back_to(from);
        scanner.ignore_comment = false;
        scanner
    }

    /// The statements of `parent` and the tokens between them in `interior`.
    fn items(&mut self, parent: NodeId, interior: Range<usize>) -> Vec<Item> {
        let mut items = self.spare_items.pop().unwrap_or_default();
        items.clear();
        let mut pos = interior.start;
        // A statement before `pos` stays in the gap tokens.
        for s in self.ast.children(parent) {
            let (start, end) = self.span(s);
            if start < pos || end > interior.end {
                continue;
            }
            self.gap_items(&mut items, pos..start);
            items.push(Item {
                kind: ItemKind::Statement(s),
                start,
                end,
                newlines: 0,
            });
            pos = end;
        }
        self.gap_items(&mut items, pos..interior.end);
        let mut pos = interior.start;
        for item in &mut items {
            item.newlines = self.text[pos..item.start].matches('\n').count();
            pos = item.end;
        }
        items
    }

    fn gap_items(&self, items: &mut Vec<Item>, gap: Range<usize>) {
        if gap.is_empty() {
            return;
        }
        let mut scanner = self.scanner(gap.start);
        loop {
            let token = scanner.scan();
            if token.kind == TokenType::Eof || token.offset >= gap.end {
                return;
            }
            let end = token.end().min(gap.end);
            let kind = match token.kind {
                TokenType::Comment | TokenType::SingleLineComment => ItemKind::Comment,
                TokenType::SemiColon => ItemKind::Semicolon,
                _ => ItemKind::Other,
            };
            match items.last_mut() {
                Some(last) if kind == ItemKind::Other && last.kind == ItemKind::Other => {
                    last.end = end;
                }
                _ => items.push(Item {
                    kind,
                    start: token.offset,
                    end,
                    newlines: 0,
                }),
            }
        }
    }

    fn write_indent(&mut self, level: usize) {
        self.out.push_str(self.base_indent);
        if self.options.indent_with_tabs {
            self.out.extend(std::iter::repeat_n('\t', level));
        } else {
            let n = level * self.options.indent_size;
            self.out.extend(std::iter::repeat_n(' ', n));
        }
    }

    fn newline(&mut self, level: usize, empty_lines: usize) {
        for _ in 0..empty_lines {
            self.out.push('\n');
            if self.options.indent_empty_lines {
                self.write_indent(level);
            }
        }
        self.out.push('\n');
        self.write_indent(level);
    }

    fn is_block(&self, statement: NodeId) -> bool {
        let (start, end) = self.span(statement);
        self.text[start..end].ends_with('}')
    }

    fn print_items(
        &mut self,
        parent: NodeId,
        items: &[Item],
        selected: Range<usize>,
        level: usize,
        newline_first: bool,
    ) {
        let first = selected.start;
        // An error of the parent: the statement bounds come from the error
        // recovery.
        let parent_clean = !self.ast.is_erroneous(parent, false);
        let mut previous: Option<NodeId> = None;
        // After a statement with a parse error or other tokens, and before
        // other tokens, the source whitespace stays.
        let mut raw = false;
        let mut skip_to = first;
        for i in selected.clone() {
            if i < skip_to {
                continue;
            }
            let item = items[i];
            let text = &self.text[item.start..item.end];
            if i > first && (raw || item.kind == ItemKind::Other) {
                self.out.push_str(&self.text[items[i - 1].end..item.start]);
            } else if item.kind == ItemKind::Semicolon {
                self.out.push(';');
                continue;
            } else if item.kind == ItemKind::Comment && i > first && item.newlines == 0 {
                self.out.push(' ');
            } else if i > first || newline_first {
                let mut empty_lines = if i == first {
                    0
                } else {
                    item.newlines
                        .min(self.options.max_preserve_newlines)
                        .saturating_sub(1)
                };
                if let Some(p) = previous.take()
                    && self.options.newline_between_rules
                    && (self.is_block(p)
                        || self
                            .next_statement(items, i)
                            .is_some_and(|s| self.is_block(s)))
                {
                    empty_lines = empty_lines.max(1);
                }
                self.newline(level, empty_lines);
            }
            raw = false;
            match item.kind {
                ItemKind::Statement(s) => {
                    previous = Some(s);
                    match self.statement(s, level) {
                        Printed::Source => raw = true,
                        Printed::Closed if parent_clean && self.needs_semicolon(s, items, i) => {
                            self.out.push(';');
                        }
                        _ => {}
                    }
                }
                ItemKind::Comment if ignore_directive(text) == Some("start") => {
                    let end = (i + 1..selected.end)
                        .find(|&j| {
                            items[j].kind == ItemKind::Comment
                                && ignore_directive(&self.text[items[j].start..items[j].end])
                                    == Some("end")
                        })
                        .unwrap_or(selected.end - 1);
                    self.out.push_str(&self.text[item.start..items[end].end]);
                    skip_to = end + 1;
                    raw = true;
                }
                ItemKind::Other => {
                    raw = true;
                    self.out.push_str(text);
                }
                _ => self.out.push_str(text),
            }
        }
    }

    /// A statement that ends with `;`, without one, before the end of its
    /// block or before a statement without parse errors.
    fn needs_semicolon(&mut self, statement: NodeId, items: &[Item], i: usize) -> bool {
        let takes_semicolon = matches!(
            self.ast.node_type(statement),
            NodeType::Declaration
                | NodeType::CustomPropertyDeclaration
                | NodeType::VariableDeclaration
                | NodeType::MixinReference
                | NodeType::MixinContentReference
                | NodeType::ExtendsReference
                | NodeType::ReturnStatement
                | NodeType::Debug
                | NodeType::Import
                | NodeType::Use
                | NodeType::Forward
                | NodeType::Namespace
                | NodeType::Layer
                | NodeType::Plugin
                | NodeType::AtApplyRule
                | NodeType::UnknownAtRule
        );
        let (start, end) = self.span(statement);
        if !takes_semicolon || self.text[start..end].ends_with([';', '}']) {
            return false;
        }
        match items[i + 1..].iter().find(|i| i.kind != ItemKind::Comment) {
            None => true,
            Some(Item {
                kind: ItemKind::Statement(next),
                ..
            }) => {
                let marks = self.marks(*next);
                let clean = !marks.dirty;
                self.next_marks = Some((*next, marks));
                clean
            }
            Some(_) => false,
        }
    }

    fn next_statement(&self, items: &[Item], from: usize) -> Option<NodeId> {
        items[from..].iter().find_map(|i| match i.kind {
            ItemKind::Statement(s) => Some(s),
            _ => None,
        })
    }

    fn block(&mut self, block: NodeId, level: usize) {
        let (start, end) = self.span(block);
        let items = self.items(block, start + 1..end - 1);
        if items.is_empty() {
            self.out.push_str("{}");
        } else {
            self.out.push('{');
            self.print_items(block, &items, 0..items.len(), level + 1, true);
            self.newline(level, 0);
            self.out.push('}');
        }
        self.spare_items.push(items);
    }

    fn marks(&mut self, statement: NodeId) -> Marks {
        let mut marks = self.spare_marks.pop().unwrap_or_default();
        marks.clear();
        let mut stack = std::mem::take(&mut self.stack);
        let mut spans = std::mem::take(&mut self.spans);
        stack.clear();
        stack.push(statement);
        self.collect_marks(&mut marks, &mut stack, &mut spans);
        self.stack = stack;
        self.spans = spans;
        marks
    }

    fn collect_marks(
        &self,
        marks: &mut Marks,
        stack: &mut Vec<NodeId>,
        spans: &mut Vec<(usize, usize)>,
    ) {
        let ast = self.ast;
        while let Some(n) = stack.pop() {
            if ast.is_erroneous(n, false) {
                marks.dirty = true;
                return;
            }
            let kind = ast.kind(n);
            let (start, end) = self.span(n);
            if kind == Kind::Declarations {
                // Error recovery can end a block before its `}`.
                if !self.text[start..end].ends_with('}') || end - start < 2 {
                    marks.dirty = true;
                    return;
                }
                marks.blocks.push(n);
                continue;
            }
            match ast.node_type(n) {
                NodeType::URILiteral | NodeType::UnicodeRange => {
                    marks.atomic.push(start..end);
                    continue;
                }
                NodeType::SelectorCombinator
                | NodeType::SelectorCombinatorParent
                | NodeType::SelectorCombinatorSibling
                | NodeType::SelectorCombinatorAllSiblings => marks.combinators.push(start),
                NodeType::MediaFeature
                | NodeType::ListEntry
                | NodeType::FunctionArgument
                | NodeType::FunctionParameter
                | NodeType::ModuleConfiguration => {
                    spans.clear();
                    spans.extend(ast.children(n).map(|c| self.span(c)));
                    spans.sort_unstable();
                    if let [(_, name_end), (value_start, _), ..] = spans[..] {
                        marks.colon_ranges.push(name_end..value_start);
                    }
                }
                _ => {}
            }
            if kind == Kind::RuleSet
                && let Some(selectors) = ast.field(n, Field::Selectors)
            {
                let (s, e) = self.span(selectors);
                marks.selectors.push(s..e);
                spans.clear();
                spans.extend(ast.children(selectors).map(|c| self.span(c)));
                spans.sort_unstable();
                for pair in spans.windows(2) {
                    marks.selector_separators.push(pair[0].1..pair[1].0);
                }
            }
            if kind.is_abstract_declaration()
                && let Some(colon) = ast.colon_position(n)
            {
                let colon = colon as usize;
                marks.colons.push(colon);
                marks.values.push(colon + 1..end);
                if kind == Kind::CustomPropertyDeclaration {
                    for c in ast.children(n) {
                        let (s, e) = self.span(c);
                        if s > colon && e > s && ast.node_type(c) != NodeType::Prio {
                            marks.atomic.push(s..e);
                        }
                    }
                    continue;
                }
            }
            stack.extend(ast.children(n));
        }
        marks.blocks.sort_by_key(|&b| ast.offset(b));
        marks.atomic.sort_by_key(|r| r.start);
        marks.selectors.sort_by_key(|r| r.start);
        marks.selector_separators.sort_by_key(|r| r.start);
        marks.colon_ranges.sort_by_key(|r| r.start);
        marks.colons.sort_unstable();
        marks.combinators.sort_unstable();
        marks.values.sort_by_key(|r| r.start);
    }

    fn statement(&mut self, statement: NodeId, level: usize) -> Printed {
        let marks = match self.next_marks.take() {
            Some((id, marks)) if id == statement => marks,
            _ => self.marks(statement),
        };
        let printed = self.print_statement(statement, &marks, level);
        self.spare_marks.push(marks);
        printed
    }

    fn print_statement(&mut self, statement: NodeId, marks: &Marks, level: usize) -> Printed {
        let (start, end) = self.span(statement);
        if marks.dirty {
            self.out.push_str(&self.text[start..end]);
            return Printed::Source;
        }
        let text = self.text;
        let mut scanner = self.scanner(start);
        let mut prev: Option<Prev<'a>> = None;
        let mut depth = 0usize;
        let (mut next_block, mut next_atomic) = (0, 0);
        loop {
            let token = scanner.scan();
            if token.kind == TokenType::Eof || token.offset >= end {
                break;
            }
            let mut token_end = token.end().min(text.len());
            while marks
                .atomic
                .get(next_atomic)
                .is_some_and(|r| r.end <= token.offset)
            {
                next_atomic += 1;
            }
            if let Some(r) = marks.atomic.get(next_atomic)
                && r.start == token.offset
            {
                token_end = r.end;
                scanner.go_back_to(token_end);
            }
            let ws = prev.map_or("", |p| &text[p.end..token.offset]);
            let newline = ws.contains('\n');
            let block = marks.blocks.get(next_block).copied().filter(|&b| {
                token.kind == TokenType::CurlyL && self.ast.offset(b) as usize == token.offset
            });
            if let Some(block) = block {
                next_block += 1;
                if prev.is_some() {
                    self.separate(if self.options.brace_style_expand {
                        Sep::Line(level)
                    } else {
                        Sep::Space
                    });
                }
                self.block(block, level);
                let block_end = self.span(block).1;
                scanner.go_back_to(block_end);
                prev = Some(Prev {
                    text: "}",
                    end: block_end,
                    line_comment: false,
                    block_end: true,
                    colon: false,
                    selector_separator: false,
                    combinator: false,
                });
                continue;
            }

            let cur = &text[token.offset..token_end];
            let is_comment = matches!(
                token.kind,
                TokenType::Comment | TokenType::SingleLineComment
            );
            let colon = token.kind == TokenType::Colon
                && (marks.colons.binary_search(&token.offset).is_ok()
                    || contains(&marks.colon_ranges, token.offset));
            let selector_separator = token.kind == TokenType::Comma
                && contains(&marks.selector_separators, token.offset);
            let combinator = matches!(cur, ">" | "+" | "~")
                && marks.combinators.binary_search(&token.offset).is_ok();
            let closing = matches!(token.kind, TokenType::ParenthesisR | TokenType::BracketR);
            let in_selector = contains(&marks.selectors, token.offset);
            let keep_newline =
                newline && !in_selector && (depth > 0 || contains(&marks.values, token.offset));
            let continuation = if closing {
                level + depth.saturating_sub(1)
            } else {
                level + depth.max(1)
            };
            let line = |keep: bool, otherwise: Sep| {
                if keep {
                    Sep::Line(continuation)
                } else {
                    otherwise
                }
            };

            if let Some(p) = prev {
                let sep = if p.line_comment {
                    Sep::Line(continuation)
                } else if p.block_end {
                    match cur {
                        ";" => Sep::None,
                        _ if self.options.brace_style_expand => Sep::Line(level),
                        _ => Sep::Space,
                    }
                } else if matches!(token.kind, TokenType::SemiColon | TokenType::Comma) {
                    Sep::None
                } else if closing {
                    line(keep_newline, Sep::None)
                } else if is_comment {
                    line(
                        keep_newline,
                        if ws.is_empty() { Sep::None } else { Sep::Space },
                    )
                } else if matches!(p.text, "(" | "[") {
                    line(keep_newline, Sep::None)
                } else if p.selector_separator {
                    if self.options.selector_separator_newline {
                        Sep::Line(level)
                    } else {
                        Sep::Space
                    }
                } else if p.text == "," || p.text == ";" || p.colon {
                    line(keep_newline, Sep::Space)
                } else if colon {
                    Sep::None
                } else if cur.starts_with('!') && !in_selector {
                    Sep::Space
                } else if p.text == "!" {
                    Sep::None
                } else if p.combinator || combinator {
                    if self.options.space_around_combinator {
                        Sep::Space
                    } else {
                        Sep::None
                    }
                } else if ws.is_empty() {
                    Sep::None
                } else {
                    line(keep_newline, Sep::Space)
                };
                self.separate(sep);
            }
            self.out.push_str(cur);
            match token.kind {
                TokenType::ParenthesisL | TokenType::BracketL => depth += 1,
                TokenType::ParenthesisR | TokenType::BracketR => depth = depth.saturating_sub(1),
                _ => {}
            }
            prev = Some(Prev {
                text: cur,
                end: token_end,
                line_comment: token.kind == TokenType::SingleLineComment,
                block_end: false,
                colon,
                selector_separator,
                combinator,
            });
        }
        if depth == 0 {
            Printed::Closed
        } else {
            Printed::Open
        }
    }

    fn separate(&mut self, sep: Sep) {
        match sep {
            Sep::None => {}
            Sep::Space => self.out.push(' '),
            Sep::Line(indent) => self.newline(indent, 0),
        }
    }
}
