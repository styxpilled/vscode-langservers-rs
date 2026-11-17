//! A compact read-only JSON tree for schema documents. Node 0 is the root. Object keys keep
//! the document order; for a duplicate key the last value wins, at the first position.

use serde_json::Value;

pub(crate) type Node = u32;

#[derive(Clone, Debug)]
pub(crate) enum Kind {
    Null,
    Bool(bool),
    Num(f64),
    Str(Box<str>),
    /// Children in `kids[start..start + len]`.
    Arr(u32, u32),
    /// Values in `kids[start..start + len]`, keys in `keys[start..start + len]`.
    Obj(u32, u32),
}

#[derive(Debug, Default)]
pub(crate) struct Tree {
    nodes: Vec<Kind>,
    parents: Vec<Node>,
    kids: Vec<Node>,
    keys: Vec<Box<str>>,
}

impl Tree {
    /// Returns the tree and the offset of the first parse error. A text without a value
    /// gives an empty tree.
    pub fn parse(text: &str) -> (Tree, Option<usize>) {
        let mut builder = Builder::default();
        jsonc::visit(text, &mut builder, jsonc::ParseOptions::default());
        (builder.tree, builder.first_error)
    }

    pub fn from_json(value: &Value) -> Tree {
        let mut tree = Tree::default();
        tree.add_json(value, Node::MAX);
        tree
    }

    fn add_json(&mut self, value: &Value, parent: Node) -> Node {
        let id = self.alloc(Kind::Null, parent);
        let kind = match value {
            Value::Null => Kind::Null,
            Value::Bool(b) => Kind::Bool(*b),
            Value::Number(n) => Kind::Num(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => Kind::Str(s.as_str().into()),
            Value::Array(items) => {
                let kids: Vec<Node> = items.iter().map(|v| self.add_json(v, id)).collect();
                let start = self.kids.len() as u32;
                self.keys.extend(kids.iter().map(|_| Box::<str>::default()));
                self.kids.extend(kids);
                Kind::Arr(start, items.len() as u32)
            }
            Value::Object(map) => {
                let entries: Vec<(Box<str>, Node)> = map
                    .iter()
                    .map(|(k, v)| (k.as_str().into(), self.add_json(v, id)))
                    .collect();
                let start = self.kids.len() as u32;
                for (k, v) in entries {
                    self.keys.push(k);
                    self.kids.push(v);
                }
                Kind::Obj(start, map.len() as u32)
            }
        };
        self.nodes[id as usize] = kind;
        id
    }

    fn alloc(&mut self, kind: Kind, parent: Node) -> Node {
        self.nodes.push(kind);
        self.parents.push(parent);
        self.nodes.len() as Node - 1
    }

    pub fn root(&self) -> Option<Node> {
        (!self.nodes.is_empty()).then_some(0)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn kind(&self, n: Node) -> &Kind {
        &self.nodes[n as usize]
    }

    pub fn parent(&self, n: Node) -> Option<Node> {
        let p = self.parents[n as usize];
        (p != Node::MAX).then_some(p)
    }

    pub fn is_obj(&self, n: Node) -> bool {
        matches!(self.kind(n), Kind::Obj(..))
    }

    pub fn entries(&self, n: Node) -> impl Iterator<Item = (&str, Node)> {
        let (start, len) = match self.kind(n) {
            Kind::Obj(start, len) => (*start as usize, *len as usize),
            _ => (0, 0),
        };
        self.keys[start..start + len]
            .iter()
            .map(|k| &**k)
            .zip(self.kids[start..start + len].iter().copied())
    }

    pub fn items(&self, n: Node) -> &[Node] {
        match self.kind(n) {
            Kind::Arr(start, len) => &self.kids[*start as usize..(*start + *len) as usize],
            _ => &[],
        }
    }

    pub fn get(&self, n: Node, key: &str) -> Option<Node> {
        self.entries(n).find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn str(&self, n: Node) -> Option<&str> {
        match self.kind(n) {
            Kind::Str(s) => Some(s),
            _ => None,
        }
    }

    /// JS truthiness.
    pub fn is_truthy(&self, n: Node) -> bool {
        match self.kind(n) {
            Kind::Null => false,
            Kind::Bool(b) => *b,
            Kind::Num(x) => *x != 0.0 && !x.is_nan(),
            Kind::Str(s) => !s.is_empty(),
            Kind::Arr(..) | Kind::Obj(..) => true,
        }
    }

    /// A property that is set and truthy.
    pub fn get_truthy(&self, n: Node, key: &str) -> Option<Node> {
        self.get(n, key).filter(|&v| self.is_truthy(v))
    }

    /// Follows a JSON pointer (without `#`, already URI-decoded).
    pub fn pointer(&self, start: Node, path: &str) -> Option<Node> {
        let path = path.strip_prefix('/').unwrap_or(path);
        let mut current = start;
        for part in path.split('/') {
            let part = part.replace("~1", "/").replace("~0", "~");
            current = match self.kind(current) {
                Kind::Obj(..) => self.get(current, &part)?,
                Kind::Arr(..) => *self.items(current).get(array_index(&part)? as usize)?,
                _ => return None,
            };
        }
        Some(current)
    }

    pub fn to_json(&self, n: Node) -> Value {
        match self.kind(n) {
            Kind::Null => Value::Null,
            Kind::Bool(b) => Value::Bool(*b),
            Kind::Num(x) => jsonc::number_to_json(*x),
            Kind::Str(s) => Value::String(s.to_string()),
            Kind::Arr(..) => self.items(n).iter().map(|&c| self.to_json(c)).collect(),
            Kind::Obj(..) => Value::Object(
                self.entries(n)
                    .map(|(k, v)| (k.to_owned(), self.to_json(v)))
                    .collect(),
            ),
        }
    }
}

/// The value of a key that is an array index.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    key.parse::<u32>().ok()
}

enum Frame {
    Obj(Node, Vec<(Box<str>, Node)>, Option<Box<str>>),
    Arr(Node, Vec<Node>),
    /// A value that is not added: a second root value, or an object value without a key.
    Skip,
}

#[derive(Default)]
struct Builder {
    tree: Tree,
    stack: Vec<Frame>,
    first_error: Option<usize>,
}

impl Builder {
    fn parent(&self) -> Node {
        match self.stack.last() {
            Some(Frame::Obj(n, ..) | Frame::Arr(n, _)) => *n,
            Some(Frame::Skip) | None => Node::MAX,
        }
    }

    /// Adds a node unless it is a second root value or an object value without a key.
    fn add(&mut self, kind: Kind) -> Option<Node> {
        match self.stack.last() {
            None if self.tree.root().is_some() => return None,
            Some(Frame::Obj(_, _, None) | Frame::Skip) => return None,
            _ => {}
        }
        let parent = self.parent();
        let id = self.tree.alloc(kind, parent);
        match self.stack.last_mut() {
            Some(Frame::Arr(_, items)) => items.push(id),
            Some(Frame::Obj(_, entries, key)) => {
                let key = key.take().unwrap_or_default();
                entries.push((key, id));
            }
            Some(Frame::Skip) | None => {}
        }
        Some(id)
    }

    fn end(&mut self) {
        let tree = &mut self.tree;
        let start = tree.kids.len() as u32;
        match self.stack.pop() {
            Some(Frame::Obj(id, mut entries, _)) => {
                dedupe(&mut entries);
                let len = entries.len() as u32;
                for (k, v) in entries {
                    tree.keys.push(k);
                    tree.kids.push(v);
                }
                tree.nodes[id as usize] = Kind::Obj(start, len);
            }
            Some(Frame::Arr(id, items)) => {
                let len = items.len() as u32;
                tree.keys
                    .extend(items.iter().map(|_| Box::<str>::default()));
                tree.kids.extend(items);
                tree.nodes[id as usize] = Kind::Arr(start, len);
            }
            Some(Frame::Skip) | None => {}
        }
    }
}

/// Keeps the last value of a duplicate key, at the position of the first.
fn dedupe(entries: &mut Vec<(Box<str>, Node)>) {
    let has_duplicates = if entries.len() <= 16 {
        (1..entries.len()).any(|i| entries[..i].iter().any(|(k, _)| *k == entries[i].0))
    } else {
        let mut seen = std::collections::HashSet::new();
        !entries.iter().all(|(k, _)| seen.insert(&**k))
    };
    if !has_duplicates {
        return;
    }
    let mut out: Vec<(Box<str>, Node)> = Vec::with_capacity(entries.len());
    for (k, v) in entries.drain(..) {
        match out.iter_mut().find(|(o, _)| *o == k) {
            Some(slot) => slot.1 = v,
            None => out.push((k, v)),
        }
    }
    *entries = out;
}

impl jsonc::Visitor for Builder {
    fn on_object_begin(&mut self, _: jsonc::Span, _: &[jsonc::Segment]) -> bool {
        let frame = match self.add(Kind::Obj(0, 0)) {
            Some(id) => Frame::Obj(id, Vec::new(), None),
            None => Frame::Skip,
        };
        self.stack.push(frame);
        true
    }

    fn on_object_property(&mut self, name: &str, _: jsonc::Span, _: &[jsonc::Segment]) {
        if let Some(Frame::Obj(_, _, key)) = self.stack.last_mut() {
            *key = Some(name.into());
        }
    }

    fn on_object_end(&mut self, _: jsonc::Span) {
        self.end();
    }

    fn on_array_begin(&mut self, _: jsonc::Span, _: &[jsonc::Segment]) -> bool {
        let frame = match self.add(Kind::Arr(0, 0)) {
            Some(id) => Frame::Arr(id, Vec::new()),
            None => Frame::Skip,
        };
        self.stack.push(frame);
        true
    }

    fn on_array_end(&mut self, _: jsonc::Span) {
        self.end();
    }

    fn on_literal_value(&mut self, value: jsonc::Literal, _: jsonc::Span, _: &[jsonc::Segment]) {
        let kind = match value {
            jsonc::Literal::Null => Kind::Null,
            jsonc::Literal::Boolean(b) => Kind::Bool(b),
            // The same conversion as `jsonc::parse`: an infinite number is `null`.
            jsonc::Literal::Number(n) => jsonc::number_to_json(n)
                .as_f64()
                .map_or(Kind::Null, Kind::Num),
            jsonc::Literal::String(s) => Kind::Str(s.into()),
        };
        self.add(kind);
    }

    fn on_error(&mut self, _: jsonc::ParseErrorCode, span: jsonc::Span) {
        self.first_error.get_or_insert(span.offset);
    }
}
