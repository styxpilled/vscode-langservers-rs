use std::ops::Index;

use serde_json::{Map, Value};

use crate::{
    Literal, NodeType, ParseError, ParseErrorCode, ParseOptions, ScanError, Scanner, Segment, Span,
    SyntaxKind,
};

/// Callbacks for [`visit`]. The `path` arguments give the location of the current value.
pub trait Visitor {
    /// The span is the open brace. Return `false` to skip the callbacks for the object content.
    fn on_object_begin(&mut self, _span: Span, _path: &[Segment]) -> bool {
        true
    }

    /// The span is the property name. The path does not include the property.
    fn on_object_property(&mut self, _name: &str, _span: Span, _path: &[Segment]) {}

    fn on_object_end(&mut self, _span: Span) {}

    /// The span is the open bracket. Return `false` to skip the callbacks for the array content.
    fn on_array_begin(&mut self, _span: Span, _path: &[Segment]) -> bool {
        true
    }

    fn on_array_end(&mut self, _span: Span) {}

    fn on_literal_value(&mut self, _value: Literal, _span: Span, _path: &[Segment]) {}

    /// `separator` is `,` or `:`.
    fn on_separator(&mut self, _separator: char, _span: Span) {}

    /// Called only when comments are allowed.
    fn on_comment(&mut self, _span: Span) {}

    fn on_error(&mut self, _error: ParseErrorCode, _span: Span) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub node_type: NodeType,
    /// Set for literals, and for the property name in [`Location::previous_node`].
    pub value: Option<Literal>,
    pub offset: usize,
    pub length: usize,
    pub colon_offset: Option<usize>,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

impl Node {
    fn new(node_type: NodeType, offset: usize, parent: Option<NodeId>) -> Self {
        Self {
            node_type,
            value: None,
            offset,
            length: 0,
            colon_offset: None,
            parent,
            children: Vec::new(),
        }
    }

    pub fn end(&self) -> usize {
        self.offset + self.length
    }

    pub fn contains(&self, offset: usize, include_right_bound: bool) -> bool {
        (offset >= self.offset && offset < self.end())
            || (include_right_bound && offset == self.end())
    }
}

/// A syntax tree. A property node has the name as its first child and the value, if any,
/// as its second child.
#[derive(Clone, Debug)]
pub struct Tree {
    nodes: Vec<Node>,
}

impl Index<NodeId> for Tree {
    type Output = Node;

    fn index(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }
}

impl Tree {
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn find_node_at_location(&self, from: NodeId, path: &[Segment]) -> Option<NodeId> {
        let mut node = from;
        for segment in path {
            let current = &self[node];
            node = match segment {
                Segment::Property(name) => {
                    if current.node_type != NodeType::Object {
                        return None;
                    }
                    current.children.iter().find_map(|&property| {
                        match self[property].children.as_slice() {
                            [key, value] if self.is_string(*key, name) => Some(*value),
                            _ => None,
                        }
                    })?
                }
                Segment::Index(index) => {
                    if current.node_type != NodeType::Array {
                        return None;
                    }
                    *current.children.get(*index)?
                }
            };
        }
        Some(node)
    }

    fn is_string(&self, id: NodeId, expected: &str) -> bool {
        matches!(&self[id].value, Some(Literal::String(s)) if s == expected)
    }

    pub fn node_path(&self, id: NodeId) -> Vec<Segment> {
        let mut path = Vec::new();
        let mut node = id;
        while let Some(parent) = self[node].parent {
            let parent_node = &self[parent];
            match parent_node.node_type {
                NodeType::Property => {
                    if let Some(Literal::String(key)) = &self[parent_node.children[0]].value {
                        path.push(Segment::Property(key.clone()));
                    }
                }
                NodeType::Array => {
                    if let Some(index) = parent_node.children.iter().position(|&c| c == node) {
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
        match node.node_type {
            NodeType::Array => node.children.iter().map(|&c| self.node_value(c)).collect(),
            NodeType::Object => {
                let mut object = Map::new();
                for &property in &node.children {
                    if let [key, value] = self[property].children.as_slice()
                        && let Some(Literal::String(key)) = &self[*key].value
                    {
                        object.insert(key.clone(), self.node_value(*value));
                    }
                }
                Value::Object(object)
            }
            NodeType::Property => Value::Null,
            _ => node.value.as_ref().map_or(Value::Null, Literal::to_json),
        }
    }

    /// Finds the innermost node at the offset.
    pub fn find_node_at_offset(
        &self,
        from: NodeId,
        offset: usize,
        include_right_bound: bool,
    ) -> Option<NodeId> {
        let node = &self[from];
        if !node.contains(offset, include_right_bound) {
            return None;
        }
        node.children
            .iter()
            .take_while(|&&child| self[child].offset <= offset)
            .find_map(|&child| self.find_node_at_offset(child, offset, include_right_bound))
            .or(Some(from))
    }
}

pub struct Location {
    /// The previous property name or literal value.
    pub previous_node: Option<Node>,
    /// The path of the location. Inside an object, before a property name, the last segment is
    /// an empty property name.
    pub path: Vec<Segment>,
    pub is_at_property_key: bool,
}

impl Location {
    /// The pattern `*` matches one segment. The pattern `**` matches zero or more segments.
    pub fn matches(&self, pattern: &[Segment]) -> bool {
        let is = |segment: &Segment, s: &str| matches!(segment, Segment::Property(p) if p == s);
        let mut k = 0;
        for segment in &self.path {
            if k >= pattern.len() {
                break;
            }
            if pattern[k] == *segment || is(&pattern[k], "*") {
                k += 1;
            } else if !is(&pattern[k], "**") {
                return false;
            }
        }
        k == pattern.len()
    }
}

pub fn get_location(text: &str, position: usize) -> Location {
    struct LocationVisitor {
        position: usize,
        location: Location,
        done: bool,
    }

    impl LocationVisitor {
        fn set_previous_node(&mut self, value: Literal, span: Span, node_type: NodeType) {
            let mut node = Node::new(node_type, span.offset, None);
            node.value = Some(value);
            node.length = span.length;
            self.location.previous_node = Some(node);
        }

        fn begin(&mut self, span: Span, segment: Segment) {
            if self.done || self.position <= span.offset {
                self.done = true;
                return;
            }
            self.location.previous_node = None;
            self.location.path.push(segment);
        }

        fn end(&mut self, span: Span) {
            if self.done || self.position <= span.offset {
                self.done = true;
                return;
            }
            self.location.previous_node = None;
            self.location.path.pop();
        }
    }

    impl Visitor for LocationVisitor {
        fn on_object_begin(&mut self, span: Span, _: &[Segment]) -> bool {
            self.begin(span, Segment::Property(String::new()));
            if !self.done {
                self.location.is_at_property_key = true;
            }
            true
        }

        fn on_object_property(&mut self, name: &str, span: Span, _: &[Segment]) {
            if self.done || self.position < span.offset {
                self.done = true;
                return;
            }
            self.set_previous_node(Literal::String(name.to_owned()), span, NodeType::Property);
            if let Some(last) = self.location.path.last_mut() {
                *last = Segment::Property(name.to_owned());
            }
            if self.position <= span.offset + span.length {
                self.done = true;
            }
        }

        fn on_object_end(&mut self, span: Span) {
            self.end(span);
        }

        fn on_array_begin(&mut self, span: Span, _: &[Segment]) -> bool {
            self.begin(span, Segment::Index(0));
            true
        }

        fn on_array_end(&mut self, span: Span) {
            self.end(span);
        }

        fn on_literal_value(&mut self, value: Literal, span: Span, _: &[Segment]) {
            if self.done || self.position < span.offset {
                self.done = true;
                return;
            }
            let node_type = value.node_type();
            self.set_previous_node(value, span, node_type);
            if self.position <= span.offset + span.length {
                self.done = true;
            }
        }

        fn on_separator(&mut self, separator: char, span: Span) {
            if self.done || self.position <= span.offset {
                self.done = true;
                return;
            }
            let location = &mut self.location;
            if separator == ':' {
                if location
                    .previous_node
                    .as_ref()
                    .is_some_and(|node| node.node_type == NodeType::Property)
                {
                    location.is_at_property_key = false;
                    location.previous_node = None;
                }
            } else if separator == ',' {
                match location.path.last_mut() {
                    Some(Segment::Index(index)) => *index += 1,
                    Some(last) => {
                        *last = Segment::Property(String::new());
                        location.is_at_property_key = true;
                    }
                    None => {}
                }
                location.previous_node = None;
            }
        }
    }

    let mut visitor = LocationVisitor {
        position,
        location: Location {
            previous_node: None,
            path: Vec::new(),
            is_at_property_key: false,
        },
        done: false,
    };
    visit(text, &mut visitor, ParseOptions::default());
    visitor.location
}

/// Parses the text into a JSON value. The parser tolerates errors and returns them with the
/// value. The value is `None` only if the text has no value.
pub fn parse(text: &str, options: ParseOptions) -> (Option<Value>, Vec<ParseError>) {
    #[derive(Default)]
    struct ValueBuilder {
        stack: Vec<(Value, Option<String>)>,
        root: Option<Value>,
        errors: Vec<ParseError>,
    }

    impl ValueBuilder {
        fn on_value(&mut self, value: Value) {
            match self.stack.last_mut() {
                Some((Value::Array(array), _)) => array.push(value),
                Some((Value::Object(object), Some(property))) => {
                    object.insert(property.clone(), value);
                }
                Some(_) => {}
                None => {
                    self.root.get_or_insert(value);
                }
            }
        }

        fn end(&mut self) {
            if let Some((value, _)) = self.stack.pop() {
                self.on_value(value);
            }
        }
    }

    impl Visitor for ValueBuilder {
        fn on_object_begin(&mut self, _: Span, _: &[Segment]) -> bool {
            self.stack.push((Value::Object(Map::new()), None));
            true
        }

        fn on_object_property(&mut self, name: &str, _: Span, _: &[Segment]) {
            if let Some((_, property)) = self.stack.last_mut() {
                *property = Some(name.to_owned());
            }
        }

        fn on_object_end(&mut self, _: Span) {
            self.end();
        }

        fn on_array_begin(&mut self, _: Span, _: &[Segment]) -> bool {
            self.stack.push((Value::Array(Vec::new()), None));
            true
        }

        fn on_array_end(&mut self, _: Span) {
            self.end();
        }

        fn on_literal_value(&mut self, value: Literal, _: Span, _: &[Segment]) {
            self.on_value(value.to_json());
        }

        fn on_error(&mut self, error: ParseErrorCode, span: Span) {
            self.errors.push(parse_error(error, span));
        }
    }

    let mut builder = ValueBuilder::default();
    visit(text, &mut builder, options);
    (builder.root, builder.errors)
}

/// Parses the text into a syntax tree. The parser tolerates errors and returns them with the
/// tree. The tree is `None` only if the text has no value.
pub fn parse_tree(text: &str, options: ParseOptions) -> (Option<Tree>, Vec<ParseError>) {
    #[derive(Default)]
    struct TreeBuilder {
        nodes: Vec<Node>,
        current: Option<NodeId>,
        errors: Vec<ParseError>,
    }

    impl TreeBuilder {
        fn push(&mut self, node: Node) -> NodeId {
            let id = NodeId(self.nodes.len() as u32);
            if let Some(parent) = node.parent {
                self.nodes[parent.0 as usize].children.push(id);
            }
            self.nodes.push(node);
            id
        }

        fn current_mut(&mut self) -> Option<&mut Node> {
            self.current.map(|id| &mut self.nodes[id.0 as usize])
        }

        /// Ends the current node at `end` and makes its parent current.
        fn close(&mut self, end: usize) {
            if let Some(node) = self.current_mut() {
                node.length = end - node.offset;
                self.current = node.parent;
            }
        }

        fn ensure_property_complete(&mut self, end: usize) {
            if self
                .current_mut()
                .is_some_and(|node| node.node_type == NodeType::Property)
            {
                self.close(end);
            }
        }
    }

    impl Visitor for TreeBuilder {
        fn on_object_begin(&mut self, span: Span, _: &[Segment]) -> bool {
            self.current = Some(self.push(Node::new(NodeType::Object, span.offset, self.current)));
            true
        }

        fn on_object_property(&mut self, name: &str, span: Span, _: &[Segment]) {
            let property = self.push(Node::new(NodeType::Property, span.offset, self.current));
            self.current = Some(property);
            let mut key = Node::new(NodeType::String, span.offset, Some(property));
            key.length = span.length;
            key.value = Some(Literal::String(name.to_owned()));
            self.push(key);
        }

        fn on_object_end(&mut self, span: Span) {
            let end = span.offset + span.length;
            // A property without a value is not complete yet.
            self.ensure_property_complete(end);
            self.close(end);
            self.ensure_property_complete(end);
        }

        fn on_array_begin(&mut self, span: Span, _: &[Segment]) -> bool {
            self.current = Some(self.push(Node::new(NodeType::Array, span.offset, self.current)));
            true
        }

        fn on_array_end(&mut self, span: Span) {
            let end = span.offset + span.length;
            self.close(end);
            self.ensure_property_complete(end);
        }

        fn on_literal_value(&mut self, value: Literal, span: Span, _: &[Segment]) {
            let mut node = Node::new(value.node_type(), span.offset, self.current);
            node.length = span.length;
            node.value = Some(value);
            self.push(node);
            self.ensure_property_complete(span.offset + span.length);
        }

        fn on_separator(&mut self, separator: char, span: Span) {
            let Some(node) = self.current_mut() else {
                return;
            };
            if node.node_type != NodeType::Property {
                return;
            }
            if separator == ':' {
                node.colon_offset = Some(span.offset);
            } else if separator == ',' {
                self.ensure_property_complete(span.offset);
            }
        }

        fn on_error(&mut self, error: ParseErrorCode, span: Span) {
            self.errors.push(parse_error(error, span));
        }
    }

    let mut builder = TreeBuilder::default();
    visit(text, &mut builder, options);
    let tree = (!builder.nodes.is_empty()).then_some(Tree {
        nodes: builder.nodes,
    });
    (tree, builder.errors)
}

fn parse_error(error: ParseErrorCode, span: Span) -> ParseError {
    ParseError {
        error,
        offset: span.offset,
        length: span.length,
        start_line: span.start_line,
        start_character: span.start_character,
    }
}

/// Parses the text and calls the visitor for each object, array and literal.
pub fn visit<V: Visitor + ?Sized>(text: &str, visitor: &mut V, options: ParseOptions) {
    let mut walker = Walker {
        scanner: Scanner::new(text, false),
        visitor,
        path: Vec::new(),
        suppressed: 0,
        options,
    };
    walker.run();
}

struct Walker<'a, 'v, V: Visitor + ?Sized> {
    scanner: Scanner<'a>,
    visitor: &'v mut V,
    path: Vec<Segment>,
    /// Depth of the begin callbacks that returned `false`. Callbacks occur only at depth 0.
    suppressed: usize,
    options: ParseOptions,
}

impl<V: Visitor + ?Sized> Walker<'_, '_, V> {
    fn span(&self) -> Span {
        Span {
            offset: self.scanner.token_offset(),
            length: self.scanner.token_length(),
            start_line: self.scanner.token_start_line(),
            start_character: self.scanner.token_start_character(),
        }
    }

    fn on_begin(&mut self, array: bool) {
        if self.suppressed > 0 {
            self.suppressed += 1;
            return;
        }
        let span = self.span();
        let keep = if array {
            self.visitor.on_array_begin(span, &self.path)
        } else {
            self.visitor.on_object_begin(span, &self.path)
        };
        if !keep {
            self.suppressed = 1;
        }
    }

    fn on_end(&mut self, array: bool) {
        self.suppressed = self.suppressed.saturating_sub(1);
        if self.suppressed == 0 {
            let span = self.span();
            if array {
                self.visitor.on_array_end(span);
            } else {
                self.visitor.on_object_end(span);
            }
        }
    }

    fn on_literal_value(&mut self, value: Literal) {
        if self.suppressed == 0 {
            self.visitor
                .on_literal_value(value, self.span(), &self.path);
        }
    }

    fn on_separator(&mut self, separator: char) {
        if self.suppressed == 0 {
            self.visitor.on_separator(separator, self.span());
        }
    }

    fn on_error(&mut self, error: ParseErrorCode) {
        if self.suppressed == 0 {
            self.visitor.on_error(error, self.span());
        }
    }

    fn scan_next(&mut self) -> SyntaxKind {
        loop {
            let token = self.scanner.scan();
            let error = match self.scanner.token_error() {
                ScanError::None => None,
                ScanError::InvalidUnicode => Some(ParseErrorCode::InvalidUnicode),
                ScanError::InvalidEscapeCharacter => Some(ParseErrorCode::InvalidEscapeCharacter),
                ScanError::UnexpectedEndOfNumber => Some(ParseErrorCode::UnexpectedEndOfNumber),
                ScanError::UnexpectedEndOfComment => (!self.options.disallow_comments)
                    .then_some(ParseErrorCode::UnexpectedEndOfComment),
                ScanError::UnexpectedEndOfString => Some(ParseErrorCode::UnexpectedEndOfString),
                ScanError::InvalidCharacter => Some(ParseErrorCode::InvalidCharacter),
            };
            if let Some(error) = error {
                self.handle_error(error, &[], &[]);
            }
            match token {
                SyntaxKind::LineCommentTrivia | SyntaxKind::BlockCommentTrivia => {
                    if self.options.disallow_comments {
                        self.handle_error(ParseErrorCode::InvalidCommentToken, &[], &[]);
                    } else if self.suppressed == 0 {
                        self.visitor.on_comment(self.span());
                    }
                }
                SyntaxKind::Unknown => self.handle_error(ParseErrorCode::InvalidSymbol, &[], &[]),
                SyntaxKind::Trivia | SyntaxKind::LineBreakTrivia => {}
                _ => return token,
            }
        }
    }

    fn handle_error(
        &mut self,
        error: ParseErrorCode,
        skip_until_after: &[SyntaxKind],
        skip_until: &[SyntaxKind],
    ) {
        self.on_error(error);
        if skip_until_after.is_empty() && skip_until.is_empty() {
            return;
        }
        let mut token = self.scanner.token();
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

    fn token(&self) -> SyntaxKind {
        self.scanner.token()
    }

    fn parse_string(&mut self, is_value: bool) {
        let value = self.scanner.token_value().to_owned();
        if is_value {
            self.on_literal_value(Literal::String(value));
        } else {
            if self.suppressed == 0 {
                self.visitor
                    .on_object_property(&value, self.span(), &self.path);
            }
            self.path.push(Segment::Property(value));
        }
        self.scan_next();
    }

    fn parse_literal(&mut self) -> bool {
        let value = match self.token() {
            SyntaxKind::NumericLiteral => {
                let value = self
                    .scanner
                    .token_value()
                    .parse::<f64>()
                    .unwrap_or_else(|_| {
                        self.handle_error(ParseErrorCode::InvalidNumberFormat, &[], &[]);
                        0.0
                    });
                Literal::Number(value)
            }
            SyntaxKind::NullKeyword => Literal::Null,
            SyntaxKind::TrueKeyword => Literal::Boolean(true),
            SyntaxKind::FalseKeyword => Literal::Boolean(false),
            _ => return false,
        };
        self.on_literal_value(value);
        self.scan_next();
        true
    }

    fn parse_property(&mut self) -> bool {
        const RECOVERY: &[SyntaxKind] = &[SyntaxKind::CloseBraceToken, SyntaxKind::CommaToken];
        if self.token() != SyntaxKind::StringLiteral {
            self.handle_error(ParseErrorCode::PropertyNameExpected, &[], RECOVERY);
            return false;
        }
        self.parse_string(false);
        if self.token() == SyntaxKind::ColonToken {
            self.on_separator(':');
            self.scan_next();
            if !self.parse_value() {
                self.handle_error(ParseErrorCode::ValueExpected, &[], RECOVERY);
            }
        } else {
            self.handle_error(ParseErrorCode::ColonExpected, &[], RECOVERY);
        }
        self.path.pop();
        true
    }

    fn parse_object(&mut self) -> bool {
        self.on_begin(false);
        self.scan_next();

        let mut needs_comma = false;
        while !matches!(self.token(), SyntaxKind::CloseBraceToken | SyntaxKind::EOF) {
            if self.token() == SyntaxKind::CommaToken {
                if !needs_comma {
                    self.handle_error(ParseErrorCode::ValueExpected, &[], &[]);
                }
                self.on_separator(',');
                self.scan_next();
                if self.token() == SyntaxKind::CloseBraceToken && self.options.allow_trailing_comma
                {
                    break;
                }
            } else if needs_comma {
                self.handle_error(ParseErrorCode::CommaExpected, &[], &[]);
            }
            if !self.parse_property() {
                self.handle_error(
                    ParseErrorCode::ValueExpected,
                    &[],
                    &[SyntaxKind::CloseBraceToken, SyntaxKind::CommaToken],
                );
            }
            needs_comma = true;
        }
        self.on_end(false);
        if self.token() != SyntaxKind::CloseBraceToken {
            self.handle_error(
                ParseErrorCode::CloseBraceExpected,
                &[SyntaxKind::CloseBraceToken],
                &[],
            );
        } else {
            self.scan_next();
        }
        true
    }

    fn parse_array(&mut self) -> bool {
        self.on_begin(true);
        self.scan_next();

        let mut is_first_element = true;
        let mut needs_comma = false;
        while !matches!(
            self.token(),
            SyntaxKind::CloseBracketToken | SyntaxKind::EOF
        ) {
            if self.token() == SyntaxKind::CommaToken {
                if !needs_comma {
                    self.handle_error(ParseErrorCode::ValueExpected, &[], &[]);
                }
                self.on_separator(',');
                self.scan_next();
                if self.token() == SyntaxKind::CloseBracketToken
                    && self.options.allow_trailing_comma
                {
                    break;
                }
            } else if needs_comma {
                self.handle_error(ParseErrorCode::CommaExpected, &[], &[]);
            }
            if is_first_element {
                self.path.push(Segment::Index(0));
                is_first_element = false;
            } else if let Some(Segment::Index(index)) = self.path.last_mut() {
                *index += 1;
            }
            if !self.parse_value() {
                self.handle_error(
                    ParseErrorCode::ValueExpected,
                    &[],
                    &[SyntaxKind::CloseBracketToken, SyntaxKind::CommaToken],
                );
            }
            needs_comma = true;
        }
        self.on_end(true);
        if !is_first_element {
            self.path.pop();
        }
        if self.token() != SyntaxKind::CloseBracketToken {
            self.handle_error(
                ParseErrorCode::CloseBracketExpected,
                &[SyntaxKind::CloseBracketToken],
                &[],
            );
        } else {
            self.scan_next();
        }
        true
    }

    fn parse_value(&mut self) -> bool {
        match self.token() {
            SyntaxKind::OpenBracketToken => self.parse_array(),
            SyntaxKind::OpenBraceToken => self.parse_object(),
            SyntaxKind::StringLiteral => {
                self.parse_string(true);
                true
            }
            _ => self.parse_literal(),
        }
    }

    fn run(&mut self) {
        self.scan_next();
        if self.token() == SyntaxKind::EOF {
            if !self.options.allow_empty_content {
                self.handle_error(ParseErrorCode::ValueExpected, &[], &[]);
            }
            return;
        }
        if !self.parse_value() {
            self.handle_error(ParseErrorCode::ValueExpected, &[], &[]);
            return;
        }
        if self.token() != SyntaxKind::EOF {
            self.handle_error(ParseErrorCode::EndOfFileExpected, &[], &[]);
        }
    }
}

/// Removes comments. With `replace_ch`, replaces each comment character except line breaks
/// with `replace_ch`.
pub fn strip_comments(text: &str, replace_ch: Option<char>) -> String {
    let mut scanner = Scanner::new(text, false);
    let mut result = String::with_capacity(text.len());
    let mut offset = 0;
    loop {
        let pos = scanner.position();
        let kind = scanner.scan();
        if matches!(
            kind,
            SyntaxKind::LineCommentTrivia | SyntaxKind::BlockCommentTrivia | SyntaxKind::EOF
        ) {
            result.push_str(&text[offset..pos]);
            if let Some(replacement) = replace_ch {
                result.extend(scanner.token_value().chars().map(|c| match c {
                    '\r' | '\n' => c,
                    _ => replacement,
                }));
            }
            offset = scanner.position();
        }
        if kind == SyntaxKind::EOF {
            return result;
        }
    }
}
