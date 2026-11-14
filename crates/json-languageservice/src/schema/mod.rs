//! Port of `jsonSchema.ts`. Schemas are resolved on a [`graph::Graph`] and then
//! compiled into typed [`JsonSchema`] nodes in a [`Schemas`] arena.

mod compile;
pub(crate) mod graph;

use std::sync::Arc;

use indexmap::IndexMap;
use regress::Regex;
use serde_json::Value;

pub(crate) use compile::compile;
pub use compile::compile_json;

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

    /// `$originalId`: the id of a schema that was merged into this one.
    pub original_id: Option<String>,
    /// `$dynamicRefInfo`: the initial target and the plain-name fragment.
    pub dynamic_ref: Option<Box<(SchemaId, Option<String>)>>,
    /// `$anchorMaps.dynamic`.
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

/// An arena of compiled schemas. Index 0 is [`SchemaId::TRUE`], index 1 is [`SchemaId::FALSE`].
#[derive(Debug)]
pub struct Schemas {
    /// Boxed, so that `Vec` growth copies pointers and not nodes. This halves the peak
    /// memory of a schema load.
    #[allow(clippy::vec_box)]
    nodes: Vec<Box<JsonSchema>>,
}

impl Default for Schemas {
    fn default() -> Self {
        Schemas {
            nodes: vec![
                Box::default(),
                Box::new(JsonSchema {
                    not: Some(SchemaRef::Schema(SchemaId::TRUE)),
                    ..JsonSchema::default()
                }),
            ],
        }
    }
}

impl std::ops::Index<SchemaId> for Schemas {
    type Output = JsonSchema;

    fn index(&self, id: SchemaId) -> &JsonSchema {
        &self.nodes[id.0 as usize]
    }
}

impl Schemas {
    fn alloc(&mut self) -> SchemaId {
        self.nodes.push(Box::default());
        SchemaId(self.nodes.len() as u32 - 1)
    }

    fn set(&mut self, id: SchemaId, schema: JsonSchema) {
        *self.nodes[id.0 as usize] = schema;
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}
