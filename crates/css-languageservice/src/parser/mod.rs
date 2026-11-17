//! Port of `cssParser.ts`, `scssParser.ts` and `lessParser.ts`. One `Parser`
//! has the three dialects; a method that a subclass overrides upstream matches
//! on `dialect`.

mod css;
pub mod errors;
mod less;
pub mod nodes;
pub mod scanner;
mod scss;
pub mod symbol_scope;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use nodes::{Ast, Kind, Level, Marker, NodeId, NodeType, Rule};
use scanner::{Scanner, Token, TokenType};

/// Deepest nesting that the parser reads: the recursive parse functions
/// (blocks, expressions, selectors, conditions), plus the links of a binary
/// expression or `@else if` chain. Deeper content is a parse error, and the
/// error recovery skips it without recursion. Recursive walks of the tree
/// then have a bounded depth. Not in upstream.
pub const MAX_DEPTH: usize = 1000;

/// The CSS dialect. Upstream uses subclasses of the scanner and the parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Css,
    Scss,
    Less,
}

/// A parsed style sheet.
#[derive(Clone, Debug)]
pub struct Stylesheet {
    pub ast: Ast,
    pub root: NodeId,
}

pub fn parse_stylesheet(dialect: Dialect, text: &str) -> Stylesheet {
    let mut parser = Parser::new(dialect, text);
    let root = parser.parse_stylesheet_root();
    Stylesheet {
        ast: parser.ast,
        root,
    }
}

/// Parses each of `regions`, sorted byte ranges of `text`, alone into one style
/// sheet: an open block or an error recovery ends at the region end. Not in
/// upstream, which parses the regions of an HTML document as one text.
pub fn parse_stylesheet_regions(
    dialect: Dialect,
    text: &str,
    regions: &[std::ops::Range<usize>],
) -> Stylesheet {
    let mut parser = Parser::new(dialect, text);
    let mut root = None;
    for region in regions {
        parser.scanner.set_source(&text[..region.end]);
        parser.scanner.go_back_to(region.start);
        parser.group_ends = None;
        let token = parser.scanner.scan();
        parser.set_token(token);
        let node = *root.get_or_insert_with(|| parser.create(Kind::Stylesheet));
        parser.parse_stylesheet_body(node);
    }
    let root = match root {
        Some(node) => {
            parser.mark_too_deep(node);
            parser.finish(node)
        }
        None => parser.parse_stylesheet_root(),
    };
    Stylesheet {
        ast: parser.ast,
        root,
    }
}

/// `IMark`.
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    prev: Option<Token>,
    curr: Token,
    curr_id: u32,
    pos: usize,
    nodes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Retry {
    Selector,
    Function,
    ModuleMember,
}

/// The closing bracket of a group and the scanner position after it.
type GroupEnd = (Token, usize);

pub struct Parser<'a> {
    pub(crate) dialect: Dialect,
    pub(crate) scanner: Scanner<'a>,
    pub(crate) token: Token,
    /// Upstream compares token objects by identity; each scan has a new id.
    token_id: u32,
    next_id: u32,
    pub(crate) prev_token: Option<Token>,
    last_error_token: Option<u32>,
    /// Nesting inside `attr()`, where `type(<syntax>)` is a valid argument.
    pub(crate) attr_depth: u32,
    /// Calls of `nested` and chain links in progress.
    pub(crate) depth: usize,
    /// The tokens where `nested` stopped at `MAX_DEPTH`.
    too_deep: HashMap<usize, Token>,
    /// The end of each closed group, by the offset of its opening bracket.
    /// Built at the first skip.
    group_ends: Option<HashMap<usize, GroupEnd>>,
    /// The parse functions that failed at an offset with an `attr_depth`. The
    /// parser tries some parse functions again at the same offset; in nested
    /// input this takes time exponential in the depth (also upstream). Not in
    /// upstream.
    failed: HashSet<(Retry, usize, u32)>,
    pub ast: Ast,
}

impl<'a> Parser<'a> {
    /// `new Parser()` followed by `internalParse` up to the first token.
    pub fn new(dialect: Dialect, text: &'a str) -> Self {
        let mut scanner = Scanner::new(dialect);
        scanner.set_source(text);
        let token = scanner.scan();
        let ast = Ast::new(text);
        Parser {
            dialect,
            scanner,
            token,
            token_id: 0,
            next_id: 1,
            prev_token: None,
            last_error_token: None,
            attr_depth: 0,
            depth: 0,
            too_deep: HashMap::new(),
            group_ends: None,
            failed: HashSet::new(),
            ast,
        }
    }

    fn set_token(&mut self, token: Token) {
        self.token = token;
        self.token_id = self.next_id;
        self.next_id += 1;
    }

    /// The text of the current token.
    pub(crate) fn text(&self) -> Cow<'a, str> {
        self.scanner.text(&self.token)
    }

    pub(crate) fn tok(&self) -> TokenType {
        self.token.kind
    }

    pub(crate) fn peek_ident(&self, text: &str) -> bool {
        self.token.kind == TokenType::Ident && lower_eq(&self.text(), text)
    }

    pub(crate) fn peek_keyword(&self, text: &str) -> bool {
        self.token.kind == TokenType::AtKeyword && lower_eq(&self.text(), text)
    }

    pub(crate) fn peek_delim(&self, text: &str) -> bool {
        self.token.kind == TokenType::Delim && self.text() == text
    }

    pub(crate) fn peek(&self, kind: TokenType) -> bool {
        self.token.kind == kind
    }

    pub(crate) fn peek_one(&self, kinds: &[TokenType]) -> bool {
        kinds.contains(&self.token.kind)
    }

    pub(crate) fn has_whitespace(&self) -> bool {
        self.prev_token
            .as_ref()
            .is_some_and(|p| p.end() != self.token.offset)
    }

    pub(crate) fn consume_token(&mut self) {
        self.prev_token = Some(self.token);
        let next = self.scanner.scan();
        self.set_token(next);
    }

    /// Runs a parse function that can recurse, one level deeper. At
    /// `MAX_DEPTH`, adds an error, skips to the closing bracket of the
    /// enclosing group and returns `None`; the caller then recovers as from any
    /// missing part. Without the skip, each enclosing level would parse the
    /// deep rest again.
    pub(crate) fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        if self.depth == MAX_DEPTH {
            self.skip_too_deep();
            return None;
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    /// Skips to the closing bracket of the enclosing group. Without one, the
    /// rest of the text is in the unclosed group: the text then ends at the
    /// current token. Restores make the parser come back to skipped text, so
    /// a skip jumps over each inner group through `group_ends`.
    #[cold]
    #[inline(never)]
    fn skip_too_deep(&mut self) {
        let first = self.token;
        self.too_deep.entry(first.offset).or_insert(first);
        let ends = self.group_ends.take().unwrap_or_else(|| {
            let mut scanner = Scanner::new(self.dialect);
            scanner.set_source(self.scanner.source());
            group_ends(scanner)
        });
        let closed = loop {
            match self.token.kind {
                TokenType::CurlyL
                | TokenType::ParenthesisL
                | TokenType::BracketL
                | TokenType::InterpolationFunction => {
                    let Some(&(end, pos)) = ends.get(&self.token.offset) else {
                        break false;
                    };
                    self.scanner.go_back_to(pos);
                    self.prev_token = Some(end);
                    let next = self.scanner.scan();
                    self.set_token(next);
                }
                TokenType::CurlyR | TokenType::ParenthesisR | TokenType::BracketR => break true,
                TokenType::Eof => break false,
                _ => self.consume_token(),
            }
        };
        self.group_ends = Some(ends);
        if !closed {
            self.scanner.end_at(first.offset);
            let eof = self.scanner.scan();
            self.set_token(eof);
        }
    }

    /// Adds an error for each token where `nested` stopped at `MAX_DEPTH`.
    pub(crate) fn mark_too_deep(&mut self, root: NodeId) {
        let mut skips: Vec<_> = self.too_deep.drain().map(|(_, t)| t).collect();
        skips.sort_by_key(|t| t.offset);
        for token in skips {
            let marker = Marker {
                node: root,
                rule: &errors::NESTING_TOO_DEEP,
                level: Level::Error,
                message: errors::NESTING_TOO_DEEP.message.to_owned(),
                offset: token.offset as isize,
                length: token.len as isize,
            };
            self.ast.add_issue(root, marker);
        }
    }

    pub(crate) fn accept_unicode_range(&mut self) -> bool {
        if let Some(token) = self.scanner.try_scan_unicode() {
            self.prev_token = Some(token);
            let next = self.scanner.scan();
            self.set_token(next);
            return true;
        }
        false
    }

    pub(crate) fn mark(&self) -> Mark {
        Mark {
            prev: self.prev_token,
            curr: self.token,
            curr_id: self.token_id,
            pos: self.scanner.pos(),
            nodes: self.ast.len(),
        }
    }

    pub(crate) fn restore_at_mark(&mut self, mark: Mark) {
        self.prev_token = mark.prev;
        self.token = mark.curr;
        self.token_id = mark.curr_id;
        self.scanner.go_back_to(mark.pos);
    }

    /// `restoreAtMark`, and drops the nodes created since the mark. The caller
    /// must not use them.
    pub(crate) fn restore_and_discard(&mut self, mark: Mark) {
        let nodes = mark.nodes;
        self.restore_at_mark(mark);
        self.ast.discard_from(nodes);
    }

    /// Drops `node` and the nodes created after it. For a node that a parse
    /// function created and does not return.
    pub(crate) fn discard(&mut self, node: NodeId) -> Option<NodeId> {
        self.ast.discard_from(node.index());
        None
    }

    /// `f`, or `None` without a call when `f` failed at this position before.
    /// For parse functions whose result depends only on the position. Only
    /// failures that scanned more than two tokens are kept: most calls fail on
    /// the first token or two, and the set then stays empty.
    pub(crate) fn remember_failure(
        &mut self,
        retry: Retry,
        f: impl FnOnce(&mut Self) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let key = (retry, self.token.offset, self.attr_depth);
        if !self.failed.is_empty() && self.failed.contains(&key) {
            return None;
        }
        let scans = self.next_id;
        let node = f(self);
        if node.is_none() && self.next_id - scans > 2 {
            self.failed.insert(key);
        }
        node
    }

    /// `try`: restores the position when `f` returns nothing.
    pub(crate) fn try_parse(
        &mut self,
        f: impl FnOnce(&mut Self) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let pos = self.mark();
        let node = f(self);
        if node.is_none() {
            self.restore_and_discard(pos);
        }
        node
    }

    pub(crate) fn accept_one_keyword(&mut self, keywords: &[&str]) -> bool {
        if self.token.kind == TokenType::AtKeyword
            && keywords.iter().any(|k| lower_eq(&self.text(), k))
        {
            self.consume_token();
            return true;
        }
        false
    }

    pub(crate) fn accept(&mut self, kind: TokenType) -> bool {
        if self.token.kind == kind {
            self.consume_token();
            return true;
        }
        false
    }

    pub(crate) fn accept_ident(&mut self, text: &str) -> bool {
        if self.peek_ident(text) {
            self.consume_token();
            return true;
        }
        false
    }

    pub(crate) fn accept_keyword(&mut self, text: &str) -> bool {
        if self.peek_keyword(text) {
            self.consume_token();
            return true;
        }
        false
    }

    pub(crate) fn accept_delim(&mut self, text: &str) -> bool {
        if self.peek_delim(text) {
            self.consume_token();
            return true;
        }
        false
    }

    pub(crate) fn accept_unquoted_string(&mut self) -> bool {
        let pos = self.scanner.pos();
        self.scanner.go_back_to(self.token.offset);
        if let Some(unquoted) = self.scanner.scan_unquoted_string() {
            self.set_token(unquoted);
            self.consume_token();
            return true;
        }
        self.scanner.go_back_to(pos);
        false
    }

    pub(crate) fn resync(&mut self, tokens: &[TokenType], stop_tokens: &[TokenType]) -> bool {
        loop {
            if tokens.contains(&self.token.kind) {
                self.consume_token();
                return true;
            } else if stop_tokens.contains(&self.token.kind) {
                return true;
            } else {
                if self.token.kind == TokenType::Eof {
                    return false;
                }
                let next = self.scanner.scan();
                self.set_token(next);
            }
        }
    }

    /// `createNode(nodeType)`: a plain node of the type.
    pub(crate) fn create_node(&mut self, ty: NodeType) -> NodeId {
        let id = self.create(Kind::Node);
        self.ast.set_type(id, ty);
        id
    }

    /// `create(ctor)`.
    pub(crate) fn create(&mut self, kind: Kind) -> NodeId {
        self.ast
            .alloc(kind, self.token.offset as isize, self.token.len as isize)
    }

    pub(crate) fn finish(&mut self, node: NodeId) -> NodeId {
        self.finish_with(node, None, None)
    }

    pub(crate) fn finish_error(&mut self, node: NodeId, error: &'static Rule) -> NodeId {
        self.finish_with(node, Some(error), None)
    }

    /// `finish(node, error, resyncTokens, resyncStopTokens)` with at least one
    /// of the two lists given.
    pub(crate) fn finish_resync(
        &mut self,
        node: NodeId,
        error: &'static Rule,
        resync_tokens: &[TokenType],
        resync_stop_tokens: &[TokenType],
    ) -> NodeId {
        self.finish_with(node, Some(error), Some((resync_tokens, resync_stop_tokens)))
    }

    fn finish_with(
        &mut self,
        node: NodeId,
        error: Option<&'static Rule>,
        resync: Option<(&[TokenType], &[TokenType])>,
    ) -> NodeId {
        // Node list offsets must not change: rule sets use them for selection
        // ranges.
        if self.ast.kind(node) != Kind::Nodelist {
            if let Some(error) = error {
                self.mark_error_with(node, error, resync);
            }
            if let Some(prev) = self.prev_token {
                let n = self.ast.node_mut(node);
                n.set_length((prev.end() as isize - n.offset()).max(0));
            }
        }
        node
    }

    pub(crate) fn mark_error(&mut self, node: NodeId, error: &'static Rule) {
        self.mark_error_with(node, error, None);
    }

    /// `markError`. Upstream resyncs when one of the two lists is given, also
    /// when it is empty.
    pub(crate) fn mark_error_with(
        &mut self,
        node: NodeId,
        error: &'static Rule,
        resync: Option<(&[TokenType], &[TokenType])>,
    ) {
        if self.last_error_token != Some(self.token_id) {
            let marker = Marker {
                node,
                rule: error,
                level: Level::Error,
                message: error.message.to_owned(),
                offset: self.token.offset as isize,
                length: self.token.len as isize,
            };
            self.ast.add_issue(node, marker);
            self.last_error_token = Some(self.token_id);
        }
        if let Some((tokens, stop_tokens)) = resync {
            self.resync(tokens, stop_tokens);
        }
    }

    /// `internalParse` with a parse function given by its upstream name. For
    /// tests. `None` if there is no such function.
    pub fn internal_parse(&mut self, name: &str) -> Option<Option<NodeId>> {
        self.parse_by_name(name)
    }

    pub fn token_kind(&self) -> TokenType {
        self.token.kind
    }

    pub fn token_offset(&self) -> usize {
        self.token.offset
    }
}

/// `text === token.text.toLowerCase()` with the length check of upstream.
/// `text` is lowercase ASCII.
fn lower_eq(token: &str, text: &str) -> bool {
    if token.is_ascii() {
        return token.len() == text.len() && token.eq_ignore_ascii_case(text);
    }
    token.encode_utf16().count() == text.len() && token.to_lowercase() == text
}

fn group_ends(mut scanner: Scanner) -> HashMap<usize, GroupEnd> {
    let mut ends = HashMap::new();
    let mut open = Vec::new();
    loop {
        let token = scanner.scan();
        match token.kind {
            TokenType::CurlyL
            | TokenType::ParenthesisL
            | TokenType::BracketL
            | TokenType::InterpolationFunction => open.push(token.offset),
            TokenType::CurlyR | TokenType::ParenthesisR | TokenType::BracketR => {
                if let Some(start) = open.pop() {
                    ends.insert(start, (token, scanner.pos()));
                }
            }
            TokenType::Eof => return ends,
            _ => {}
        }
    }
}
