//! Port of `cssSymbolScope.ts`. Scopes and symbols are in arenas; a symbol
//! is identified by its index.

use foldhash::HashSet;

use super::nodes::{Ast, Field, Kind, NodeId, NodeType, ReferenceType};

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub value: Option<String>,
    pub node: NodeId,
    pub ty: ReferenceType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScopeId(usize);

const GLOBAL: ScopeId = ScopeId(0);

#[derive(Debug)]
struct Scope {
    offset: isize,
    length: isize,
    parent: Option<ScopeId>,
    children: Vec<ScopeId>,
    symbols: Vec<SymbolId>,
}

/// `Symbols`.
#[derive(Debug)]
pub struct Symbols {
    scopes: Vec<Scope>,
    symbols: Vec<Symbol>,
}

/// `findFirst`: a binary search, also on input that is not sorted.
fn find_first<T>(array: &[T], p: impl Fn(&T) -> bool) -> usize {
    let (mut low, mut high) = (0, array.len());
    while low < high {
        let mid = (low + high) / 2;
        if p(&array[mid]) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

impl Symbols {
    pub fn new(ast: &Ast, root: NodeId) -> Self {
        let mut symbols = Symbols {
            scopes: vec![Scope {
                offset: 0,
                length: isize::MAX,
                parent: None,
                children: Vec::new(),
                symbols: Vec::new(),
            }],
            symbols: Vec::new(),
        };
        ast.accept(root, &mut |node| {
            symbols.visit_node(ast, node);
            true
        });
        symbols
    }

    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0]
    }

    fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.0]
    }

    fn find_scope(&self, id: ScopeId, offset: isize, length: isize) -> Option<ScopeId> {
        let s = self.scope(id);
        if s.offset <= offset && s.offset + s.length > offset + length
            || s.offset == offset && s.length == length
        {
            return Some(self.find_in_scope(id, offset, length));
        }
        None
    }

    fn find_in_scope(&self, id: ScopeId, offset: isize, length: isize) -> ScopeId {
        let end = offset + length;
        let children = &self.scope(id).children;
        let idx = find_first(children, |&c| self.scope(c).offset > end);
        if idx == 0 {
            return id;
        }
        let res = children[idx - 1];
        let r = self.scope(res);
        if r.offset <= offset && r.offset + r.length >= offset + length {
            return self.find_in_scope(res, offset, length);
        }
        id
    }

    fn get_symbol(&self, scope: ScopeId, name: &str, ty: ReferenceType) -> Option<SymbolId> {
        self.scope(scope)
            .symbols
            .iter()
            .copied()
            .find(|&s| self.symbol(s).name == name && self.symbol(s).ty == ty)
    }

    fn push_symbol(&mut self, scope: ScopeId, symbol: Symbol) {
        let id = SymbolId(self.symbols.len());
        self.symbols.push(symbol);
        self.scopes[scope.0].symbols.push(id);
    }

    // ScopeBuilder

    fn add_symbol(
        &mut self,
        ast: &Ast,
        node: NodeId,
        name: String,
        value: Option<String>,
        ty: ReferenceType,
    ) {
        let n = ast.node(node);
        if n.offset() != -1
            && let Some(current) = self.find_scope(GLOBAL, n.offset(), n.length())
        {
            self.push_symbol(
                current,
                Symbol {
                    name,
                    value,
                    node,
                    ty,
                },
            );
        }
    }

    fn add_scope(&mut self, ast: &Ast, node: NodeId) -> Option<ScopeId> {
        let n = ast.node(node);
        if n.offset() == -1 {
            return None;
        }
        let current = self.find_scope(GLOBAL, n.offset(), n.length())?;
        let c = self.scope(current);
        if c.offset != n.offset() || c.length != n.length() {
            let id = ScopeId(self.scopes.len());
            self.scopes.push(Scope {
                offset: n.offset(),
                length: n.length(),
                parent: Some(current),
                children: Vec::new(),
                symbols: Vec::new(),
            });
            self.scopes[current.0].children.push(id);
            return Some(id);
        }
        Some(current)
    }

    fn add_symbol_to_child_scope(
        &mut self,
        ast: &Ast,
        scope_node: NodeId,
        node: NodeId,
        name: String,
        value: Option<String>,
        ty: ReferenceType,
    ) {
        if ast.offset(scope_node) != -1
            && let Some(current) = self.add_scope(ast, scope_node)
        {
            self.push_symbol(
                current,
                Symbol {
                    name,
                    value,
                    node,
                    ty,
                },
            );
        }
    }

    fn visit_node(&mut self, ast: &Ast, node: NodeId) {
        let text = |n: Option<NodeId>| n.map(|n| ast.get_text(n).to_owned());
        match ast.node_type(node) {
            NodeType::Keyframe => {
                let name = ast.identifier_name(node).to_owned();
                self.add_symbol(ast, node, name, None, ReferenceType::Keyframe);
            }
            NodeType::CustomPropertyDeclaration => {
                if let Some(property) = ast.field(node, Field::Property)
                    && ast.offset(property) != -1
                {
                    let value = text(ast.field(node, Field::Value)).unwrap_or_default();
                    let name = ast.property_name(property).to_owned();
                    let symbol = Symbol {
                        name,
                        value: Some(value),
                        node: property,
                        ty: ReferenceType::Variable,
                    };
                    self.push_symbol(GLOBAL, symbol);
                }
            }
            NodeType::VariableDeclaration => {
                // No variable: an assignment to a variable of another module (`lib.$x: red`).
                if let Some(variable) = ast.field(node, Field::Variable) {
                    let name = ast.get_text(variable).to_owned();
                    let value = text(ast.field(node, Field::Value));
                    self.add_symbol(ast, node, name, value, ReferenceType::Variable);
                }
            }
            NodeType::Ruleset => {
                let n = ast.node(node);
                if let Some(current) = self.find_scope(GLOBAL, n.offset(), n.length()) {
                    for child in ast.list_children(node, Field::Selectors) {
                        if ast.kind(child) == Kind::Selector
                            && let Some(only) = ast.only_child(child)
                        {
                            let name = ast.get_text(only).to_owned();
                            let symbol = Symbol {
                                name,
                                value: None,
                                node: child,
                                ty: ReferenceType::Rule,
                            };
                            self.push_symbol(current, symbol);
                        }
                    }
                }
            }
            NodeType::MixinDeclaration => {
                let name = ast.identifier_name(node).to_owned();
                self.add_symbol(ast, node, name, None, ReferenceType::Mixin);
            }
            NodeType::FunctionDeclaration => {
                let name = ast.identifier_name(node).to_owned();
                self.add_symbol(ast, node, name, None, ReferenceType::Function);
            }
            NodeType::FunctionParameter => {
                // Parameters are part of the body scope.
                if let Some(scope_node) = ast
                    .get_parent(node)
                    .and_then(|p| ast.field(p, Field::Declarations))
                {
                    let value = text(ast.field(node, Field::DefaultValue));
                    let name = ast.identifier_name(node).to_owned();
                    self.add_symbol_to_child_scope(
                        ast,
                        scope_node,
                        node,
                        name,
                        value,
                        ReferenceType::Variable,
                    );
                }
            }
            NodeType::Declarations => {
                self.add_scope(ast, node);
            }
            NodeType::For => {
                if let Some(scope_node) = ast.field(node, Field::Declarations)
                    && let Some(variable) = ast.field(node, Field::Variable)
                {
                    let name = ast.get_text(variable).to_owned();
                    self.add_symbol_to_child_scope(
                        ast,
                        scope_node,
                        variable,
                        name,
                        None,
                        ReferenceType::Variable,
                    );
                }
            }
            NodeType::Each => {
                if let Some(scope_node) = ast.field(node, Field::Declarations) {
                    for variable in ast.list_children(node, Field::Variables) {
                        let name = ast.get_text(variable).to_owned();
                        self.add_symbol_to_child_scope(
                            ast,
                            scope_node,
                            variable,
                            name,
                            None,
                            ReferenceType::Variable,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // Queries

    /// `findSymbolsAtOffset`: the symbols of the type in the scope at the
    /// offset and its parents; an inner symbol hides an outer one.
    pub fn find_symbols_at_offset(&self, offset: isize, ty: ReferenceType) -> Vec<SymbolId> {
        let mut result = Vec::new();
        let mut names: HashSet<&str> = HashSet::default();
        let mut scope = self.find_scope(GLOBAL, offset, 0);
        while let Some(s) = scope {
            for &id in &self.scope(s).symbols {
                let symbol = self.symbol(id);
                if symbol.ty == ty && names.insert(&symbol.name) {
                    result.push(id);
                }
            }
            scope = self.scope(s).parent;
        }
        result
    }

    fn internal_find_symbol(
        &self,
        ast: &Ast,
        node: NodeId,
        types: &[ReferenceType],
    ) -> Option<SymbolId> {
        let mut scope_node = Some(node);
        if let Some(parent) = ast.parent(node) {
            if ast.kind(parent) == Kind::FunctionParameter
                && let Some(body) = ast.get_parent(parent)
                && ast.kind(body).is_body_declaration()
            {
                scope_node = ast.field(body, Field::Declarations);
            }
            if ast.kind(parent) == Kind::FunctionArgument
                && let Some(func) = ast.get_parent(parent)
                && ast.kind(func) == Kind::Function
                && let Some(func_id) = ast.field(func, Field::Identifier)
                && let Some(symbol) =
                    self.internal_find_symbol(ast, func_id, &[ReferenceType::Function])
            {
                scope_node = ast.field(self.symbol(symbol).node, Field::Declarations);
            }
        }
        let scope_node = scope_node?;
        let name = ast.get_text(node);
        let n = ast.node(scope_node);
        let mut scope = self.find_scope(GLOBAL, n.offset(), n.length());
        while let Some(s) = scope {
            for &ty in types {
                if let Some(symbol) = self.get_symbol(s, name, ty) {
                    return Some(symbol);
                }
            }
            scope = self.scope(s).parent;
        }
        None
    }

    fn evaluate_reference_types(
        &self,
        ast: &Ast,
        node: NodeId,
    ) -> Option<&'static [ReferenceType]> {
        match ast.kind(node) {
            Kind::Identifier => {
                let n = ast.node(node);
                if let Some(types) = ast.extra(node).and_then(|e| e.reference_types) {
                    return Some(types);
                }
                if n.is_custom_property {
                    return Some(&[ReferenceType::Variable]);
                }
                // A reference to a keyframe?
                if let Some(decl) = ast.get_parent_declaration(node) {
                    let name = ast.non_prefixed_property_name(decl);
                    if (name == "animation" || name == "animation-name")
                        && ast
                            .field(decl, Field::Value)
                            .is_some_and(|v| ast.offset(v) == ast.offset(node))
                    {
                        return Some(&[ReferenceType::Keyframe]);
                    }
                }
            }
            Kind::Variable => return Some(&[ReferenceType::Variable]),
            _ => {}
        }
        ast.find_a_parent(node, &[NodeType::Selector, NodeType::ExtendsReference])
            .map(|_| &[ReferenceType::Rule][..])
    }

    fn skip_interpolations(ast: &Ast, mut node: NodeId) -> Option<NodeId> {
        while ast.node_type(node) == NodeType::Interpolation {
            node = ast.get_parent(node)?;
        }
        Some(node)
    }

    pub fn find_symbol_from_node(&self, ast: &Ast, node: NodeId) -> Option<SymbolId> {
        let node = Self::skip_interpolations(ast, node)?;
        let types = self.evaluate_reference_types(ast, node)?;
        self.internal_find_symbol(ast, node, types)
    }

    pub fn matches_symbol(&self, ast: &Ast, node: NodeId, symbol: SymbolId) -> bool {
        let Some(node) = Self::skip_interpolations(ast, node) else {
            return false;
        };
        let s = self.symbol(symbol);
        if !ast.matches(node, &s.name) {
            return false;
        }
        match self.evaluate_reference_types(ast, node) {
            Some(types) if types.contains(&s.ty) => {
                self.internal_find_symbol(ast, node, types) == Some(symbol)
            }
            _ => false,
        }
    }

    pub fn find_symbol(&self, name: &str, ty: ReferenceType, offset: isize) -> Option<SymbolId> {
        let mut scope = self.find_scope(GLOBAL, offset, 0);
        while let Some(s) = scope {
            if let Some(symbol) = self.get_symbol(s, name, ty) {
                return Some(symbol);
            }
            scope = self.scope(s).parent;
        }
        None
    }
}
