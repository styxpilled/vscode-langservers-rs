//! A loaded schema document: its tree and the index that `$ref` resolution reads.

use indexmap::IndexMap;

use super::tree::{Kind, Node, Tree};
use crate::schema_service::{SchemaDiagnostic, normalize_id, to_diagnostic};
use crate::validator::get_schema_draft_from_id;
use crate::{ErrorCode, SchemaDraft};

pub(crate) type Resolve<'a> = &'a super::ResolveFn;

const UNSET: u32 = u32::MAX;

#[derive(Debug)]
pub(crate) struct Doc {
    pub uri: String,
    pub tree: Tree,
    pub draft: Option<SchemaDraft>,
    /// For each node, its index in `bases`, or `UNSET` where the schema walk did not go.
    base: Vec<u32>,
    bases: Vec<String>,
    /// For each node, its resource root, or `UNSET` where the schema walk did not go.
    scope: Vec<u32>,
    /// The resource URIs: the document, and each `$id` in it.
    pub resources: Vec<(String, Node)>,
    /// `resource#anchor`.
    pub anchors: Vec<(String, Node)>,
    /// The `$dynamicAnchor` names of each resource root.
    pub dynamic_anchors: Vec<(Node, IndexMap<String, Node>)>,
    /// Load and parse errors.
    pub load_errors: Vec<SchemaDiagnostic>,
    /// Other errors of the content.
    pub errors: Vec<SchemaDiagnostic>,
}

/// `$id || id` if it is a string that starts a new base URI.
pub(crate) fn base_id(tree: &Tree, n: Node) -> Option<&str> {
    let id = tree
        .get_truthy(n, "$id")
        .or_else(|| tree.get_truthy(n, "id"))?;
    tree.str(id).filter(|id| !id.starts_with('#'))
}

/// `$id || id`, any truthy value.
fn any_id(tree: &Tree, n: Node) -> Option<Node> {
    tree.get_truthy(n, "$id")
        .or_else(|| tree.get_truthy(n, "id"))
}

fn draft_of(tree: &Tree, n: Node) -> Option<SchemaDraft> {
    tree.get_truthy(n, "$schema")
        .and_then(|v| get_schema_draft_from_id(tree.str(v)?))
}

/// The schema children in the order of `traverseSchemaProperties`.
pub(crate) fn schema_children(tree: &Tree, n: Node) -> Vec<Node> {
    const MAP_PROPS: [&str; 6] = [
        "definitions",
        "$defs",
        "properties",
        "patternProperties",
        "dependencies",
        "dependentSchemas",
    ];
    const SINGLE_PROPS: [&str; 15] = [
        "additionalItems",
        "additionalProperties",
        "not",
        "contains",
        "propertyNames",
        "if",
        "then",
        "else",
        "unevaluatedItems",
        "unevaluatedProperties",
        "items",
        "anyOf",
        "allOf",
        "oneOf",
        "prefixItems",
    ];
    fn visit(tree: &Tree, v: Node, out: &mut Vec<Node>) {
        match tree.kind(v) {
            Kind::Arr(..) => tree.items(v).iter().for_each(|&c| visit(tree, c, out)),
            Kind::Obj(..) => out.push(v),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for prop in MAP_PROPS {
        if let Some(map) = tree.get(n, prop) {
            match tree.kind(map) {
                Kind::Obj(..) => tree
                    .entries(map)
                    .for_each(|(_, v)| visit(tree, v, &mut out)),
                Kind::Arr(..) => tree
                    .items(map)
                    .iter()
                    .for_each(|&v| visit(tree, v, &mut out)),
                _ => {}
            }
        }
    }
    for prop in SINGLE_PROPS {
        if let Some(v) = tree.get_truthy(n, prop) {
            visit(tree, v, &mut out);
        }
    }
    out
}

impl Doc {
    pub fn new(
        uri: String,
        mut tree: Tree,
        load_errors: Vec<SchemaDiagnostic>,
        resolve: Option<Resolve>,
    ) -> Doc {
        if !tree.root().is_some_and(|r| tree.is_obj(r)) {
            tree = Tree::from_json(&serde_json::json!({}));
        }
        let draft = draft_of(&tree, 0);
        let mut doc = Doc {
            base: vec![UNSET; tree.len()],
            bases: Vec::new(),
            scope: vec![UNSET; tree.len()],
            resources: vec![(uri.clone(), 0)],
            anchors: Vec::new(),
            dynamic_anchors: Vec::new(),
            uri,
            tree,
            draft,
            load_errors,
            errors: Vec::new(),
        };
        doc.index(resolve);
        doc
    }

    fn resolve_path(resolve: Option<Resolve>, id: &str, base: &str) -> String {
        match resolve {
            Some(resolve) if !crate::schema_service::has_scheme(id) => resolve(id, base),
            _ => id.to_owned(),
        }
    }

    fn index(&mut self, resolve: Option<Resolve>) {
        struct Frame {
            node: Node,
            base: u32,
            resource: u32,
            draft: Option<SchemaDraft>,
            /// The resource root for dynamic anchors.
            scope: Node,
        }
        self.bases.push(self.uri.clone());
        let mut stack = vec![Frame {
            node: 0,
            base: 0,
            resource: 0,
            draft: self.draft,
            scope: 0,
        }];
        let mut dynamic: IndexMap<Node, IndexMap<String, Node>> = IndexMap::new();
        dynamic.insert(0, IndexMap::new());
        let mut anchors: IndexMap<String, Node> = IndexMap::new();
        while let Some(f) = stack.pop() {
            let tree = &self.tree;
            let n = f.node;
            let (mut base, mut resource, mut scope) = (f.base, f.resource, f.scope);
            let draft = draft_of(tree, n).or(f.draft);
            if let Some(id) = base_id(tree, n) {
                let resolved =
                    normalize_id(&Self::resolve_path(resolve, id, &self.bases[base as usize]));
                if n == 0 && resolved == self.uri {
                    // The document URI.
                } else {
                    self.resources.push((resolved.clone(), n));
                    self.bases.push(resolved);
                    base = self.bases.len() as u32 - 1;
                }
                resource = base;
                if n != 0 {
                    scope = n;
                    dynamic.entry(n).or_default();
                }
            }
            self.base[n as usize] = base;
            self.scope[n as usize] = scope;

            let fragment_anchor = draft
                .is_none_or(|d| d < SchemaDraft::V2019_09)
                .then(|| any_id(tree, n).and_then(|id| tree.str(id)?.strip_prefix('#')))
                .flatten();
            let dollar_anchor = draft
                .is_none_or(|d| d >= SchemaDraft::V2019_09)
                .then(|| tree.get_truthy(n, "$anchor").and_then(|v| tree.str(v)))
                .flatten();
            let dynamic_anchor = draft
                .is_none_or(|d| d >= SchemaDraft::V2020_12)
                .then(|| {
                    tree.get_truthy(n, "$dynamicAnchor")
                        .and_then(|v| tree.str(v))
                })
                .flatten();
            let first = fragment_anchor.or(dollar_anchor);
            for name in [first, dynamic_anchor].into_iter().flatten() {
                if name.is_empty() {
                    continue;
                }
                let key = format!("{}#{name}", self.bases[resource as usize]);
                match anchors.get(&key) {
                    Some(&existing) if existing != n => self.errors.push(to_diagnostic(
                        format!("Duplicate anchor declaration: '{name}'"),
                        ErrorCode::SCHEMA_RESOLVE_ERROR,
                        None,
                    )),
                    _ => {
                        anchors.insert(key, n);
                    }
                }
            }
            if draft.is_none_or(|d| d >= SchemaDraft::V2020_12)
                && let Some(name) = tree.get(n, "$dynamicAnchor").and_then(|v| tree.str(v))
            {
                dynamic
                    .entry(scope)
                    .or_default()
                    .entry(name.to_owned())
                    .or_insert(n);
            }
            let children = schema_children(tree, n);
            stack.extend(children.into_iter().rev().map(|c| Frame {
                node: c,
                base,
                resource,
                draft,
                scope,
            }));
        }
        self.anchors = anchors.into_iter().collect();
        self.dynamic_anchors = dynamic.into_iter().filter(|(_, m)| !m.is_empty()).collect();
    }

    /// The resource root of a node: the document root or the nearest `$id`.
    pub fn resource_root(&self, n: Node) -> Node {
        let mut current = Some(n);
        while let Some(c) = current {
            if self.scope[c as usize] != UNSET {
                return self.scope[c as usize];
            }
            current = self.tree.parent(c);
        }
        0
    }

    /// Whether the resource of a node has a `$dynamicAnchor` or a `$recursiveAnchor`. A
    /// `$ref` chain does not skip such a node: a validation through it enters its resource,
    /// which changes `$dynamicRef` and `$recursiveRef` targets.
    pub fn in_dynamic_scope(&self, n: Node) -> bool {
        let root = self.resource_root(n);
        self.dynamic_anchors.iter().any(|(r, _)| *r == root)
            || self
                .tree
                .get(root, "$recursiveAnchor")
                .is_some_and(|v| matches!(self.tree.kind(v), Kind::Bool(true)))
    }

    /// The base URI of a node. With `exclude_self`, an `$id` of the node itself does not
    /// count.
    pub fn base(&self, n: Node, exclude_self: bool) -> &str {
        let mut current = if exclude_self {
            self.tree.parent(n)
        } else {
            Some(n)
        };
        while let Some(c) = current {
            if self.base[c as usize] != UNSET && !(exclude_self && c == n) {
                return &self.bases[self.base[c as usize] as usize];
            }
            current = self.tree.parent(c);
        }
        &self.bases[0]
    }
}
