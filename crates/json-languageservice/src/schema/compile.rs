use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

use super::graph::{Graph, ObjId, Val};
use super::*;
use crate::strings::extended_regexp;

/// Compiles a schema from JSON, without `$ref` resolution.
pub fn compile_json(value: &Value) -> (Schemas, SchemaId) {
    let mut graph = Graph::default();
    match graph.add_json(value) {
        Val::Obj(root) => compile(&graph, root),
        Val::Bool(false) => (Schemas::default(), SchemaId::FALSE),
        _ => (Schemas::default(), SchemaId::TRUE),
    }
}

/// Compiles the schema objects that are reachable from `root`.
pub(crate) fn compile(graph: &Graph, root: ObjId) -> (Schemas, SchemaId) {
    let mut c = Compiler {
        graph,
        schemas: Schemas::default(),
        ids: HashMap::new(),
        queue: Vec::new(),
        property_maps: HashMap::new(),
        pattern_maps: HashMap::new(),
    };
    let root = c.id_of(root);
    while let Some((obj, id)) = c.queue.pop() {
        let schema = c.compile_obj(obj);
        c.schemas.set(id, schema);
    }
    (c.schemas, root)
}

struct Compiler<'a> {
    graph: &'a Graph,
    schemas: Schemas,
    ids: HashMap<ObjId, SchemaId>,
    queue: Vec<(ObjId, SchemaId)>,
    property_maps: HashMap<ObjId, Arc<IndexMap<String, SchemaRef>>>,
    pattern_maps: HashMap<ObjId, Arc<Vec<PatternSchema>>>,
}

fn string(v: Option<&Val>) -> Option<String> {
    v.and_then(Val::as_str).map(str::to_owned)
}

/// A string that upstream tests for truthiness.
fn truthy_string(v: Option<&Val>) -> Option<String> {
    string(v).filter(|s| !s.is_empty())
}

fn number(v: Option<&Val>) -> Option<f64> {
    match v {
        Some(Val::Num(n)) => Some(*n),
        _ => None,
    }
}

fn boolean(v: Option<&Val>) -> Option<bool> {
    match v {
        Some(Val::Bool(b)) => Some(*b),
        _ => None,
    }
}

fn strings(v: Option<&Val>) -> Option<Vec<String>> {
    match v {
        Some(Val::Arr(items)) => Some(
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => None,
    }
}

fn optional_strings(v: Option<&Val>) -> Option<Vec<Option<String>>> {
    match v {
        Some(Val::Arr(items)) => Some(
            items
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => None,
    }
}

impl Compiler<'_> {
    fn id_of(&mut self, obj: ObjId) -> SchemaId {
        if let Some(&id) = self.ids.get(&obj) {
            return id;
        }
        let id = self.schemas.alloc();
        self.ids.insert(obj, id);
        self.queue.push((obj, id));
        id
    }

    fn schema_ref(&mut self, v: Option<&Val>) -> Option<SchemaRef> {
        match v {
            Some(Val::Bool(b)) => Some(SchemaRef::Bool(*b)),
            Some(Val::Obj(obj)) => Some(SchemaRef::Schema(self.id_of(*obj))),
            _ => None,
        }
    }

    fn schema_refs(&mut self, v: Option<&Val>) -> Option<Vec<SchemaRef>> {
        match v {
            Some(Val::Arr(items)) => Some(
                items
                    .iter()
                    .filter_map(|v| self.schema_ref(Some(v)))
                    .collect(),
            ),
            _ => None,
        }
    }

    fn schema_map(&mut self, map: ObjId) -> IndexMap<String, SchemaRef> {
        let graph = self.graph;
        graph[map]
            .iter()
            .filter_map(|(k, v)| Some((k.to_owned(), self.schema_ref(Some(v))?)))
            .collect()
    }

    fn values(&self, v: Option<&Val>) -> Option<Vec<Value>> {
        match v {
            Some(Val::Arr(items)) => Some(items.iter().map(|v| self.graph.to_json(v)).collect()),
            _ => None,
        }
    }

    fn compile_obj(&mut self, obj: ObjId) -> JsonSchema {
        let graph = self.graph;
        let o = &graph[obj];
        let get = |k: &str| o.get(k);

        let properties = match get("properties") {
            Some(Val::Obj(map)) => Some(match self.property_maps.get(map) {
                Some(m) => m.clone(),
                None => {
                    let m = Arc::new(self.schema_map(*map));
                    self.property_maps.insert(*map, m.clone());
                    m
                }
            }),
            _ => None,
        };
        let pattern_properties = match get("patternProperties") {
            Some(Val::Obj(map)) => Some(match self.pattern_maps.get(map) {
                Some(m) => m.clone(),
                None => {
                    let m: Vec<PatternSchema> = graph[*map]
                        .iter()
                        .filter_map(|(k, v)| {
                            Some(PatternSchema {
                                pattern: k.to_owned(),
                                regex: extended_regexp(k),
                                schema: self.schema_ref(Some(v))?,
                            })
                        })
                        .collect();
                    let m = Arc::new(m);
                    self.pattern_maps.insert(*map, m.clone());
                    m
                }
            }),
            _ => None,
        };
        let dependencies: Option<IndexMap<_, _>> = match get("dependencies") {
            Some(Val::Obj(map)) => Some(
                graph[*map]
                    .iter()
                    .filter_map(|(k, v)| {
                        let dep = match v {
                            Val::Arr(_) => Dependency::Required(strings(Some(v))?),
                            _ => Dependency::Schema(self.schema_ref(Some(v))?),
                        };
                        Some((k.to_owned(), dep))
                    })
                    .collect(),
            ),
            _ => None,
        };
        let items = match get("items") {
            Some(v @ Val::Arr(_)) => self.schema_refs(Some(v)).map(Items::Many),
            v => self.schema_ref(v).map(Items::One),
        };
        let exclusive = |v: Option<&Val>| match v {
            Some(Val::Bool(b)) => Some(Exclusive::Bool(*b)),
            Some(Val::Num(n)) => Some(Exclusive::Number(*n)),
            _ => None,
        };
        let type_ = match get("type") {
            Some(v @ Val::Arr(_)) => strings(Some(v)).map(SchemaType::Many),
            v => truthy_string(v).map(SchemaType::One),
        };
        let pattern = string(get("pattern"));
        let pattern_regex = pattern.as_deref().and_then(extended_regexp);
        let dependent_required: Option<IndexMap<_, _>> = match get("dependentRequired") {
            Some(Val::Obj(map)) => Some(
                graph[*map]
                    .iter()
                    .filter_map(|(k, v)| Some((k.to_owned(), strings(Some(v))?)))
                    .collect(),
            ),
            _ => None,
        };
        let dependent_schemas = match get("dependentSchemas") {
            Some(Val::Obj(map)) => Some(self.schema_map(*map)),
            _ => None,
        };
        let default_snippets = match get("defaultSnippets") {
            Some(Val::Arr(items)) => Some(
                items
                    .iter()
                    .filter_map(Val::as_obj)
                    .map(|s| {
                        let s = &graph[s];
                        DefaultSnippet {
                            label: string(s.get("label")),
                            description: string(s.get("description")),
                            markdown_description: string(s.get("markdownDescription")),
                            body: s.get("body").map(|b| graph.to_json(b)),
                            body_text: string(s.get("bodyText")),
                        }
                    })
                    .collect(),
            ),
            _ => None,
        };
        let dynamic_ref = o.dynamic_ref_info.and_then(|info| {
            let info = graph.info(info);
            Some((self.id_of(info.target?), info.name.clone()))
        });
        let dynamic_anchors: Option<IndexMap<_, _>> = o.anchor_maps.as_ref().map(|maps| {
            maps.dynamic
                .iter()
                .map(|(k, &v)| (k.clone(), self.id_of(v)))
                .collect()
        });

        let limits = Limits {
            min_properties: number(get("minProperties")),
            max_properties: number(get("maxProperties")),
            min_items: number(get("minItems")),
            max_items: number(get("maxItems")),
            min_length: number(get("minLength")),
            max_length: number(get("maxLength")),
            minimum: number(get("minimum")),
            maximum: number(get("maximum")),
            exclusive_minimum: exclusive(get("exclusiveMinimum")),
            exclusive_maximum: exclusive(get("exclusiveMaximum")),
            multiple_of: number(get("multipleOf")),
            min_contains: number(get("minContains")),
            max_contains: number(get("maxContains")),
        };
        let vscode = VsCodeKeywords {
            default_snippets,
            error_message: truthy_string(get("errorMessage")),
            pattern_error_message: truthy_string(get("patternErrorMessage")),
            deprecation_message: truthy_string(get("deprecationMessage")),
            enum_descriptions: optional_strings(get("enumDescriptions")),
            enum_sort_texts: optional_strings(get("enumSortTexts")),
            enum_details: optional_strings(get("enumDetails")),
            markdown_enum_descriptions: optional_strings(get("markdownEnumDescriptions")),
            suggest_sort_text: string(get("suggestSortText")),
            completion_detail: string(get("completionDetail")),
        };

        JsonSchema {
            id: truthy_string(get("id")),
            dollar_id: truthy_string(get("$id")),
            schema: string(get("$schema")),
            type_,
            title: string(get("title")),
            default: get("default").map(|v| Box::new(graph.to_json(v))),
            description: string(get("description")),
            properties,
            pattern_properties,
            additional_properties: self.schema_ref(get("additionalProperties")),
            dependencies: dependencies.map(Box::new),
            items,
            unique_items: boolean(get("uniqueItems")) == Some(true),
            additional_items: self.schema_ref(get("additionalItems")),
            pattern,
            pattern_regex: pattern_regex.map(Box::new),
            required: strings(get("required")),
            any_of: self.schema_refs(get("anyOf")),
            all_of: self.schema_refs(get("allOf")),
            one_of: self.schema_refs(get("oneOf")),
            not: self.schema_ref(get("not")),
            enum_: self.values(get("enum")).map(Arc::new),
            format: truthy_string(get("format")),
            const_: get("const").map(|v| Box::new(graph.to_json(v))),
            contains: self.schema_ref(get("contains")),
            property_names: self.schema_ref(get("propertyNames")),
            examples: self.values(get("examples")),
            if_: self.schema_ref(get("if")),
            then: self.schema_ref(get("then")),
            else_: self.schema_ref(get("else")),
            unevaluated_properties: self.schema_ref(get("unevaluatedProperties")),
            unevaluated_items: self.schema_ref(get("unevaluatedItems")),
            deprecated: get("deprecated").is_some_and(Val::is_truthy),
            dependent_required: dependent_required.map(Box::new),
            dependent_schemas: dependent_schemas.map(Box::new),
            recursive_ref: get("$recursiveRef").is_some_and(Val::is_truthy),
            recursive_anchor: boolean(get("$recursiveAnchor")) == Some(true),
            prefix_items: self.schema_refs(get("prefixItems")),
            dynamic_anchor: string(get("$dynamicAnchor")),
            limits: (limits != Limits::default()).then(|| Box::new(limits)),
            vscode: (vscode != VsCodeKeywords::default()).then(|| Box::new(vscode)),
            markdown_description: string(get("markdownDescription")),
            do_not_suggest: get("doNotSuggest").is_some_and(Val::is_truthy),
            allow_comments: boolean(get("allowComments")),
            allow_trailing_commas: boolean(get("allowTrailingCommas")),
            allows_trailing_commas: boolean(get("allowsTrailingCommas")),
            original_id: o.original_id.clone().filter(|s| !s.is_empty()),
            dynamic_ref: dynamic_ref.map(Box::new),
            dynamic_anchors: dynamic_anchors.map(Box::new),
        }
    }
}
