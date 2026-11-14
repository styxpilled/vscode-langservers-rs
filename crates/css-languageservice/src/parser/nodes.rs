//! Port of `cssNodes.ts`. Nodes are in an arena (`Ast`). Upstream classes are
//! `Kind` variants; their node fields are slots keyed by `Field`.

use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum NodeType {
    #[default]
    Undefined,
    Identifier,
    Stylesheet,
    Ruleset,
    Selector,
    SimpleSelector,
    SelectorInterpolation,
    SelectorCombinator,
    SelectorCombinatorParent,
    SelectorCombinatorSibling,
    SelectorCombinatorAllSiblings,
    SelectorCombinatorShadowPiercingDescendant,
    Page,
    PageBoxMarginBox,
    ClassSelector,
    IdentifierSelector,
    ElementNameSelector,
    PseudoSelector,
    AttributeSelector,
    Declaration,
    Declarations,
    Property,
    Expression,
    BinaryExpression,
    Term,
    Operator,
    Value,
    StringLiteral,
    URILiteral,
    EscapedValue,
    Function,
    NumericValue,
    HexColorValue,
    RatioValue,
    MixinDeclaration,
    MixinReference,
    VariableName,
    VariableDeclaration,
    Prio,
    Interpolation,
    NestedProperties,
    ExtendsReference,
    SelectorPlaceholder,
    Debug,
    If,
    Else,
    For,
    Each,
    While,
    MixinContentReference,
    MixinContentDeclaration,
    Media,
    Scope,
    Keyframe,
    FontFace,
    Import,
    Namespace,
    Invocation,
    FunctionDeclaration,
    ReturnStatement,
    MediaQuery,
    MediaCondition,
    MediaFeature,
    FunctionParameter,
    FunctionArgument,
    KeyframeSelector,
    ViewPort,
    Document,
    AtApplyRule,
    CustomPropertyDeclaration,
    CustomPropertySet,
    ListEntry,
    Supports,
    SupportsCondition,
    NamespacePrefix,
    GridLine,
    Plugin,
    UnknownAtRule,
    Use,
    ModuleConfiguration,
    Forward,
    ForwardVisibility,
    Module,
    UnicodeRange,
    Layer,
    LayerNameList,
    LayerName,
    PropertyAtRule,
    Container,
    ModuleConfig,
    SelectorList,
    StartingStyleAtRule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReferenceType {
    Mixin,
    Rule,
    Variable,
    Function,
    Keyframe,
    Unknown,
    Module,
    Forward,
    ForwardVisibility,
    Property,
}

/// The upstream node classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Node,
    Nodelist,
    UnicodeRange,
    Identifier,
    Stylesheet,
    Declarations,
    BodyDeclaration,
    RuleSet,
    Selector,
    SimpleSelector,
    AtApplyRule,
    CustomPropertySet,
    Declaration,
    CustomPropertyDeclaration,
    Property,
    Invocation,
    Function,
    FunctionParameter,
    FunctionArgument,
    IfStatement,
    ForStatement,
    EachStatement,
    WhileStatement,
    ElseStatement,
    FunctionDeclaration,
    ViewPort,
    FontFace,
    NestedProperties,
    Keyframe,
    KeyframeSelector,
    Import,
    Use,
    ModuleConfiguration,
    Forward,
    ForwardVisibility,
    Namespace,
    Media,
    Scope,
    ScopeLimits,
    Supports,
    Layer,
    PropertyAtRule,
    StartingStyleAtRule,
    Document,
    Container,
    Medialist,
    MediaQuery,
    MediaCondition,
    MediaFeature,
    SupportsCondition,
    Page,
    PageBoxMarginBox,
    Expression,
    BinaryExpression,
    Term,
    AttributeSelector,
    Operator,
    HexColorValue,
    RatioValue,
    NumericValue,
    VariableDeclaration,
    Interpolation,
    Variable,
    ExtendsReference,
    MixinContentReference,
    MixinContentDeclaration,
    MixinReference,
    MixinDeclaration,
    UnknownAtRule,
    ListEntry,
    LessGuard,
    GuardCondition,
    Module,
}

impl Kind {
    /// The type of classes that override the `type` getter.
    fn fixed_type(self) -> Option<NodeType> {
        use NodeType as T;
        Some(match self {
            Kind::Node | Kind::Nodelist | Kind::BodyDeclaration | Kind::Medialist => return None,
            Kind::LessGuard | Kind::GuardCondition => return None,
            Kind::UnicodeRange => T::UnicodeRange,
            Kind::Identifier => T::Identifier,
            Kind::Stylesheet => T::Stylesheet,
            Kind::Declarations => T::Declarations,
            Kind::RuleSet => T::Ruleset,
            Kind::Selector => T::Selector,
            Kind::SimpleSelector => T::SimpleSelector,
            Kind::AtApplyRule => T::AtApplyRule,
            Kind::CustomPropertySet => T::CustomPropertySet,
            Kind::Declaration => T::Declaration,
            Kind::CustomPropertyDeclaration => T::CustomPropertyDeclaration,
            Kind::Property => T::Property,
            Kind::Invocation => T::Invocation,
            Kind::Function => T::Function,
            Kind::FunctionParameter => T::FunctionParameter,
            Kind::FunctionArgument => T::FunctionArgument,
            Kind::IfStatement => T::If,
            Kind::ForStatement => T::For,
            Kind::EachStatement => T::Each,
            Kind::WhileStatement => T::While,
            Kind::ElseStatement => T::Else,
            Kind::FunctionDeclaration => T::FunctionDeclaration,
            Kind::ViewPort => T::ViewPort,
            Kind::FontFace => T::FontFace,
            Kind::NestedProperties => T::NestedProperties,
            Kind::Keyframe => T::Keyframe,
            Kind::KeyframeSelector => T::KeyframeSelector,
            Kind::Import => T::Import,
            Kind::Use => T::Use,
            Kind::ModuleConfiguration => T::ModuleConfiguration,
            Kind::Forward => T::Forward,
            Kind::ForwardVisibility => T::ForwardVisibility,
            Kind::Namespace => T::Namespace,
            Kind::Media => T::Media,
            Kind::Scope | Kind::ScopeLimits => T::Scope,
            Kind::Supports => T::Supports,
            Kind::Layer => T::Layer,
            Kind::PropertyAtRule => T::PropertyAtRule,
            Kind::StartingStyleAtRule => T::StartingStyleAtRule,
            Kind::Document => T::Document,
            Kind::Container => T::Container,
            Kind::MediaQuery => T::MediaQuery,
            Kind::MediaCondition => T::MediaCondition,
            Kind::MediaFeature => T::MediaFeature,
            Kind::SupportsCondition => T::SupportsCondition,
            Kind::Page => T::Page,
            Kind::PageBoxMarginBox => T::PageBoxMarginBox,
            Kind::Expression => T::Expression,
            Kind::BinaryExpression => T::BinaryExpression,
            Kind::Term => T::Term,
            Kind::AttributeSelector => T::AttributeSelector,
            Kind::Operator => T::Operator,
            Kind::HexColorValue => T::HexColorValue,
            Kind::RatioValue => T::RatioValue,
            Kind::NumericValue => T::NumericValue,
            Kind::VariableDeclaration => T::VariableDeclaration,
            Kind::Interpolation => T::Interpolation,
            Kind::Variable => T::VariableName,
            Kind::ExtendsReference => T::ExtendsReference,
            Kind::MixinContentReference => T::MixinContentReference,
            Kind::MixinContentDeclaration => T::MixinContentDeclaration,
            Kind::MixinReference => T::MixinReference,
            Kind::MixinDeclaration => T::MixinDeclaration,
            Kind::UnknownAtRule => T::UnknownAtRule,
            Kind::ListEntry => T::ListEntry,
            Kind::Module => T::Module,
        })
    }

    /// `instanceof BodyDeclaration`.
    pub fn is_body_declaration(self) -> bool {
        use Kind::*;
        matches!(
            self,
            BodyDeclaration
                | RuleSet
                | CustomPropertySet
                | IfStatement
                | ForStatement
                | EachStatement
                | WhileStatement
                | ElseStatement
                | FunctionDeclaration
                | ViewPort
                | FontFace
                | NestedProperties
                | Keyframe
                | KeyframeSelector
                | Media
                | Scope
                | Supports
                | Layer
                | PropertyAtRule
                | StartingStyleAtRule
                | Document
                | Container
                | Page
                | PageBoxMarginBox
                | MixinContentDeclaration
                | MixinDeclaration
                | UnknownAtRule
        )
    }

    /// `instanceof AbstractDeclaration`.
    pub fn is_abstract_declaration(self) -> bool {
        matches!(
            self,
            Kind::Declaration | Kind::CustomPropertyDeclaration | Kind::VariableDeclaration
        )
    }

    /// `instanceof Declaration`.
    pub fn is_declaration(self) -> bool {
        matches!(self, Kind::Declaration | Kind::CustomPropertyDeclaration)
    }

    /// `instanceof Invocation`.
    pub fn is_invocation(self) -> bool {
        matches!(self, Kind::Invocation | Kind::Function)
    }
}

/// Named node fields (`setNode(field, ...)` upstream).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Arguments,
    Conditions,
    Content,
    Declarations,
    DefaultValue,
    ElseClause,
    Expression,
    Guard,
    Identifier,
    Key,
    Keyword,
    Left,
    Module,
    Name,
    Names,
    Namespaces,
    NamespacePrefix,
    NestedProperties,
    Operator,
    Parameters,
    Property,
    PropertySet,
    RangeEnd,
    RangeStart,
    Right,
    ScopeEnd,
    ScopeStart,
    Selectors,
    Value,
    Variable,
    Variables,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ignore = 1,
    Warning = 2,
    Error = 4,
}

/// `IRule`.
#[derive(Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Debug)]
pub struct Marker {
    pub node: NodeId,
    pub rule: &'static Rule,
    pub level: Level,
    pub message: String,
    pub offset: isize,
    pub length: isize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(NonZeroU32);

impl NodeId {
    fn new(index: usize) -> Self {
        let n = u32::try_from(index + 1).expect("too many nodes");
        NodeId(NonZeroU32::new(n).expect("index + 1 is not 0"))
    }

    pub fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

/// Data that few nodes have.
#[derive(Clone, Debug)]
pub struct Extra {
    owner: NodeId,
    pub issues: Vec<Marker>,
    /// The fields after the first two.
    fields: Vec<(Field, NodeId)>,
    pub reference_types: Option<&'static [ReferenceType]>,
    pub colon_position: Option<u32>,
    pub semicolon_position: Option<u32>,
    pub l_parent: Option<u32>,
    pub r_parent: Option<u32>,
    pub needs_semicolon: bool,
    pub is_negated: Option<bool>,
}

/// The number of fields in a node. More fields go to `Extra`.
const INLINE_FIELDS: usize = 2;

#[derive(Clone, Copy, Debug)]
pub struct Node {
    offset: i32,
    length: i32,
    pub parent: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    next_sibling: Option<NodeId>,
    prev_sibling: Option<NodeId>,
    /// Index in `Ast::extras`, plus 1.
    extra: Option<NonZeroU32>,
    field_nodes: [Option<NodeId>; INLINE_FIELDS],
    field_names: [Field; INLINE_FIELDS],
    pub kind: Kind,
    /// The fixed type of the class, or the type that the parser set.
    ty: NodeType,
    /// `Identifier.isCustomProperty`.
    pub is_custom_property: bool,
}

impl Node {
    pub fn offset(&self) -> isize {
        self.offset as isize
    }

    pub fn length(&self) -> isize {
        self.length as isize
    }

    pub fn end(&self) -> isize {
        self.offset() + self.length()
    }

    pub(crate) fn set_offset(&mut self, offset: isize) {
        self.offset = offset as i32;
    }

    pub(crate) fn set_length(&mut self, length: isize) {
        self.length = length as i32;
    }

    pub fn node_type(&self) -> NodeType {
        self.ty
    }

    fn inline_fields(&self) -> impl Iterator<Item = (Field, NodeId)> {
        std::iter::zip(self.field_names, self.field_nodes)
            .map_while(|(name, node)| Some((name, node?)))
    }
}

/// The children of a node, in order.
#[derive(Clone)]
pub struct Children<'a> {
    nodes: &'a [Node],
    front: Option<NodeId>,
    back: Option<NodeId>,
}

impl Children<'_> {
    fn step(&mut self, from_front: bool) -> Option<NodeId> {
        let id = if from_front { self.front } else { self.back }?;
        if self.front == self.back {
            self.front = None;
            self.back = None;
        } else if from_front {
            self.front = self.nodes[id.index()].next_sibling;
        } else {
            self.back = self.nodes[id.index()].prev_sibling;
        }
        Some(id)
    }
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        self.step(true)
    }
}

impl DoubleEndedIterator for Children<'_> {
    fn next_back(&mut self) -> Option<NodeId> {
        self.step(false)
    }
}

/// The parse result: the nodes and the text.
#[derive(Clone, Debug, Default)]
pub struct Ast {
    text: String,
    nodes: Vec<Node>,
    extras: Vec<Extra>,
    /// Also set by issues of discarded nodes.
    has_issues: bool,
}

impl Ast {
    pub(crate) fn new(text: &str) -> Self {
        // Offsets are `i32`. JS strings are shorter than that.
        assert!(text.len() < i32::MAX as usize, "text too long");
        Ast {
            text: text.to_owned(),
            // Style sheets have about one node for each 2 to 5 bytes.
            nodes: Vec::with_capacity(text.len() / 2),
            extras: Vec::new(),
            has_issues: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub(crate) fn alloc(&mut self, kind: Kind, offset: isize, length: isize) -> NodeId {
        let id = NodeId::new(self.nodes.len());
        self.nodes.push(Node {
            offset: offset as i32,
            length: length as i32,
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
            extra: None,
            field_nodes: [None; INLINE_FIELDS],
            field_names: [Field::Arguments; INLINE_FIELDS],
            kind,
            ty: kind.fixed_type().unwrap_or_default(),
            is_custom_property: false,
        });
        id
    }

    /// Drops the nodes from `start` on, if none of them links to an older
    /// node. Failed parse attempts create nodes that upstream leaves to the
    /// garbage collector.
    pub(crate) fn discard_from(&mut self, start: usize) {
        let old = |id: NodeId| id.index() < start;
        let Some(new) = self.nodes.get(start..) else {
            return;
        };
        // A sibling of a new node has the same parent, so the sibling links
        // need no check.
        let linked = new.iter().enumerate().any(|(i, n)| {
            n.parent.is_some_and(old)
                || self.children(NodeId::new(start + i)).any(old)
                || self.fields(NodeId::new(start + i)).any(|(_, f)| old(f))
        });
        if linked {
            return;
        }
        // An older node can only link to a new node through a field whose
        // target moved to another parent.
        debug_assert!((0..start).all(|i| self.fields(NodeId::new(i)).all(|(_, f)| old(f))));
        self.nodes.truncate(start);
        // Extras of dropped nodes are at the end, after those that older
        // nodes got before the attempt.
        while self.extras.last().is_some_and(|e| !old(e.owner)) {
            self.extras.pop();
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    pub fn kind(&self, id: NodeId) -> Kind {
        self.node(id).kind
    }

    pub fn node_type(&self, id: NodeId) -> NodeType {
        self.node(id).node_type()
    }

    /// The `type` setter. Upstream throws for classes with a fixed type.
    pub(crate) fn set_type(&mut self, id: NodeId, ty: NodeType) {
        debug_assert!(self.kind(id).fixed_type().is_none());
        self.node_mut(id).ty = ty;
    }

    pub fn offset(&self, id: NodeId) -> isize {
        self.node(id).offset()
    }

    pub fn length(&self, id: NodeId) -> isize {
        self.node(id).length()
    }

    pub fn end(&self, id: NodeId) -> isize {
        self.node(id).end()
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    pub fn children(&self, id: NodeId) -> Children<'_> {
        let node = self.node(id);
        Children {
            nodes: &self.nodes,
            front: node.first_child,
            back: node.last_child,
        }
    }

    pub fn has_children(&self, id: NodeId) -> bool {
        self.node(id).first_child.is_some()
    }

    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).first_child
    }

    /// The child of a node that has one child.
    pub fn only_child(&self, id: NodeId) -> Option<NodeId> {
        let node = self.node(id);
        node.first_child
            .filter(|_| node.first_child == node.last_child)
    }

    pub fn child(&self, id: NodeId, index: usize) -> Option<NodeId> {
        self.children(id).nth(index)
    }

    pub fn field(&self, id: NodeId, field: Field) -> Option<NodeId> {
        self.fields(id).find(|(f, _)| *f == field).map(|(_, n)| n)
    }

    /// The fields in the order of their first assignment.
    pub fn fields(&self, id: NodeId) -> impl Iterator<Item = (Field, NodeId)> {
        let more = self.extra(id).map_or(&[][..], |e| &e.fields);
        self.node(id).inline_fields().chain(more.iter().copied())
    }

    pub fn extra(&self, id: NodeId) -> Option<&Extra> {
        let index = self.node(id).extra?;
        Some(&self.extras[index.get() as usize - 1])
    }

    pub(crate) fn extra_mut(&mut self, id: NodeId) -> &mut Extra {
        let index = match self.node(id).extra {
            Some(index) => index.get() as usize - 1,
            None => {
                self.extras.push(Extra {
                    owner: id,
                    issues: Vec::new(),
                    fields: Vec::new(),
                    reference_types: None,
                    colon_position: None,
                    semicolon_position: None,
                    l_parent: None,
                    r_parent: None,
                    needs_semicolon: true,
                    is_negated: None,
                });
                self.node_mut(id).extra = NonZeroU32::new(self.extras.len() as u32);
                self.extras.len() - 1
            }
        };
        &mut self.extras[index]
    }

    /// `Declaration.colonPosition`.
    pub fn colon_position(&self, id: NodeId) -> Option<isize> {
        Some(self.extra(id)?.colon_position? as isize)
    }

    /// `Declaration.semicolonPosition`.
    pub fn semicolon_position(&self, id: NodeId) -> Option<isize> {
        Some(self.extra(id)?.semicolon_position? as isize)
    }

    /// `text.substr(offset, length)` with byte offsets. A negative offset counts
    /// from the end.
    fn substr(&self, offset: isize, length: isize) -> &str {
        let n = self.text.len() as isize;
        let start = if offset < 0 {
            (n + offset).max(0)
        } else {
            offset.min(n)
        };
        let end = (start + length.max(0)).min(n);
        self.text.get(start as usize..end as usize).unwrap_or("")
    }

    /// `getText()`.
    pub fn get_text(&self, id: NodeId) -> &str {
        let n = self.node(id);
        self.substr(n.offset(), n.length())
    }

    pub fn matches(&self, id: NodeId, s: &str) -> bool {
        self.length(id) == s.len() as isize && self.get_text(id) == s
    }

    pub fn starts_with(&self, id: NodeId, s: &str) -> bool {
        self.get_text(id).starts_with(s)
    }

    pub fn ends_with(&self, id: NodeId, s: &str) -> bool {
        self.get_text(id).ends_with(s)
    }

    /// `accept`: pre-order; the visitor returns `false` to skip the children.
    pub fn accept(&self, id: NodeId, visitor: &mut impl FnMut(NodeId) -> bool) {
        let mut current = id;
        loop {
            let node = self.node(current);
            if visitor(current)
                && let Some(child) = node.first_child
            {
                current = child;
                continue;
            }
            // The next sibling of the node or of its first ancestor that has
            // one.
            loop {
                if current == id {
                    return;
                }
                let node = self.node(current);
                if let Some(next) = node.next_sibling {
                    current = next;
                    break;
                }
                let Some(parent) = node.parent else {
                    return;
                };
                current = parent;
            }
        }
    }

    /// `adoptChild`.
    pub(crate) fn adopt_child(&mut self, parent: NodeId, node: NodeId, index: Option<usize>) {
        self.unlink(node);
        // The child that follows the new one.
        let next = index.and_then(|i| self.child(parent, i));
        let prev = match next {
            Some(next) => self.node(next).prev_sibling,
            None => self.node(parent).last_child,
        };
        let n = self.node_mut(node);
        n.parent = Some(parent);
        n.prev_sibling = prev;
        n.next_sibling = next;
        match prev {
            Some(prev) => self.node_mut(prev).next_sibling = Some(node),
            None => self.node_mut(parent).first_child = Some(node),
        }
        match next {
            Some(next) => self.node_mut(next).prev_sibling = Some(node),
            None => self.node_mut(parent).last_child = Some(node),
        }
    }

    /// Removes the node from the children of its parent.
    fn unlink(&mut self, node: NodeId) {
        let Node {
            parent: Some(parent),
            prev_sibling: prev,
            next_sibling: next,
            ..
        } = *self.node(node)
        else {
            return;
        };
        match prev {
            Some(prev) => self.node_mut(prev).next_sibling = next,
            None => self.node_mut(parent).first_child = next,
        }
        match next {
            Some(next) => self.node_mut(next).prev_sibling = prev,
            None => self.node_mut(parent).last_child = prev,
        }
    }

    /// `setNode`: attaches `node` and stores it in `field`.
    pub(crate) fn set_node(
        &mut self,
        id: NodeId,
        field: Field,
        node: Option<NodeId>,
        index: Option<usize>,
    ) -> bool {
        let Some(node) = node else {
            return false;
        };
        self.adopt_child(id, node, index);
        self.set_field(id, field, node);
        true
    }

    fn set_field(&mut self, id: NodeId, field: Field, node: NodeId) {
        let n = self.node_mut(id);
        for i in 0..INLINE_FIELDS {
            if n.field_nodes[i].is_none() || n.field_names[i] == field {
                n.field_nodes[i] = Some(node);
                n.field_names[i] = field;
                return;
            }
        }
        let fields = &mut self.extra_mut(id).fields;
        match fields.iter_mut().find(|(f, _)| *f == field) {
            Some(slot) => slot.1 = node,
            None => fields.push((field, node)),
        }
    }

    /// `addChild`: attaches `node` and extends the range of `id` over it.
    pub(crate) fn add_child(&mut self, id: NodeId, node: Option<NodeId>) -> bool {
        let Some(node) = node else {
            return false;
        };
        self.adopt_child(id, node, None);
        let child = self.node(node);
        let (offset, end) = (child.offset, child.offset + child.length);
        let this = self.node_mut(id);
        if offset < this.offset || this.offset == -1 {
            this.offset = offset;
        }
        if end > this.offset + this.length || this.length == -1 {
            this.length = end - this.offset;
        }
        true
    }

    /// The lazy `Nodelist` getters (`getSelectors()` and others).
    pub(crate) fn list(&mut self, id: NodeId, field: Field) -> NodeId {
        if let Some(list) = self.field(id, field) {
            return list;
        }
        let list = self.alloc(Kind::Nodelist, -1, -1);
        self.adopt_child(id, list, None);
        self.set_field(id, field, list);
        list
    }

    pub(crate) fn add_issue(&mut self, id: NodeId, marker: Marker) {
        self.extra_mut(id).issues.push(marker);
        self.has_issues = true;
    }

    pub fn issues(&self, id: NodeId) -> &[Marker] {
        self.extra(id).map_or(&[], |e| &e.issues)
    }

    pub fn has_issue(&self, id: NodeId, rule: &Rule) -> bool {
        self.issues(id).iter().any(|m| m.rule.id == rule.id)
    }

    pub fn is_erroneous(&self, id: NodeId, recursive: bool) -> bool {
        if !self.issues(id).is_empty() {
            return true;
        }
        recursive && self.children(id).any(|c| self.is_erroneous(c, true))
    }

    /// `ParseErrorCollector.entries`.
    pub fn parse_errors(&self, id: NodeId) -> Vec<&Marker> {
        let mut entries = Vec::new();
        if !self.has_issues {
            return entries;
        }
        self.accept(id, &mut |n| {
            entries.extend(self.issues(n));
            true
        });
        entries
    }

    pub fn find_first_child_before_offset(&self, id: NodeId, offset: isize) -> Option<NodeId> {
        self.children(id).rev().find(|&c| self.offset(c) <= offset)
    }

    pub fn find_child_at_offset(&self, id: NodeId, offset: isize, go_deep: bool) -> Option<NodeId> {
        let current = self.find_first_child_before_offset(id, offset)?;
        if self.end(current) >= offset {
            if go_deep {
                return Some(
                    self.find_child_at_offset(current, offset, true)
                        .unwrap_or(current),
                );
            }
            return Some(current);
        }
        None
    }

    pub fn encloses(&self, id: NodeId, candidate: NodeId) -> bool {
        self.offset(id) <= self.offset(candidate) && self.end(id) >= self.end(candidate)
    }

    /// `getParent()`: the first parent that is not a `Nodelist`.
    pub fn get_parent(&self, id: NodeId) -> Option<NodeId> {
        let mut result = self.parent(id);
        while let Some(r) = result {
            if self.kind(r) != Kind::Nodelist {
                break;
            }
            result = self.parent(r);
        }
        result
    }

    /// `findParent`: `id` or its first ancestor of the type.
    pub fn find_parent(&self, id: NodeId, ty: NodeType) -> Option<NodeId> {
        self.find_a_parent(id, &[ty])
    }

    pub fn find_a_parent(&self, id: NodeId, types: &[NodeType]) -> Option<NodeId> {
        let mut result = Some(id);
        while let Some(r) = result {
            if types.contains(&self.node_type(r)) {
                break;
            }
            result = self.parent(r);
        }
        result
    }

    /// `getNodeAtOffset`: the shortest node at the offset.
    pub fn get_node_at_offset(&self, id: NodeId, offset: isize) -> Option<NodeId> {
        if offset < self.offset(id) || offset > self.end(id) {
            return None;
        }
        let mut candidate: Option<NodeId> = None;
        self.accept(id, &mut |n| {
            let node = self.node(n);
            if node.offset == -1 && node.length == -1 {
                return true;
            }
            if node.offset() <= offset && node.end() >= offset {
                if candidate.is_none_or(|c| node.length() <= self.length(c)) {
                    candidate = Some(n);
                }
                return true;
            }
            false
        });
        candidate
    }

    pub fn get_node_path(&self, id: NodeId, offset: isize) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut candidate = self.get_node_at_offset(id, offset);
        while let Some(c) = candidate {
            path.push(c);
            candidate = self.parent(c);
        }
        path.reverse();
        path
    }
}

/// Getters of the upstream classes.
impl Ast {
    /// `getName()` of the classes that return the text of `identifier`.
    pub fn identifier_name(&self, id: NodeId) -> &str {
        self.field(id, Field::Identifier)
            .map_or("", |n| self.get_text(n))
    }

    /// The children of a lazy `Nodelist` (`getArguments()` and others). Empty
    /// if upstream would create the list now.
    pub fn list_children(&self, id: NodeId, field: Field) -> Children<'_> {
        match self.field(id, field) {
            Some(list) => self.children(list),
            None => Children {
                nodes: &self.nodes,
                front: None,
                back: None,
            },
        }
    }

    /// `Property.getName()`: the text without trailing `_` and `+` (Less merge).
    pub fn property_name(&self, id: NodeId) -> &str {
        self.get_text(id).trim_end_matches(['_', '+'])
    }

    /// `Declaration.getFullPropertyName()`.
    pub fn full_property_name(&self, id: NodeId) -> String {
        let name = self
            .field(id, Field::Property)
            .map_or("unknown", |p| self.property_name(p));
        if let Some(parent) = self.parent(id)
            && self.kind(parent) == Kind::Declarations
            && let Some(nested) = self.get_parent(parent)
            && self.kind(nested) == Kind::NestedProperties
            && let Some(decl) = self.get_parent(nested)
            && self.kind(decl).is_declaration()
        {
            return self.full_property_name(decl) + name;
        }
        name.to_owned()
    }

    /// `Declaration.getNonPrefixedPropertyName()`.
    pub fn non_prefixed_property_name(&self, id: NodeId) -> String {
        let name = self.full_property_name(id);
        if name.starts_with('-')
            && let Some(end) = name[1..].find('-')
        {
            return name[end + 2..].to_owned();
        }
        name
    }

    /// `getParentDeclaration`: the declaration whose value holds the node.
    pub fn get_parent_declaration(&self, id: NodeId) -> Option<NodeId> {
        let decl = self.find_parent(id, NodeType::Declaration)?;
        let value = self.field(decl, Field::Value)?;
        self.encloses(value, id).then_some(decl)
    }

    /// `NumericValue.getValue()`: the digits and dots, and the unit.
    pub fn numeric_value(&self, id: NodeId) -> (&str, Option<&str>) {
        let raw = self.get_text(id);
        let n = raw
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        (&raw[..n], (n < raw.len()).then(|| &raw[n..]))
    }
}
