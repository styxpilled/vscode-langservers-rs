//! A JS-like mutable object graph for schema resolution.
//!
//! Upstream resolves `$ref` by mutating the schema objects in place and by sharing
//! sub-objects between schemas. Object identity, aliasing and the JS key order all
//! change the result, so the resolver works on this graph and not on typed schemas.

use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InfoId(u32);

#[derive(Clone, Debug)]
pub enum Val {
    Null,
    Bool(bool),
    Num(f64),
    Str(Arc<str>),
    Arr(Arc<Vec<Val>>),
    Obj(ObjId),
}

impl Val {
    pub fn as_obj(&self) -> Option<ObjId> {
        match self {
            Val::Obj(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }

    /// JS truthiness.
    pub fn is_truthy(&self) -> bool {
        match self {
            Val::Null => false,
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Str(s) => !s.is_empty(),
            Val::Arr(_) | Val::Obj(_) => true,
        }
    }
}

/// `$dynamicRefInfo`. Several objects can share one record.
#[derive(Clone, Debug, Default)]
pub struct DynamicRefInfo {
    pub target: Option<ObjId>,
    pub name: Option<String>,
    pub scope: Option<ObjId>,
}

/// `$anchorMaps`.
#[derive(Clone, Debug, Default)]
pub struct AnchorMaps {
    pub dynamic: IndexMap<String, ObjId>,
    pub local: IndexMap<String, ObjId>,
}

#[derive(Clone, Debug, Default)]
pub struct Obj {
    entries: IndexMap<String, Val, foldhash::fast::RandomState>,
    pub original_id: Option<String>,
    pub dynamic_ref_info: Option<InfoId>,
    pub anchor_maps: Option<Box<AnchorMaps>>,
}

impl Obj {
    pub fn get(&self, key: &str) -> Option<&Val> {
        // Most schema objects are small, and a scan is faster than a hash.
        if self.entries.len() <= 8 {
            return self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v);
        }
        self.entries.get(key)
    }

    pub fn has(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Val)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Sets a property with the JS key order: array index keys first, in numeric
    /// order, then the other keys in insertion order.
    pub fn set(&mut self, key: &str, value: Val) {
        if let Some(slot) = self.entries.get_mut(key) {
            *slot = value;
            return;
        }
        match array_index(key) {
            None => {
                self.entries.insert(key.to_owned(), value);
            }
            Some(index) => {
                let pos = self
                    .entries
                    .keys()
                    .position(|k| array_index(k).is_none_or(|i| i > index))
                    .unwrap_or(self.entries.len());
                self.entries.shift_insert(pos, key.to_owned(), value);
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Val> {
        self.entries.shift_remove(key)
    }

    /// `{ ...obj }`: copies the properties, not the hidden metadata.
    pub fn spread(&self) -> Obj {
        Obj {
            entries: self.entries.clone(),
            ..Obj::default()
        }
    }
}

/// The value of a key that JS orders as an integer index.
pub fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    key.parse::<u32>().ok().filter(|&i| i != u32::MAX)
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    objs: Vec<Obj>,
    infos: Vec<DynamicRefInfo>,
}

impl std::ops::Index<ObjId> for Graph {
    type Output = Obj;

    fn index(&self, id: ObjId) -> &Obj {
        &self.objs[id.0 as usize]
    }
}

impl std::ops::IndexMut<ObjId> for Graph {
    fn index_mut(&mut self, id: ObjId) -> &mut Obj {
        &mut self.objs[id.0 as usize]
    }
}

impl Graph {
    pub fn add(&mut self, obj: Obj) -> ObjId {
        self.objs.push(obj);
        ObjId(self.objs.len() as u32 - 1)
    }

    pub fn new_obj(&mut self) -> ObjId {
        self.add(Obj::default())
    }

    pub fn info(&self, id: InfoId) -> &DynamicRefInfo {
        &self.infos[id.0 as usize]
    }

    pub fn info_mut(&mut self, id: InfoId) -> &mut DynamicRefInfo {
        &mut self.infos[id.0 as usize]
    }

    /// The `$dynamicRefInfo` of an object. Creates it on first use.
    pub fn info_of(&mut self, obj: ObjId) -> InfoId {
        if let Some(id) = self[obj].dynamic_ref_info {
            return id;
        }
        self.infos.push(DynamicRefInfo::default());
        let id = InfoId(self.infos.len() as u32 - 1);
        self[obj].dynamic_ref_info = Some(id);
        id
    }

    pub fn get(&self, obj: ObjId, key: &str) -> Option<&Val> {
        self[obj].get(key)
    }

    pub fn get_str(&self, obj: ObjId, key: &str) -> Option<&str> {
        self[obj].get(key).and_then(Val::as_str)
    }

    /// Whether a property is set and is not `undefined`. JSON has no `undefined`.
    pub fn has(&self, obj: ObjId, key: &str) -> bool {
        self[obj].has(key)
    }

    /// `jsonc::parse` followed by `add_json`, without the `Value` step. Returns the value and
    /// the offset of the first parse error.
    pub fn parse(&mut self, text: &str) -> (Option<Val>, Option<usize>) {
        enum Frame {
            Obj(Obj, Option<String>),
            Arr(Vec<Val>),
        }

        struct Builder<'g> {
            graph: &'g mut Graph,
            stack: Vec<Frame>,
            root: Option<Val>,
            first_error: Option<usize>,
        }

        impl Builder<'_> {
            fn on_value(&mut self, value: Val) {
                match self.stack.last_mut() {
                    Some(Frame::Arr(items)) => items.push(value),
                    Some(Frame::Obj(obj, Some(key))) => obj.set(key, value),
                    Some(Frame::Obj(_, None)) => {}
                    None => {
                        self.root.get_or_insert(value);
                    }
                }
            }

            fn end(&mut self) {
                let value = match self.stack.pop() {
                    Some(Frame::Obj(obj, _)) => Val::Obj(self.graph.add(obj)),
                    Some(Frame::Arr(items)) => Val::Arr(Arc::new(items)),
                    None => return,
                };
                self.on_value(value);
            }
        }

        impl jsonc::Visitor for Builder<'_> {
            fn on_object_begin(&mut self, _: jsonc::Span, _: &[jsonc::Segment]) -> bool {
                self.stack.push(Frame::Obj(Obj::default(), None));
                true
            }

            fn on_object_property(&mut self, name: &str, _: jsonc::Span, _: &[jsonc::Segment]) {
                if let Some(Frame::Obj(_, key)) = self.stack.last_mut() {
                    let key = key.get_or_insert_default();
                    key.clear();
                    key.push_str(name);
                }
            }

            fn on_object_end(&mut self, _: jsonc::Span) {
                self.end();
            }

            fn on_array_begin(&mut self, _: jsonc::Span, _: &[jsonc::Segment]) -> bool {
                self.stack.push(Frame::Arr(Vec::new()));
                true
            }

            fn on_array_end(&mut self, _: jsonc::Span) {
                self.end();
            }

            fn on_literal_value(
                &mut self,
                value: jsonc::Literal,
                _: jsonc::Span,
                _: &[jsonc::Segment],
            ) {
                let value = match value {
                    jsonc::Literal::Null => Val::Null,
                    jsonc::Literal::Boolean(b) => Val::Bool(b),
                    // The same conversion as `jsonc::parse`: an infinite number is `null`.
                    jsonc::Literal::Number(n) => jsonc::number_to_json(n)
                        .as_f64()
                        .map_or(Val::Null, Val::Num),
                    jsonc::Literal::String(s) => Val::Str(s.into()),
                };
                self.on_value(value);
            }

            fn on_error(&mut self, _: jsonc::ParseErrorCode, span: jsonc::Span) {
                self.first_error.get_or_insert(span.offset);
            }
        }

        let mut builder = Builder {
            graph: self,
            stack: Vec::new(),
            root: None,
            first_error: None,
        };
        jsonc::visit(text, &mut builder, jsonc::ParseOptions::default());
        (builder.root, builder.first_error)
    }

    pub fn add_json(&mut self, value: &Value) -> Val {
        match value {
            Value::Null => Val::Null,
            Value::Bool(b) => Val::Bool(*b),
            Value::Number(n) => Val::Num(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => Val::Str(s.as_str().into()),
            Value::Array(items) => {
                Val::Arr(Arc::new(items.iter().map(|v| self.add_json(v)).collect()))
            }
            Value::Object(map) => {
                let mut obj = Obj::default();
                for (k, v) in map {
                    let v = self.add_json(v);
                    obj.set(k, v);
                }
                Val::Obj(self.add(obj))
            }
        }
    }

    /// Converts to JSON. A cycle becomes `null`.
    pub fn to_json(&self, value: &Val) -> Value {
        self.to_json_marked(value, &Value::Null)
    }

    /// Converts to JSON. A cycle becomes `cycle`.
    pub fn to_json_marked(&self, value: &Val, cycle: &Value) -> Value {
        self.to_json_guarded(value, &mut Vec::new(), cycle)
    }

    fn to_json_guarded(&self, value: &Val, stack: &mut Vec<ObjId>, cycle: &Value) -> Value {
        match value {
            Val::Null => Value::Null,
            Val::Bool(b) => Value::Bool(*b),
            Val::Num(n) => jsonc::number_to_json(*n),
            Val::Str(s) => Value::String(s.to_string()),
            Val::Arr(items) => items
                .iter()
                .map(|v| self.to_json_guarded(v, stack, cycle))
                .collect(),
            Val::Obj(id) => {
                if stack.contains(id) {
                    return cycle.clone();
                }
                stack.push(*id);
                let map = self[*id]
                    .iter()
                    .map(|(k, v)| (k.to_owned(), self.to_json_guarded(v, stack, cycle)))
                    .collect();
                stack.pop();
                Value::Object(map)
            }
        }
    }

    /// The object (not array) children that `traverseSchemaProperties` visits, in its order.
    pub fn schema_children(&self, obj: ObjId) -> Vec<ObjId> {
        const MAP_PROPS: [&str; 6] = [
            "definitions",
            "$defs",
            "properties",
            "patternProperties",
            "dependencies",
            "dependentSchemas",
        ];
        const SINGLE_PROPS: [&str; 11] = [
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
        ];
        const ARRAY_PROPS: [&str; 4] = ["anyOf", "allOf", "oneOf", "prefixItems"];

        fn visit(value: &Val, out: &mut Vec<ObjId>) {
            match value {
                Val::Arr(items) => items.iter().for_each(|v| visit(v, out)),
                Val::Obj(id) => out.push(*id),
                _ => {}
            }
        }

        let node = &self[obj];
        let mut out = Vec::new();
        for prop in MAP_PROPS {
            match node.get(prop) {
                Some(Val::Obj(map)) => self[*map].iter().for_each(|(_, v)| visit(v, &mut out)),
                Some(Val::Arr(items)) => items.iter().for_each(|v| visit(v, &mut out)),
                _ => {}
            }
        }
        for prop in SINGLE_PROPS.iter().chain(&ARRAY_PROPS) {
            if let Some(v) = node.get(prop).filter(|v| v.is_truthy()) {
                visit(v, &mut out);
            }
        }
        out
    }
}
