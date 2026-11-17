use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Index;

use jsonc::{MAX_DEPTH, NodeType, ScanError, Scanner, Segment, SyntaxKind};
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Range};
use serde_json::{Map, Value};

use crate::{ErrorCode, TextDocument};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

#[derive(Clone, Debug, PartialEq)]
pub enum AstKind {
    /// Children are the properties.
    Object,
    /// Children are the items.
    Array,
    /// Children are the key and, if present, the value.
    Property {
        colon_offset: Option<u32>,
    },
    /// Read the value with [`JsonDocument::string`].
    String(StringSpan),
    /// `value` is NaN if the number has a syntax error.
    Number {
        value: f64,
        is_integer: bool,
    },
    Boolean(bool),
    Null,
}

/// The location of a string value in [`JsonDocument`]'s string buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StringSpan {
    start: u32,
    len: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AstNode {
    pub kind: AstKind,
    offset: u32,
    length: u32,
    pub parent: Option<NodeId>,
    /// Range in [`JsonDocument::children`]. Set after the parse.
    children_start: u32,
    children_len: u32,
}

impl AstNode {
    pub fn node_type(&self) -> NodeType {
        match self.kind {
            AstKind::Object => NodeType::Object,
            AstKind::Array => NodeType::Array,
            AstKind::Property { .. } => NodeType::Property,
            AstKind::String(_) => NodeType::String,
            AstKind::Number { .. } => NodeType::Number,
            AstKind::Boolean(_) => NodeType::Boolean,
            AstKind::Null => NodeType::Null,
        }
    }

    pub fn offset(&self) -> usize {
        self.offset as usize
    }

    pub fn length(&self) -> usize {
        self.length as usize
    }

    pub fn end(&self) -> usize {
        self.offset() + self.length()
    }

    fn set_end(&mut self, end: usize) {
        self.length = (end - self.offset()) as u32;
    }

    pub fn contains(&self, offset: usize, include_right_bound: bool) -> bool {
        (offset >= self.offset() && offset < self.end())
            || (include_right_bound && offset == self.end())
    }
}

#[derive(Clone, Debug, Default)]
pub struct JsonDocument {
    nodes: Vec<AstNode>,
    /// The child lists of all nodes, one after the other.
    children: Vec<NodeId>,
    /// The values of all string nodes, one after the other.
    strings: String,
    pub syntax_errors: Vec<Diagnostic>,
    pub comments: Vec<Range>,
}

impl Index<NodeId> for JsonDocument {
    type Output = AstNode;

    fn index(&self, id: NodeId) -> &AstNode {
        &self.nodes[id.0 as usize]
    }
}

impl JsonDocument {
    /// The properties of an object, the items of an array, or the key and the value of a
    /// property.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        let node = &self[id];
        let start = node.children_start as usize;
        &self.children[start..start + node.children_len as usize]
    }

    /// Fills the child lists from the parent links. Nodes are in pre-order, so the children
    /// of each node are in document order.
    fn index_children(&mut self) {
        for i in 0..self.nodes.len() {
            if let Some(parent) = self.nodes[i].parent {
                self.nodes[parent.0 as usize].children_len += 1;
            }
        }
        let mut start = 0;
        for node in &mut self.nodes {
            node.children_start = start;
            start += node.children_len;
        }
        let mut filled = vec![0u32; self.nodes.len()];
        self.children = vec![NodeId(0); start as usize];
        for i in 0..self.nodes.len() {
            if let Some(parent) = self.nodes[i].parent {
                let p = parent.0 as usize;
                let slot = self.nodes[p].children_start + filled[p];
                self.children[slot as usize] = NodeId(i as u32);
                filled[p] += 1;
            }
        }
    }

    pub fn root(&self) -> Option<NodeId> {
        (!self.nodes.is_empty()).then_some(NodeId(0))
    }

    /// The value of a string node.
    pub fn string(&self, id: NodeId) -> Option<&str> {
        match self[id].kind {
            AstKind::String(span) => Some(self.span(span)),
            _ => None,
        }
    }

    fn span(&self, span: StringSpan) -> &str {
        &self.strings[span.start as usize..(span.start + span.len) as usize]
    }

    /// The key of a property node.
    pub fn key(&self, property: NodeId) -> &str {
        self.string(self.children(property)[0]).unwrap_or_default()
    }

    /// The value of a property node.
    pub fn value(&self, property: NodeId) -> Option<NodeId> {
        self.children(property).get(1).copied()
    }

    pub fn get_node_from_offset(&self, offset: usize, include_right_bound: bool) -> Option<NodeId> {
        self.root()
            .and_then(|root| self.find_node_at_offset(root, offset, include_right_bound))
    }

    fn find_node_at_offset(
        &self,
        id: NodeId,
        offset: usize,
        include_right_bound: bool,
    ) -> Option<NodeId> {
        let node = &self[id];
        if !node.contains(offset, include_right_bound) {
            return None;
        }
        self.children(id)
            .iter()
            .take_while(|&&child| self[child].offset() <= offset)
            .find_map(|&child| self.find_node_at_offset(child, offset, include_right_bound))
            .or(Some(id))
    }

    pub fn node_path(&self, id: NodeId) -> Vec<Segment> {
        let mut path = Vec::new();
        let mut node = id;
        while let Some(parent) = self[node].parent {
            match self[parent].kind {
                AstKind::Property { .. } => path.push(Segment::from(self.key(parent))),
                AstKind::Array => {
                    if let Some(index) = self.children(parent).iter().position(|&c| c == node) {
                        path.push(Segment::Index(index));
                    }
                }
                _ => {}
            }
            node = parent;
        }
        path.reverse();
        path
    }

    pub fn node_value(&self, id: NodeId) -> Value {
        let node = &self[id];
        match &node.kind {
            AstKind::Array => self
                .children(id)
                .iter()
                .map(|&c| self.node_value(c))
                .collect(),
            AstKind::Object => {
                let mut object = Map::new();
                for &property in self.children(id) {
                    if let Some(value) = self.value(property) {
                        object.insert(self.key(property).to_owned(), self.node_value(value));
                    }
                }
                Value::Object(object)
            }
            AstKind::Property { .. } | AstKind::Null => Value::Null,
            AstKind::String(span) => Value::String(self.span(*span).to_owned()),
            AstKind::Number { value, .. } => jsonc::number_to_json(*value),
            AstKind::Boolean(b) => Value::Bool(*b),
        }
    }

    /// Visits the nodes depth-first. Return `false` from the visitor to stop.
    pub fn visit(&self, visitor: impl FnMut(NodeId) -> bool) {
        if let Some(root) = self.root() {
            self.visit_from(root, visitor);
        }
    }

    /// Visits `start` and its descendants depth-first. Return `false` from the visitor to stop.
    pub fn visit_from(&self, start: NodeId, mut visitor: impl FnMut(NodeId) -> bool) {
        fn visit_node(
            doc: &JsonDocument,
            id: NodeId,
            visitor: &mut impl FnMut(NodeId) -> bool,
        ) -> bool {
            visitor(id)
                && doc
                    .children(id)
                    .iter()
                    .all(|&c| visit_node(doc, c, visitor))
        }
        visit_node(self, start, &mut visitor);
    }
}

/// Panics if the document is 4 GiB or larger: offsets are `u32`.
pub fn parse(document: &TextDocument, collect_comments: bool) -> JsonDocument {
    assert!(
        u32::try_from(document.text().len()).is_ok(),
        "document too large"
    );
    let mut parser = Parser {
        document,
        text: document.text(),
        scanner: Scanner::new(document.text(), false),
        collect_comments,
        last_problem_offset: None,
        doc: JsonDocument::default(),
        key_maps: Vec::new(),
        depth: 0,
    };
    parser.run();
    parser.doc.index_children();
    parser.doc
}

type KeysSeen<'a> = HashMap<Cow<'a, str>, Option<NodeId>, foldhash::fast::RandomState>;

struct Parser<'a> {
    document: &'a TextDocument,
    text: &'a str,
    scanner: Scanner<'a>,
    collect_comments: bool,
    last_problem_offset: Option<usize>,
    doc: JsonDocument,
    /// Unused duplicate-key maps, to reuse their allocations.
    key_maps: Vec<KeysSeen<'a>>,
    depth: usize,
}

const OBJECT_RECOVERY: &[SyntaxKind] = &[SyntaxKind::CloseBraceToken, SyntaxKind::CommaToken];
const ARRAY_RECOVERY: &[SyntaxKind] = &[SyntaxKind::CloseBracketToken, SyntaxKind::CommaToken];

impl<'a> Parser<'a> {
    fn token(&self) -> SyntaxKind {
        self.scanner.token()
    }

    fn token_end(&self) -> usize {
        self.scanner.token_offset() + self.scanner.token_length()
    }

    fn node_mut(&mut self, id: NodeId) -> &mut AstNode {
        &mut self.doc.nodes[id.0 as usize]
    }

    fn push(&mut self, kind: AstKind, parent: Option<NodeId>) -> NodeId {
        let id = NodeId(self.doc.nodes.len() as u32);
        self.doc.nodes.push(AstNode {
            kind,
            offset: self.scanner.token_offset() as u32,
            length: 0,
            parent,
            children_start: 0,
            children_len: 0,
        });
        id
    }

    fn scan_next(&mut self) -> SyntaxKind {
        loop {
            let token = self.scanner.scan();
            self.check_scan_error();
            match token {
                SyntaxKind::LineCommentTrivia | SyntaxKind::BlockCommentTrivia => {
                    if self.collect_comments {
                        let start = self.scanner.token_offset();
                        let range = self.document.range(start, self.token_end());
                        self.doc.comments.push(range);
                    }
                }
                SyntaxKind::Trivia | SyntaxKind::LineBreakTrivia => {}
                _ => return token,
            }
        }
    }

    fn error_at_range(
        &mut self,
        message: &str,
        code: ErrorCode,
        start: usize,
        end: usize,
        severity: DiagnosticSeverity,
    ) {
        if self.doc.syntax_errors.is_empty() || self.last_problem_offset != Some(start) {
            let range = self.document.range(start, end);
            self.doc.syntax_errors.push(Diagnostic {
                range,
                severity: Some(severity),
                code: Some(NumberOrString::Number(code.0 as i32)),
                source: Some(self.document.language_id().to_owned()),
                message: message.to_owned(),
                ..Default::default()
            });
            self.last_problem_offset = Some(start);
        }
    }

    /// Reports an error at the current token. A node given is completed at the current token.
    fn error(
        &mut self,
        message: &str,
        code: ErrorCode,
        node: Option<NodeId>,
        skip_until_after: &[SyntaxKind],
        skip_until: &[SyntaxKind],
    ) -> Option<NodeId> {
        let mut start = self.scanner.token_offset();
        let mut end = self.token_end();
        if start == end && start > 0 {
            // At the end of the text: mark the last character that is not whitespace.
            let mut chars = self.text[..start].char_indices().rev();
            let mut last = chars.next();
            while let Some((i, c)) = last
                && i > 0
                && is_js_whitespace(c)
            {
                last = chars.next();
            }
            if let Some((i, c)) = last {
                start = i;
                end = i + c.len_utf8();
            }
        }
        self.error_at_range(message, code, start, end, DiagnosticSeverity::ERROR);

        if let Some(node) = node {
            self.finalize(node, false);
        }
        if !skip_until_after.is_empty() || !skip_until.is_empty() {
            let mut token = self.token();
            while token != SyntaxKind::EOF {
                if skip_until_after.contains(&token) {
                    self.scan_next();
                    break;
                } else if skip_until.contains(&token) {
                    break;
                }
                token = self.scan_next();
            }
        }
        node
    }

    fn simple_error(&mut self, message: &str, code: ErrorCode) {
        self.error(message, code, None, &[], &[]);
    }

    fn check_scan_error(&mut self) {
        let (message, code) = match self.scanner.token_error() {
            ScanError::None => return,
            ScanError::InvalidUnicode => (
                "Invalid unicode sequence in string.",
                ErrorCode::INVALID_UNICODE,
            ),
            ScanError::InvalidEscapeCharacter => (
                "Invalid escape character in string.",
                ErrorCode::INVALID_ESCAPE_CHARACTER,
            ),
            ScanError::UnexpectedEndOfNumber => (
                "Unexpected end of number.",
                ErrorCode::UNEXPECTED_END_OF_NUMBER,
            ),
            ScanError::UnexpectedEndOfComment => (
                "Unexpected end of comment.",
                ErrorCode::UNEXPECTED_END_OF_COMMENT,
            ),
            ScanError::UnexpectedEndOfString => (
                "Unexpected end of string.",
                ErrorCode::UNEXPECTED_END_OF_STRING,
            ),
            ScanError::InvalidCharacter => (
                "Invalid characters in string. Control characters must be escaped.",
                ErrorCode::INVALID_CHARACTER,
            ),
        };
        self.simple_error(message, code);
    }

    fn finalize(&mut self, node: NodeId, scan_next: bool) -> NodeId {
        let end = self.token_end();
        self.node_mut(node).set_end(end);
        if scan_next {
            self.scan_next();
        }
        node
    }

    fn parse_array(&mut self, parent: Option<NodeId>) -> Option<NodeId> {
        if self.token() != SyntaxKind::OpenBracketToken {
            return None;
        }
        let node = self.push(AstKind::Array, parent);
        self.scan_next();

        let mut needs_comma = false;
        while !matches!(
            self.token(),
            SyntaxKind::CloseBracketToken | SyntaxKind::EOF
        ) {
            if self.token() == SyntaxKind::CommaToken {
                if !needs_comma {
                    self.simple_error("Value expected", ErrorCode::VALUE_EXPECTED);
                }
                let comma_offset = self.scanner.token_offset();
                self.scan_next();
                if self.token() == SyntaxKind::CloseBracketToken {
                    if needs_comma {
                        self.trailing_comma(comma_offset);
                    }
                    continue;
                }
            } else if needs_comma {
                self.simple_error("Expected comma", ErrorCode::COMMA_EXPECTED);
            }
            if self.parse_value(Some(node)).is_none() {
                self.error(
                    "Value expected",
                    ErrorCode::VALUE_EXPECTED,
                    None,
                    &[],
                    ARRAY_RECOVERY,
                );
            }
            needs_comma = true;
        }

        if self.token() != SyntaxKind::CloseBracketToken {
            return self.error(
                "Expected comma or closing bracket",
                ErrorCode::COMMA_OR_CLOSE_BACKET_EXPECTED,
                Some(node),
                &[],
                &[],
            );
        }
        Some(self.finalize(node, true))
    }

    fn trailing_comma(&mut self, comma_offset: usize) {
        self.error_at_range(
            "Trailing comma",
            ErrorCode::TRAILING_COMMA,
            comma_offset,
            comma_offset + 1,
            DiagnosticSeverity::ERROR,
        );
    }

    fn parse_property(&mut self, parent: NodeId, keys_seen: &mut KeysSeen<'a>) -> Option<NodeId> {
        let node = self.push(AstKind::Property { colon_offset: None }, Some(parent));
        let key = match self.parse_string(Some(node)) {
            Some(key) => key,
            None if self.token() == SyntaxKind::Unknown => {
                self.simple_error(
                    "Property keys must be doublequoted",
                    ErrorCode::PROPERTY_KEYS_MUST_BE_DOUBLEQUOTED,
                );
                let key = self.push_string(Some(node));
                self.node_mut(key).length = self.scanner.token_length() as u32;
                self.scan_next();
                key
            }
            None => {
                self.doc.nodes.pop();
                return None;
            }
        };

        // Files that forbid comments use the key "//" for comments, so it can repeat.
        let key_value = self.key_value(key);
        if key_value != "//" {
            let (key_start, key_end) = (self.doc[key].offset(), self.doc[key].end());
            match keys_seen.entry(key_value) {
                Entry::Occupied(mut seen) => {
                    self.duplicate_key(key_start, key_end);
                    if let Some(first_key) = seen.get_mut().take() {
                        let (start, end) =
                            (self.doc[first_key].offset(), self.doc[first_key].end());
                        self.duplicate_key(start, end);
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert(Some(key));
                }
            }
        }

        if self.token() == SyntaxKind::ColonToken {
            let colon_offset = self.scanner.token_offset();
            self.node_mut(node).kind = AstKind::Property {
                colon_offset: Some(colon_offset as u32),
            };
            self.scan_next();
        } else {
            self.simple_error("Colon expected", ErrorCode::COLON_EXPECTED);
            let key_end_line = self.document.position_at(self.doc[key].end()).line;
            let token_line = self.document.position_at(self.scanner.token_offset()).line;
            if self.token() == SyntaxKind::StringLiteral && key_end_line < token_line {
                self.node_mut(node).length = self.doc[key].length;
                return Some(node);
            }
        }
        let Some(value) = self.parse_value(Some(node)) else {
            return self.error(
                "Value expected",
                ErrorCode::VALUE_EXPECTED,
                Some(node),
                &[],
                OBJECT_RECOVERY,
            );
        };
        let value_end = self.doc[value].end();
        self.node_mut(node).set_end(value_end);
        Some(node)
    }

    /// Borrows the key from the text if it has no escapes.
    fn key_value(&self, key: NodeId) -> Cow<'a, str> {
        let value = self.doc.string(key).unwrap_or_default();
        let node = &self.doc[key];
        match self
            .text
            .get(node.offset() + 1..node.end().saturating_sub(1))
        {
            Some(raw) if raw == value => Cow::Borrowed(raw),
            _ => Cow::Owned(value.to_owned()),
        }
    }

    fn duplicate_key(&mut self, start: usize, end: usize) {
        self.error_at_range(
            "Duplicate object key",
            ErrorCode::DUPLICATE_KEY,
            start,
            end,
            DiagnosticSeverity::WARNING,
        );
    }

    fn parse_object(&mut self, parent: Option<NodeId>) -> Option<NodeId> {
        if self.token() != SyntaxKind::OpenBraceToken {
            return None;
        }
        let node = self.push(AstKind::Object, parent);
        let mut keys_seen = self.key_maps.pop().unwrap_or_default();
        self.scan_next();
        let mut needs_comma = false;

        while !matches!(self.token(), SyntaxKind::CloseBraceToken | SyntaxKind::EOF) {
            if self.token() == SyntaxKind::CommaToken {
                if !needs_comma {
                    self.simple_error("Property expected", ErrorCode::PROPERTY_EXPECTED);
                }
                let comma_offset = self.scanner.token_offset();
                self.scan_next();
                if self.token() == SyntaxKind::CloseBraceToken {
                    if needs_comma {
                        self.trailing_comma(comma_offset);
                    }
                    continue;
                }
            } else if needs_comma {
                self.simple_error("Expected comma", ErrorCode::COMMA_EXPECTED);
            }
            if self.parse_property(node, &mut keys_seen).is_none() {
                self.error(
                    "Property expected",
                    ErrorCode::PROPERTY_EXPECTED,
                    None,
                    &[],
                    OBJECT_RECOVERY,
                );
            }
            needs_comma = true;
        }
        // `clear` takes time in proportion to the capacity, so do not keep large maps.
        if keys_seen.capacity() <= 256 {
            keys_seen.clear();
            self.key_maps.push(keys_seen);
        }

        if self.token() != SyntaxKind::CloseBraceToken {
            return self.error(
                "Expected comma or closing brace",
                ErrorCode::COMMA_OR_CLOSE_BRACE_EXPECTED,
                Some(node),
                &[],
                &[],
            );
        }
        Some(self.finalize(node, true))
    }

    fn parse_string(&mut self, parent: Option<NodeId>) -> Option<NodeId> {
        if self.token() != SyntaxKind::StringLiteral {
            return None;
        }
        let node = self.push_string(parent);
        Some(self.finalize(node, true))
    }

    fn push_string(&mut self, parent: Option<NodeId>) -> NodeId {
        let value = self.scanner.token_value();
        let span = StringSpan {
            start: self.doc.strings.len() as u32,
            len: value.len() as u32,
        };
        self.doc.strings.push_str(value);
        self.push(AstKind::String(span), parent)
    }

    fn parse_number(&mut self, parent: Option<NodeId>) -> Option<NodeId> {
        if self.token() != SyntaxKind::NumericLiteral {
            return None;
        }
        let node = self.push(
            AstKind::Number {
                value: f64::NAN,
                is_integer: true,
            },
            parent,
        );
        if self.scanner.token_error() == ScanError::None {
            let token_value = self.scanner.token_value();
            let Ok(value) = token_value.parse::<f64>() else {
                return self.error(
                    "Invalid number format.",
                    ErrorCode::UNDEFINED,
                    Some(node),
                    &[],
                    &[],
                );
            };
            let is_integer = !token_value.contains('.');
            self.node_mut(node).kind = AstKind::Number { value, is_integer };
        }
        Some(self.finalize(node, true))
    }

    fn parse_literal(&mut self, parent: Option<NodeId>) -> Option<NodeId> {
        let kind = match self.token() {
            SyntaxKind::NullKeyword => AstKind::Null,
            SyntaxKind::TrueKeyword => AstKind::Boolean(true),
            SyntaxKind::FalseKeyword => AstKind::Boolean(false),
            _ => return None,
        };
        let node = self.push(kind, parent);
        Some(self.finalize(node, true))
    }

    /// Reads a value deeper than `MAX_DEPTH` as an empty array or object.
    #[cold]
    #[inline(never)]
    fn skip_too_deep(&mut self, parent: Option<NodeId>) -> NodeId {
        let kind = if self.token() == SyntaxKind::OpenBracketToken {
            AstKind::Array
        } else {
            AstKind::Object
        };
        let (start, end) = (self.scanner.token_offset(), self.token_end());
        self.error_at_range(
            "Nesting too deep",
            ErrorCode::NESTING_TOO_DEEP,
            start,
            end,
            DiagnosticSeverity::ERROR,
        );
        let node = self.push(kind, parent);
        let mut depth = 0;
        loop {
            match self.token() {
                SyntaxKind::OpenBracketToken | SyntaxKind::OpenBraceToken => depth += 1,
                SyntaxKind::CloseBracketToken | SyntaxKind::CloseBraceToken => {
                    depth -= 1;
                    if depth == 0 {
                        return self.finalize(node, true);
                    }
                }
                SyntaxKind::EOF => return self.finalize(node, false),
                _ => {}
            }
            self.scan_next();
        }
    }

    fn parse_value(&mut self, parent: Option<NodeId>) -> Option<NodeId> {
        match self.token() {
            SyntaxKind::OpenBracketToken | SyntaxKind::OpenBraceToken
                if self.depth == MAX_DEPTH =>
            {
                Some(self.skip_too_deep(parent))
            }
            SyntaxKind::OpenBracketToken => {
                self.depth += 1;
                let node = self.parse_array(parent);
                self.depth -= 1;
                node
            }
            SyntaxKind::OpenBraceToken => {
                self.depth += 1;
                let node = self.parse_object(parent);
                self.depth -= 1;
                node
            }
            SyntaxKind::StringLiteral => self.parse_string(parent),
            SyntaxKind::NumericLiteral => self.parse_number(parent),
            _ => self.parse_literal(parent),
        }
    }

    fn run(&mut self) {
        if self.scan_next() == SyntaxKind::EOF {
            return;
        }
        match self.parse_value(None) {
            None => self.simple_error(
                "Expected a JSON object, array or literal.",
                ErrorCode::UNDEFINED,
            ),
            Some(_) if self.token() != SyntaxKind::EOF => {
                self.simple_error("End of file expected.", ErrorCode::UNDEFINED)
            }
            Some(_) => {}
        }
    }
}

/// Characters of the JS regular expression class `\s`.
fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}
