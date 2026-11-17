use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

use super::doc::Doc;
use super::tree::{Kind, Node, Tree};
use super::*;
use crate::strings::extended_regexp;

/// The keys that do not make a `$ref` sibling.
const IGNORED_REF_SIBLINGS: [&str; 5] = ["$ref", "$schema", "$id", "id", "$comment"];

/// The `$ref` of a node that is only a reference: it has no sibling keywords, or its
/// document is before draft 2019-09, where siblings are ignored.
pub(crate) fn pure_ref(doc: &Doc, n: Node) -> Option<&str> {
    let tree = &doc.tree;
    let r = tree.str(tree.get_truthy(n, "$ref")?)?;
    let pre_201909 = doc.draft.is_some_and(|d| d < SchemaDraft::V2019_09);
    (pre_201909
        || tree
            .entries(n)
            .all(|(k, _)| IGNORED_REF_SIBLINGS.contains(&k)))
    .then_some(r)
}

/// Keywords that do not change validation.
const ANNOTATIONS: [&str; 20] = [
    "title",
    "description",
    "markdownDescription",
    "default",
    "examples",
    "deprecated",
    "deprecationMessage",
    "doNotSuggest",
    "defaultSnippets",
    "enumDescriptions",
    "markdownEnumDescriptions",
    "enumSortTexts",
    "enumDetails",
    "errorMessage",
    "patternErrorMessage",
    "suggestSortText",
    "completionDetail",
    "readOnly",
    "writeOnly",
    "$comment",
];

fn only_annotation_siblings(tree: &Tree, n: Node) -> bool {
    tree.entries(n)
        .all(|(k, _)| IGNORED_REF_SIBLINGS.contains(&k) || ANNOTATIONS.contains(&k))
}

/// Compiles node `n` of document `d`.
pub(crate) fn compile(set: &Schemas, d: usize, n: Node) -> JsonSchema {
    let doc = set.doc(d);
    let c = Compiler {
        set,
        d,
        tree: &doc.tree,
    };
    let mut schema = c.compile(n);
    if let Some(r) = c.tree.get_truthy(n, "$ref").and_then(|r| c.tree.str(r)) {
        let target = set.resolve_ref(d, n, r, false);
        if pure_ref(doc, n).is_some() {
            // Only reached for a root or an anchor target; references to it are resolved.
            schema = JsonSchema {
                id: schema.id,
                dollar_id: schema.dollar_id,
                schema: schema.schema,
                ..JsonSchema::default()
            };
        } else if let SchemaRef::Schema(t) = target
            && t > SchemaId::FALSE
        {
            if only_annotation_siblings(&doc.tree, n)
                && let Some(merged) = c.merge_annotations(&schema, t)
            {
                return merged;
            }
            if schema.additional_properties.is_some() {
                c.add_target_properties(&mut schema, t);
            }
        }
        schema.all_of.get_or_insert_default().push(target);
    }
    if doc.draft.is_none_or(|x| x >= SchemaDraft::V2020_12)
        && let Some(r) = c
            .tree
            .get_truthy(n, "$dynamicRef")
            .and_then(|r| c.tree.str(r))
    {
        let (_, fragment) = split_ref(r);
        let name = fragment.filter(|f| !f.is_empty() && !f.starts_with('/'));
        let initial = set.resolve_ref(d, n, r, false).as_schema();
        schema.dynamic_ref = Some(Box::new((initial, name.map(str::to_owned))));
    }
    schema.resource = Some(set.id(d, doc.resource_root(n)));
    if n == 0 && schema.dollar_id.is_none() && schema.id.is_none() {
        schema.original_id = Some(doc.uri.clone());
    }
    if let Some((_, anchors)) = doc.dynamic_anchors.iter().find(|(root, _)| *root == n) {
        schema.dynamic_anchors = Some(Box::new(
            anchors
                .iter()
                .map(|(k, &v)| (k.clone(), set.id(d, v)))
                .collect(),
        ));
    }
    schema
}

struct Compiler<'a> {
    set: &'a Schemas,
    d: usize,
    tree: &'a Tree,
}

impl Compiler<'_> {
    fn string(&self, v: Option<Node>) -> Option<String> {
        self.tree.str(v?).map(str::to_owned)
    }

    /// A string that upstream tests for truthiness.
    fn truthy_string(&self, v: Option<Node>) -> Option<String> {
        self.string(v).filter(|s| !s.is_empty())
    }

    fn number(&self, v: Option<Node>) -> Option<f64> {
        match self.tree.kind(v?) {
            Kind::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn boolean(&self, v: Option<Node>) -> Option<bool> {
        match self.tree.kind(v?) {
            Kind::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn is_truthy(&self, v: Option<Node>) -> bool {
        v.is_some_and(|v| self.tree.is_truthy(v))
    }

    fn strings(&self, v: Option<Node>) -> Option<Vec<String>> {
        let v = v.filter(|&v| matches!(self.tree.kind(v), Kind::Arr(..)))?;
        Some(
            self.tree
                .items(v)
                .iter()
                .filter_map(|&i| self.string(Some(i)))
                .collect(),
        )
    }

    fn optional_strings(&self, v: Option<Node>) -> Option<Vec<Option<String>>> {
        let v = v.filter(|&v| matches!(self.tree.kind(v), Kind::Arr(..)))?;
        Some(
            self.tree
                .items(v)
                .iter()
                .map(|&i| self.string(Some(i)))
                .collect(),
        )
    }

    fn values(&self, v: Option<Node>) -> Option<Vec<Value>> {
        let v = v.filter(|&v| matches!(self.tree.kind(v), Kind::Arr(..)))?;
        Some(
            self.tree
                .items(v)
                .iter()
                .map(|&i| self.tree.to_json(i))
                .collect(),
        )
    }

    fn json(&self, v: Option<Node>) -> Option<Box<Value>> {
        v.map(|v| Box::new(self.tree.to_json(v)))
    }

    fn schema_ref(&self, v: Option<Node>) -> Option<SchemaRef> {
        let v = v?;
        match self.tree.kind(v) {
            Kind::Bool(b) => Some(SchemaRef::Bool(*b)),
            Kind::Obj(..) => Some(self.set.node_ref(self.d, v, true)),
            _ => None,
        }
    }

    fn schema_refs(&self, v: Option<Node>) -> Option<Vec<SchemaRef>> {
        let v = v.filter(|&v| matches!(self.tree.kind(v), Kind::Arr(..)))?;
        Some(
            self.tree
                .items(v)
                .iter()
                .filter_map(|&i| self.schema_ref(Some(i)))
                .collect(),
        )
    }

    fn obj(&self, v: Option<Node>) -> Option<Node> {
        v.filter(|&v| self.tree.is_obj(v))
    }

    fn schema_map(&self, map: Node) -> IndexMap<String, SchemaRef> {
        self.tree
            .entries(map)
            .filter_map(|(k, v)| Some((k.to_owned(), self.schema_ref(Some(v))?)))
            .collect()
    }

    /// `{ "$ref": ..., "description": ... }` is the target with the annotations that the
    /// target does not have, as the upstream merge gives it. Completion and hover read the
    /// annotations of the schema that has `const` or `enum`. `None` if the target has a
    /// `$ref` itself.
    fn merge_annotations(&self, own: &JsonSchema, target: SchemaId) -> Option<JsonSchema> {
        let (d, n) = self.set.locate(target);
        let tree = &self.set.doc(d).tree;
        if tree.get(n, "$ref").is_some() || tree.get(n, "$dynamicRef").is_some() {
            return None;
        }
        let c = Compiler {
            set: self.set,
            d,
            tree,
        };
        let mut merged = c.compile(n);
        merged.id = own.id.clone();
        merged.dollar_id = own.dollar_id.clone();
        merged.resource = Some(self.set.id(d, self.set.doc(d).resource_root(n)));
        merged.title = merged.title.or_else(|| own.title.clone());
        merged.description = merged.description.or_else(|| own.description.clone());
        merged.markdown_description = merged
            .markdown_description
            .or_else(|| own.markdown_description.clone());
        merged.default = merged.default.or_else(|| own.default.clone());
        merged.examples = merged.examples.or_else(|| own.examples.clone());
        merged.deprecated |= own.deprecated;
        merged.do_not_suggest |= own.do_not_suggest;
        if let Some(theirs) = &own.vscode {
            let ours = merged.vscode.get_or_insert_default();
            macro_rules! fill {
                ($($f:ident),*) => {$(
                    if ours.$f.is_none() {
                        ours.$f = theirs.$f.clone();
                    }
                )*};
            }
            fill!(
                default_snippets,
                error_message,
                pattern_error_message,
                deprecation_message,
                enum_descriptions,
                enum_sort_texts,
                enum_details,
                markdown_enum_descriptions,
                suggest_sort_text,
                completion_detail
            );
        }
        Some(merged)
    }

    /// `{ "$ref": ..., "additionalProperties": false }` accepts the properties of the
    /// target, as upstream does: it merges the target into the schema. VS Code settings
    /// schemas use this form. Not if the target has its own `additionalProperties`, where
    /// upstream keeps the two apart.
    fn add_target_properties(&self, schema: &mut JsonSchema, target: SchemaId) {
        let (d, n) = self.set.locate(target);
        let c = Compiler {
            set: self.set,
            d,
            tree: &self.set.doc(d).tree,
        };
        if c.tree.get(n, "additionalProperties").is_some() {
            return;
        }
        let target = c.compile(n);
        if let Some(theirs) = target.properties {
            let mut merged = schema.properties.as_deref().cloned().unwrap_or_default();
            merged.extend(theirs.iter().map(|(k, v)| (k.clone(), *v)));
            schema.properties = Some(Arc::new(merged));
        }
        if let Some(theirs) = target.pattern_properties {
            match &mut schema.pattern_properties {
                Some(ours) => {
                    let ours = Arc::get_mut(ours).expect("a new schema is not shared");
                    ours.extend(theirs.iter().map(|p| PatternSchema {
                        pattern: p.pattern.clone(),
                        regex: extended_regexp(&p.pattern),
                        schema: p.schema,
                    }));
                }
                None => schema.pattern_properties = Some(theirs),
            }
        }
    }

    fn compile(&self, n: Node) -> JsonSchema {
        let tree = self.tree;
        let get = |k: &str| tree.get(n, k);

        let properties = self
            .obj(get("properties"))
            .map(|map| Arc::new(self.schema_map(map)));
        let pattern_properties = self.obj(get("patternProperties")).map(|map| {
            Arc::new(
                tree.entries(map)
                    .filter_map(|(k, v)| {
                        Some(PatternSchema {
                            pattern: k.to_owned(),
                            regex: extended_regexp(k),
                            schema: self.schema_ref(Some(v))?,
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        });
        let dependencies: Option<IndexMap<_, _>> = self.obj(get("dependencies")).map(|map| {
            tree.entries(map)
                .filter_map(|(k, v)| {
                    let dep = match tree.kind(v) {
                        Kind::Arr(..) => Dependency::Required(self.strings(Some(v))?),
                        _ => Dependency::Schema(self.schema_ref(Some(v))?),
                    };
                    Some((k.to_owned(), dep))
                })
                .collect()
        });
        let items = match get("items") {
            Some(v) if matches!(tree.kind(v), Kind::Arr(..)) => {
                self.schema_refs(Some(v)).map(Items::Many)
            }
            v => self.schema_ref(v).map(Items::One),
        };
        let exclusive = |v: Option<Node>| match tree.kind(v?) {
            Kind::Bool(b) => Some(Exclusive::Bool(*b)),
            Kind::Num(n) => Some(Exclusive::Number(*n)),
            _ => None,
        };
        let type_ = match get("type") {
            Some(v) if matches!(tree.kind(v), Kind::Arr(..)) => {
                self.strings(Some(v)).map(SchemaType::Many)
            }
            v => self.truthy_string(v).map(SchemaType::One),
        };
        let pattern = self.string(get("pattern"));
        let pattern_regex = pattern.as_deref().and_then(extended_regexp);
        let dependent_required: Option<IndexMap<_, _>> =
            self.obj(get("dependentRequired")).map(|map| {
                tree.entries(map)
                    .filter_map(|(k, v)| Some((k.to_owned(), self.strings(Some(v))?)))
                    .collect()
            });
        let dependent_schemas = self
            .obj(get("dependentSchemas"))
            .map(|map| self.schema_map(map));
        let default_snippets = get("defaultSnippets")
            .filter(|&v| matches!(tree.kind(v), Kind::Arr(..)))
            .map(|v| {
                tree.items(v)
                    .iter()
                    .filter(|&&s| tree.is_obj(s))
                    .map(|&s| DefaultSnippet {
                        label: self.string(tree.get(s, "label")),
                        description: self.string(tree.get(s, "description")),
                        markdown_description: self.string(tree.get(s, "markdownDescription")),
                        body: tree.get(s, "body").map(|b| tree.to_json(b)),
                        body_text: self.string(tree.get(s, "bodyText")),
                    })
                    .collect()
            });

        let limits = Limits {
            min_properties: self.number(get("minProperties")),
            max_properties: self.number(get("maxProperties")),
            min_items: self.number(get("minItems")),
            max_items: self.number(get("maxItems")),
            min_length: self.number(get("minLength")),
            max_length: self.number(get("maxLength")),
            minimum: self.number(get("minimum")),
            maximum: self.number(get("maximum")),
            exclusive_minimum: exclusive(get("exclusiveMinimum")),
            exclusive_maximum: exclusive(get("exclusiveMaximum")),
            multiple_of: self.number(get("multipleOf")),
            min_contains: self.number(get("minContains")),
            max_contains: self.number(get("maxContains")),
        };
        let vscode = VsCodeKeywords {
            default_snippets,
            error_message: self.truthy_string(get("errorMessage")),
            pattern_error_message: self.truthy_string(get("patternErrorMessage")),
            deprecation_message: self.truthy_string(get("deprecationMessage")),
            enum_descriptions: self.optional_strings(get("enumDescriptions")),
            enum_sort_texts: self.optional_strings(get("enumSortTexts")),
            enum_details: self.optional_strings(get("enumDetails")),
            markdown_enum_descriptions: self.optional_strings(get("markdownEnumDescriptions")),
            suggest_sort_text: self.string(get("suggestSortText")),
            completion_detail: self.string(get("completionDetail")),
        };

        JsonSchema {
            id: self.truthy_string(get("id")),
            dollar_id: self.truthy_string(get("$id")),
            schema: self.string(get("$schema")),
            type_,
            title: self.string(get("title")),
            default: self.json(get("default")),
            description: self.string(get("description")),
            properties,
            pattern_properties,
            additional_properties: self.schema_ref(get("additionalProperties")),
            dependencies: dependencies.map(Box::new),
            items,
            unique_items: self.boolean(get("uniqueItems")) == Some(true),
            additional_items: self.schema_ref(get("additionalItems")),
            pattern,
            pattern_regex: pattern_regex.map(Box::new),
            required: self.strings(get("required")),
            any_of: self.schema_refs(get("anyOf")),
            all_of: self.schema_refs(get("allOf")),
            one_of: self.schema_refs(get("oneOf")),
            not: self.schema_ref(get("not")),
            enum_: self.values(get("enum")).map(Arc::new),
            format: self.truthy_string(get("format")),
            const_: self.json(get("const")),
            contains: self.schema_ref(get("contains")),
            property_names: self.schema_ref(get("propertyNames")),
            examples: self.values(get("examples")),
            if_: self.schema_ref(get("if")),
            then: self.schema_ref(get("then")),
            else_: self.schema_ref(get("else")),
            unevaluated_properties: self.schema_ref(get("unevaluatedProperties")),
            unevaluated_items: self.schema_ref(get("unevaluatedItems")),
            deprecated: self.is_truthy(get("deprecated")),
            dependent_required: dependent_required.map(Box::new),
            dependent_schemas: dependent_schemas.map(Box::new),
            recursive_ref: self.is_truthy(get("$recursiveRef")),
            recursive_anchor: self.boolean(get("$recursiveAnchor")) == Some(true),
            prefix_items: self.schema_refs(get("prefixItems")),
            dynamic_anchor: self.string(get("$dynamicAnchor")),
            limits: (limits != Limits::default()).then(|| Box::new(limits)),
            vscode: (vscode != VsCodeKeywords::default()).then(|| Box::new(vscode)),
            markdown_description: self.string(get("markdownDescription")),
            do_not_suggest: self.is_truthy(get("doNotSuggest")),
            allow_comments: self.boolean(get("allowComments")),
            allow_trailing_commas: self.boolean(get("allowTrailingCommas")),
            allows_trailing_commas: self.boolean(get("allowsTrailingCommas")),
            original_id: None,
            dynamic_ref: None,
            resource: None,
            dynamic_anchors: None,
        }
    }
}
