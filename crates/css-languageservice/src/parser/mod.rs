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

use nodes::{Ast, Kind, Level, Marker, NodeId, NodeType, Rule};
use scanner::{Scanner, Token, TokenType};

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

/// `IMark`.
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    prev: Option<Token>,
    curr: Token,
    curr_id: u32,
    pos: usize,
    nodes: usize,
}

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
