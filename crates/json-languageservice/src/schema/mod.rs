//! Port of `jsonSchema.ts`. Each schema document is kept as a [`tree::Tree`]. Its nodes
//! are compiled into typed [`JsonSchema`] nodes on first use, and `$ref` is resolved then.

mod compile;
pub(crate) mod doc;
pub(crate) mod tree;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use indexmap::IndexMap;
use regress::Regex;
use serde_json::Value;

use crate::schema_service::{
    SchemaDiagnostic, decode_uri_component, has_scheme, normalize_id, split_ref, to_diagnostic,
};
use crate::{ErrorCode, SchemaDraft};
use doc::Doc;
use tree::{Kind, Node, Tree};

/// Compiles a schema from JSON, without external `$ref`.
pub fn compile_json(value: &Value) -> (Schemas, SchemaId) {
    Schemas::from_json(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SchemaId(u32);

impl SchemaId {
    /// The empty schema, which is `true`.
    pub const TRUE: SchemaId = SchemaId(0);
    /// `{ "not": {} }`, which is `false`.
    pub const FALSE: SchemaId = SchemaId(1);
}

/// `JSONSchemaRef`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaRef {
    Bool(bool),
    Schema(SchemaId),
}

impl SchemaRef {
    /// `asSchema`.
    pub fn as_schema(self) -> SchemaId {
        match self {
            SchemaRef::Bool(true) => SchemaId::TRUE,
            SchemaRef::Bool(false) => SchemaId::FALSE,
            SchemaRef::Schema(id) => id,
        }
    }
}

#[derive(Clone, Debug)]
pub enum SchemaType {
    One(String),
    Many(Vec<String>),
}

impl SchemaType {
    pub fn names(&self) -> &[String] {
        match self {
            SchemaType::One(t) => std::slice::from_ref(t),
            SchemaType::Many(t) => t,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Items {
    One(SchemaRef),
    Many(Vec<SchemaRef>),
}

#[derive(Clone, Debug)]
pub enum Dependency {
    Required(Vec<String>),
    Schema(SchemaRef),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exclusive {
    Bool(bool),
    Number(f64),
}

#[derive(Debug)]
pub struct PatternSchema {
    pub pattern: String,
    /// `None` if the pattern is not a valid regular expression.
    pub regex: Option<Regex>,
    pub schema: SchemaRef,
}

/// A `defaultSnippets` entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DefaultSnippet {
    pub label: Option<String>,
    pub description: Option<String>,
    pub markdown_description: Option<String>,
    pub body: Option<Value>,
    pub body_text: Option<String>,
}

/// A compiled schema. Fields hold only values of the type that upstream checks for,
/// so a field with an unexpected JSON type is `None`.
#[derive(Debug, Default)]
pub struct JsonSchema {
    pub id: Option<String>,
    pub dollar_id: Option<String>,
    pub schema: Option<String>,
    pub type_: Option<SchemaType>,
    pub title: Option<String>,
    pub default: Option<Box<Value>>,
    pub description: Option<String>,
    pub properties: Option<Arc<IndexMap<String, SchemaRef>>>,
    pub pattern_properties: Option<Arc<Vec<PatternSchema>>>,
    pub additional_properties: Option<SchemaRef>,
    pub dependencies: Option<Box<IndexMap<String, Dependency>>>,
    pub items: Option<Items>,
    pub unique_items: bool,
    pub additional_items: Option<SchemaRef>,
    pub pattern: Option<String>,
    pub pattern_regex: Option<Box<Regex>>,
    pub required: Option<Vec<String>>,
    pub any_of: Option<Vec<SchemaRef>>,
    pub all_of: Option<Vec<SchemaRef>>,
    pub one_of: Option<Vec<SchemaRef>>,
    pub not: Option<SchemaRef>,
    pub enum_: Option<Arc<Vec<Value>>>,
    pub format: Option<String>,
    pub const_: Option<Box<Value>>,
    pub contains: Option<SchemaRef>,
    pub property_names: Option<SchemaRef>,
    pub examples: Option<Vec<Value>>,
    pub if_: Option<SchemaRef>,
    pub then: Option<SchemaRef>,
    pub else_: Option<SchemaRef>,
    pub unevaluated_properties: Option<SchemaRef>,
    pub unevaluated_items: Option<SchemaRef>,
    pub deprecated: bool,
    pub dependent_required: Option<Box<IndexMap<String, Vec<String>>>>,
    pub dependent_schemas: Option<Box<IndexMap<String, SchemaRef>>>,
    pub recursive_ref: bool,
    pub recursive_anchor: bool,
    pub prefix_items: Option<Vec<SchemaRef>>,
    pub dynamic_anchor: Option<String>,
    /// Rare keywords are boxed, so that a node is small. Most nodes have 2 to 4 keywords.
    pub limits: Option<Box<Limits>>,
    pub vscode: Option<Box<VsCodeKeywords>>,

    pub markdown_description: Option<String>,
    pub do_not_suggest: bool,
    pub allow_comments: Option<bool>,
    pub allow_trailing_commas: Option<bool>,
    /// The deprecated spelling `allowsTrailingCommas`.
    pub allows_trailing_commas: Option<bool>,

    /// The URI of a document root, which starts a resource without an `$id`.
    pub original_id: Option<String>,
    /// `$dynamicRef`: the initial target and the plain-name fragment.
    pub dynamic_ref: Option<Box<(SchemaId, Option<String>)>>,
    /// The enclosing resource root: the document root or the nearest `$id`.
    pub resource: Option<SchemaId>,
    /// The `$dynamicAnchor` names of a resource root.
    pub dynamic_anchors: Option<Box<IndexMap<String, SchemaId>>>,
}

/// The numeric limit keywords.
#[derive(Debug, Default, PartialEq)]
pub struct Limits {
    pub min_properties: Option<f64>,
    pub max_properties: Option<f64>,
    pub min_items: Option<f64>,
    pub max_items: Option<f64>,
    pub min_length: Option<f64>,
    pub max_length: Option<f64>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub exclusive_minimum: Option<Exclusive>,
    pub exclusive_maximum: Option<Exclusive>,
    pub multiple_of: Option<f64>,
    pub min_contains: Option<f64>,
    pub max_contains: Option<f64>,
}

impl Limits {
    const NONE: Limits = Limits {
        min_properties: None,
        max_properties: None,
        min_items: None,
        max_items: None,
        min_length: None,
        max_length: None,
        minimum: None,
        maximum: None,
        exclusive_minimum: None,
        exclusive_maximum: None,
        multiple_of: None,
        min_contains: None,
        max_contains: None,
    };
}

/// The VS Code extension keywords, except the common ones.
#[derive(Debug, Default, PartialEq)]
pub struct VsCodeKeywords {
    pub default_snippets: Option<Vec<DefaultSnippet>>,
    pub error_message: Option<String>,
    pub pattern_error_message: Option<String>,
    pub deprecation_message: Option<String>,
    pub enum_descriptions: Option<Vec<Option<String>>>,
    pub enum_sort_texts: Option<Vec<Option<String>>>,
    pub enum_details: Option<Vec<Option<String>>>,
    pub markdown_enum_descriptions: Option<Vec<Option<String>>>,
    pub suggest_sort_text: Option<String>,
    pub completion_detail: Option<String>,
}

impl VsCodeKeywords {
    const NONE: VsCodeKeywords = VsCodeKeywords {
        default_snippets: None,
        error_message: None,
        pattern_error_message: None,
        deprecation_message: None,
        enum_descriptions: None,
        enum_sort_texts: None,
        enum_details: None,
        markdown_enum_descriptions: None,
        suggest_sort_text: None,
        completion_detail: None,
    };
}

impl JsonSchema {
    pub fn limits(&self) -> &Limits {
        self.limits.as_deref().unwrap_or(&Limits::NONE)
    }

    pub fn vscode(&self) -> &VsCodeKeywords {
        self.vscode.as_deref().unwrap_or(&VsCodeKeywords::NONE)
    }

    /// `$id || id || $originalId`, as the validator uses it to find resource roots.
    pub fn is_resource_root(&self) -> bool {
        self.dollar_id.is_some() || self.id.is_some() || self.original_id.is_some()
    }
}

/// A snapshot of the loaded schema documents. Each schema node is compiled on first use, and
/// `$ref` is resolved then. A `$ref` to a document that is not loaded gives `true` and is
/// recorded in [`Schemas::missing`]; the schema service loads it and makes a new snapshot.
pub struct Schemas {
    entries: Vec<Entry>,
    resources: HashMap<String, (usize, Node)>,
    anchors: HashMap<String, (usize, Node)>,
    resolve: Option<Arc<ResolveFn>>,
    true_schema: JsonSchema,
    false_schema: JsonSchema,
    issues: Mutex<Issues>,
}

pub(crate) type ResolveFn = dyn Fn(&str, &str) -> String + Send + Sync;

struct Entry {
    doc: Arc<Doc>,
    base: u32,
    cache: Box<[OnceLock<Box<JsonSchema>>]>,
}

#[derive(Default)]
struct Issues {
    missing: Vec<String>,
    errors: Vec<SchemaDiagnostic>,
}

impl std::fmt::Debug for Schemas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Schemas")
            .field("documents", &self.entries.len())
            .finish()
    }
}

impl std::ops::Index<SchemaId> for Schemas {
    type Output = JsonSchema;

    fn index(&self, id: SchemaId) -> &JsonSchema {
        match id {
            SchemaId::TRUE => &self.true_schema,
            SchemaId::FALSE => &self.false_schema,
            id => {
                let (d, n) = self.locate(id);
                self.entries[d].cache[n as usize]
                    .get_or_init(|| Box::new(compile::compile(self, d, n)))
            }
        }
    }
}

impl Schemas {
    /// `docs` are in the order of their base ids. A base id is at least 2.
    pub(crate) fn new(docs: &[(Arc<Doc>, u32)], resolve: Option<Arc<ResolveFn>>) -> Schemas {
        let mut resources = HashMap::new();
        let mut anchors = HashMap::new();
        let entries = docs
            .iter()
            .enumerate()
            .map(|(d, (doc, base))| {
                for (uri, n) in &doc.resources {
                    resources.entry(uri.clone()).or_insert((d, *n));
                }
                for (key, n) in &doc.anchors {
                    anchors.entry(key.clone()).or_insert((d, *n));
                }
                Entry {
                    doc: doc.clone(),
                    base: *base,
                    cache: (0..doc.tree.len()).map(|_| OnceLock::new()).collect(),
                }
            })
            .collect();
        Schemas {
            entries,
            resources,
            anchors,
            resolve,
            true_schema: JsonSchema::default(),
            false_schema: JsonSchema {
                not: Some(SchemaRef::Schema(SchemaId::TRUE)),
                ..JsonSchema::default()
            },
            issues: Mutex::default(),
        }
    }

    pub(crate) fn id(&self, d: usize, n: Node) -> SchemaId {
        SchemaId(self.entries[d].base + n)
    }

    /// The document and node of a schema id above [`SchemaId::FALSE`].
    pub(crate) fn locate(&self, id: SchemaId) -> (usize, Node) {
        let d = self.entries.partition_point(|e| e.base <= id.0) - 1;
        (d, id.0 - self.entries[d].base)
    }

    pub(crate) fn doc(&self, d: usize) -> &Doc {
        &self.entries[d].doc
    }

    /// The document and node of a resource URI.
    pub(crate) fn resource(&self, uri: &str) -> Option<(usize, Node)> {
        self.resources.get(uri).copied()
    }

    /// The URIs of documents that a `$ref` needs and that are not loaded.
    pub fn missing(&self) -> Vec<String> {
        self.issues.lock().unwrap().missing.clone()
    }

    /// The `$ref` resolution errors found so far.
    pub fn errors(&self) -> Vec<SchemaDiagnostic> {
        self.issues.lock().unwrap().errors.clone()
    }

    fn note_missing(&self, uri: &str) {
        let mut issues = self.issues.lock().unwrap();
        if !issues.missing.iter().any(|m| m == uri) {
            issues.missing.push(uri.to_owned());
        }
    }

    fn note_error(&self, error: SchemaDiagnostic) {
        let mut issues = self.issues.lock().unwrap();
        if !issues.errors.contains(&error) {
            issues.errors.push(error);
        }
    }

    fn resolve_path(&self, path: &str, base: &str) -> String {
        match &self.resolve {
            Some(resolve) if !has_scheme(path) => resolve(path, base),
            _ => path.to_owned(),
        }
    }

    /// The target of a `$ref` (or `$dynamicRef`) `r` of node `n` of document `d`. Follows
    /// `$ref` chains. With `lazy`, a chain stops at a node whose target document is not
    /// loaded: that node loads it when it is compiled, so only a used `$ref` loads a
    /// document.
    pub(crate) fn resolve_ref(&self, d: usize, n: Node, r: &str, lazy: bool) -> SchemaRef {
        let mut current = (d, n, r.to_owned());
        for _ in 0..32 {
            let (d, n, r) = current;
            if lazy && self.resource(&self.ref_uri(d, n, &r).0).is_none() {
                return SchemaRef::Schema(self.id(d, n));
            }
            let Some((d2, n2)) = self.ref_target(d, n, &r) else {
                return SchemaRef::Bool(true);
            };
            match self.doc(d2).tree.kind(n2) {
                Kind::Obj(..) => match compile::pure_ref(self.doc(d2), n2) {
                    Some(next) if !self.doc(d2).in_dynamic_scope(n2) => {
                        current = (d2, n2, next.to_owned());
                    }
                    _ => return SchemaRef::Schema(self.id(d2, n2)),
                },
                Kind::Bool(b) => return SchemaRef::Bool(*b),
                _ => return SchemaRef::Bool(true),
            }
        }
        SchemaRef::Bool(true)
    }

    /// The resource URI and the fragment of a `$ref`.
    fn ref_uri<'r>(&self, d: usize, n: Node, r: &'r str) -> (String, Option<&'r str>) {
        let doc = self.doc(d);
        let pre_201909 = doc.draft.is_some_and(|x| x < SchemaDraft::V2019_09);
        let base = doc.base(n, pre_201909);
        let (path, fragment) = split_ref(r);
        let uri = if path.is_empty() {
            base.to_owned()
        } else {
            normalize_id(&self.resolve_path(path, base))
        };
        (uri, fragment)
    }

    fn ref_target(&self, d: usize, n: Node, r: &str) -> Option<(usize, Node)> {
        let (uri, fragment) = self.ref_uri(d, n, r);
        let Some((d2, root)) = self.resource(&uri) else {
            self.note_missing(&uri);
            return None;
        };
        let target = self.doc(d2);
        if let Some(error) = target.load_errors.first() {
            let message = match fragment.filter(|s| !s.is_empty()) {
                Some(seg) => format!("Problems loading reference '{seg}': {}", error.message),
                None => error.message.clone(),
            };
            self.note_error(to_diagnostic(message, error.code, Some(&target.uri)));
            return None;
        }
        for error in &target.errors {
            self.note_error(error.clone());
        }
        let found = match fragment {
            None | Some("") => Some((d2, root)),
            Some(f) if f.starts_with('/') => decode_uri_component(f)
                .and_then(|p| target.tree.pointer(root, &p))
                .map(|t| (d2, t)),
            Some(f) => self.anchors.get(&format!("{uri}#{f}")).copied(),
        };
        let found = found.filter(|&(d3, t)| {
            let tree = &self.doc(d3).tree;
            tree.is_truthy(t) || matches!(tree.kind(t), Kind::Bool(false))
        });
        if found.is_none() {
            self.note_error(to_diagnostic(
                format!(
                    "$ref '{}' in '{uri}' can not be resolved.",
                    fragment.unwrap_or("")
                ),
                ErrorCode::SCHEMA_RESOLVE_ERROR,
                None,
            ));
        }
        found
    }

    /// A schema from JSON, without external `$ref`.
    pub fn from_json(value: &Value) -> (Schemas, SchemaId) {
        let doc = Doc::new(
            "schemaservice://json".to_owned(),
            Tree::from_json(value),
            Vec::new(),
            None,
        );
        let schemas = Schemas::new(&[(Arc::new(doc), 2)], None);
        let root = match value {
            Value::Bool(false) => SchemaRef::Bool(false),
            Value::Object(_) => schemas.node_ref(0, 0, false),
            _ => SchemaRef::Bool(true),
        };
        (schemas, root.as_schema())
    }

    /// Node `n` of document `d`, after `$ref` chains. `lazy` as in [`Self::resolve_ref`].
    pub(crate) fn node_ref(&self, d: usize, n: Node, lazy: bool) -> SchemaRef {
        match compile::pure_ref(self.doc(d), n) {
            Some(r) => self.resolve_ref(d, n, r, lazy),
            None => SchemaRef::Schema(self.id(d, n)),
        }
    }
}
