//! The validator from `parser/jsonParser.ts`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
use regress::Regex;
use serde_json::Value;

use crate::schema::{Dependency, Exclusive, Items, SchemaId, SchemaRef, SchemaType, Schemas};
use crate::strings::{js_equals, js_number_to_string, js_stringify};
use crate::vocabularies::{Vocabularies, is_format_assertion_enabled, is_keyword_enabled};
use crate::{AstKind, ErrorCode, JsonDocument, NodeId, SchemaDraft, TextDocument};

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub offset: usize,
    pub length: usize,
    pub severity: Option<DiagnosticSeverity>,
    pub code: Option<ErrorCode>,
    pub message: String,
}

/// `IApplicableSchema`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicableSchema {
    pub node: NodeId,
    pub inverted: bool,
    pub schema: SchemaId,
}

/// `SchemaCollector` and, if `enabled` is false, `NoOpSchemaCollector`.
struct Collector {
    enabled: bool,
    focus_offset: Option<usize>,
    exclude: Option<NodeId>,
    schemas: Vec<ApplicableSchema>,
}

impl Collector {
    fn noop() -> Self {
        Collector {
            enabled: false,
            focus_offset: None,
            exclude: None,
            schemas: Vec::new(),
        }
    }

    fn add(&mut self, schema: ApplicableSchema) {
        if self.enabled {
            self.schemas.push(schema);
        }
    }

    fn merge(&mut self, other: Collector) {
        if self.enabled {
            self.schemas.extend(other.schemas);
        }
    }

    fn include(&self, doc: &JsonDocument, node: NodeId) -> bool {
        !self.enabled
            || (self
                .focus_offset
                .is_none_or(|o| doc[node].contains(o, false))
                && Some(node) != self.exclude)
    }

    fn new_sub(&self) -> Collector {
        Collector {
            enabled: self.enabled,
            focus_offset: None,
            exclude: self.exclude,
            schemas: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ValidationResult {
    pub problems: Vec<Problem>,
    pub properties_matches: i32,
    pub processed_properties: HashSet<String>,
    pub properties_value_matches: i32,
    pub primary_value_matches: i32,
    pub enum_value_match: bool,
    pub enum_values: Option<Arc<Vec<Value>>>,
}

impl ValidationResult {
    /// A deprecation problem does not count as a failure.
    pub fn has_problems(&self) -> bool {
        self.problems
            .iter()
            .any(|p| p.code != Some(ErrorCode::DEPRECATED))
    }

    fn merge(&mut self, other: ValidationResult) {
        self.problems.extend(other.problems);
        self.properties_matches += other.properties_matches;
        self.properties_value_matches += other.properties_value_matches;
        self.processed_properties.extend(other.processed_properties);
    }

    fn merge_enum_values(&mut self, other: &ValidationResult) {
        if !self.enum_value_match
            && !other.enum_value_match
            && let (Some(a), Some(b)) = (&self.enum_values, &other.enum_values)
        {
            let mut values = a.as_ref().clone();
            values.extend(b.iter().cloned());
            self.enum_values = Some(Arc::new(values));
        }
    }

    fn update_enum_mismatch_problem_messages(&mut self) {
        if let (false, Some(values)) = (self.enum_value_match, &self.enum_values) {
            let message = format!(
                "Value is not accepted. Valid values: {}.",
                join_values(values)
            );
            for p in &mut self.problems {
                if p.code == Some(ErrorCode::ENUM_VALUE_MISMATCH) {
                    p.message = message.clone();
                }
            }
        }
    }

    fn merge_property_match(&mut self, other: ValidationResult) {
        self.properties_matches += 1;
        if other.enum_value_match || (!other.has_problems() && other.properties_matches != 0) {
            self.properties_value_matches += 1;
        }
        if other.enum_value_match && other.enum_values.as_ref().is_some_and(|v| v.len() == 1) {
            self.primary_value_matches += 1;
        }
        self.problems.extend(other.problems);
    }

    fn compare(&self, other: &ValidationResult) -> i32 {
        let has_problems = self.has_problems();
        if has_problems != other.has_problems() {
            return if has_problems { -1 } else { 1 };
        }
        if self.enum_value_match != other.enum_value_match {
            return if other.enum_value_match { -1 } else { 1 };
        }
        if self.primary_value_matches != other.primary_value_matches {
            return self.primary_value_matches - other.primary_value_matches;
        }
        if self.properties_value_matches != other.properties_value_matches {
            return self.properties_value_matches - other.properties_value_matches;
        }
        self.properties_matches - other.properties_matches
    }
}

fn join_values(values: &[Value]) -> String {
    values
        .iter()
        .map(js_stringify)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `normalizeId` and the draft table of `getSchemaDraftFromId`.
pub fn get_schema_draft_from_id(schema_id: &str) -> Option<SchemaDraft> {
    match crate::schema_service::normalize_id(schema_id).as_str() {
        "https://json-schema.org/draft-03/schema" => Some(SchemaDraft::V3),
        "https://json-schema.org/draft-04/schema" => Some(SchemaDraft::V4),
        "https://json-schema.org/draft-06/schema" => Some(SchemaDraft::V6),
        "https://json-schema.org/draft-07/schema" => Some(SchemaDraft::V7),
        "https://json-schema.org/draft/2019-09/schema" => Some(SchemaDraft::V2019_09),
        "https://json-schema.org/draft/2020-12/schema" => Some(SchemaDraft::V2020_12),
        _ => None,
    }
}

struct Context<'a> {
    doc: &'a JsonDocument,
    schemas: &'a Schemas,
    draft: SchemaDraft,
    explicit_draft: Option<SchemaDraft>,
    vocabularies: Option<&'a Vocabularies>,
    stack: Vec<SchemaId>,
    roots: Vec<SchemaId>,
}

fn resolve_drafts(
    schemas: &Schemas,
    root: SchemaId,
    draft_override: Option<SchemaDraft>,
) -> (Option<SchemaDraft>, SchemaDraft) {
    let explicit = draft_override.or_else(|| {
        schemas[root]
            .schema
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(get_schema_draft_from_id)
    });
    (explicit, explicit.unwrap_or(SchemaDraft::V2020_12))
}

impl JsonDocument {
    /// `JSONDocument.validate`.
    pub fn validate(
        &self,
        text_document: &TextDocument,
        schemas: &Schemas,
        schema: SchemaId,
        severity: DiagnosticSeverity,
        schema_draft: Option<SchemaDraft>,
        vocabularies: Option<&Vocabularies>,
    ) -> Option<Vec<Diagnostic>> {
        let root = self.root()?;
        let mut result = ValidationResult::default();
        let (explicit, effective) = resolve_drafts(schemas, schema, schema_draft);
        let mut cx = Context {
            doc: self,
            schemas,
            draft: effective,
            explicit_draft: explicit,
            vocabularies,
            stack: Vec::new(),
            roots: vec![schema],
        };
        cx.validate(root, schema, &mut result, &mut Collector::noop());
        Some(
            result
                .problems
                .into_iter()
                .map(|p| Diagnostic {
                    range: text_document.range(p.offset, p.offset + p.length),
                    severity: Some(p.severity.unwrap_or(severity)),
                    code: p.code.map(|c| NumberOrString::Number(c.0 as i32)),
                    message: p.message,
                    ..Diagnostic::default()
                })
                .collect(),
        )
    }

    /// `JSONDocument.getMatchingSchemas`.
    pub fn get_matching_schemas(
        &self,
        schemas: &Schemas,
        schema: SchemaId,
        focus_offset: Option<usize>,
        exclude: Option<NodeId>,
        vocabularies: Option<&Vocabularies>,
    ) -> Vec<ApplicableSchema> {
        let Some(root) = self.root() else {
            return Vec::new();
        };
        let mut collector = Collector {
            enabled: true,
            focus_offset,
            exclude,
            schemas: Vec::new(),
        };
        let (explicit, effective) = resolve_drafts(schemas, schema, None);
        let mut cx = Context {
            doc: self,
            schemas,
            draft: effective,
            explicit_draft: explicit,
            vocabularies,
            stack: Vec::new(),
            roots: vec![schema],
        };
        cx.validate(
            root,
            schema,
            &mut ValidationResult::default(),
            &mut collector,
        );
        collector.schemas
    }
}

const KEYWORDS_201909: [&str; 6] = [
    "dependentRequired",
    "dependentSchemas",
    "unevaluatedProperties",
    "unevaluatedItems",
    "minContains",
    "maxContains",
];

fn fmt_num(n: f64) -> String {
    js_number_to_string(n)
}

impl Context<'_> {
    fn enabled(&self, keyword: &str) -> bool {
        if keyword == "dependencies" {
            return self.draft <= SchemaDraft::V7;
        }
        if KEYWORDS_201909.contains(&keyword) {
            return self.draft >= SchemaDraft::V2019_09;
        }
        if keyword == "prefixItems" {
            return self.draft >= SchemaDraft::V2020_12;
        }
        if self.draft >= SchemaDraft::V2019_09 && self.vocabularies.is_some() {
            return is_keyword_enabled(keyword, self.vocabularies);
        }
        true
    }

    fn validate(
        &mut self,
        n: NodeId,
        schema_id: SchemaId,
        result: &mut ValidationResult,
        collector: &mut Collector,
    ) {
        let doc = self.doc;
        if !collector.include(doc, n) {
            return;
        }
        if let AstKind::Property { .. } = doc[n].kind {
            if let Some(value) = doc.value(n) {
                self.validate(value, schema_id, result, collector);
            }
            return;
        }
        let node = n;
        let schemas = self.schemas;
        let schema = &schemas[schema_id];

        if schema.recursive_ref {
            let current_root = self
                .stack
                .iter()
                .rev()
                .copied()
                .find(|&s| schemas[s].is_resource_root());
            let target = match current_root {
                Some(r) if schemas[r].recursive_anchor => self
                    .stack
                    .iter()
                    .copied()
                    .find(|&s| schemas[s].recursive_anchor),
                Some(r) => Some(r),
                None => self.roots.first().copied(),
            };
            if let Some(target) = target.filter(|&t| t != schema_id) {
                self.validate(node, target, result, collector);
            }
        }

        let resource = schema.resource.unwrap_or(schema_id);
        let is_new_root = schema.resource.is_some() && !self.roots.contains(&resource);
        if is_new_root {
            self.roots.push(resource);
        }
        self.stack.push(schema_id);

        if let Some((initial, name)) = schema.dynamic_ref.as_deref() {
            let mut target = *initial;
            if let Some(name) = name
                && schemas[*initial].dynamic_anchor.as_ref() == Some(name)
                && let Some(candidate) = self.roots.iter().find_map(|&r| {
                    schemas[r]
                        .dynamic_anchors
                        .as_ref()
                        .and_then(|m| m.get(name).copied())
                })
            {
                target = candidate;
            }
            if target != schema_id {
                self.validate(node, target, result, collector);
            }
        }

        self.validate_node(node, schema_id, result, collector);
        match doc[node].kind {
            AstKind::Object => self.validate_object(node, schema_id, result, collector),
            AstKind::Array => self.validate_array(node, schema_id, result, collector),
            AstKind::String(_) => self.validate_string(node, schema_id, result),
            AstKind::Number { .. } => self.validate_number(node, schema_id, result),
            _ => {}
        }
        collector.add(ApplicableSchema {
            node,
            inverted: false,
            schema: schema_id,
        });

        self.stack.pop();
        if is_new_root {
            self.roots.pop();
        }
    }

    fn problem(&self, node: NodeId, message: String) -> Problem {
        let n = &self.doc[node];
        Problem {
            offset: n.offset(),
            length: n.length(),
            severity: None,
            code: None,
            message,
        }
    }

    fn validate_node(
        &mut self,
        node: NodeId,
        schema_id: SchemaId,
        result: &mut ValidationResult,
        collector: &mut Collector,
    ) {
        let doc = self.doc;
        let schemas = self.schemas;
        let schema = &schemas[schema_id];
        let matches_type = |t: &str| {
            let kind = &doc[node].kind;
            doc[node].node_type().as_str() == t
                || (t == "integer"
                    && matches!(
                        kind,
                        AstKind::Number {
                            is_integer: true,
                            ..
                        }
                    ))
        };

        if let Some(type_) = &schema.type_
            && self.enabled("type")
        {
            match type_ {
                SchemaType::Many(types) => {
                    if !types.iter().any(|t| matches_type(t)) {
                        let message = schema.vscode().error_message.clone().unwrap_or_else(|| {
                            format!("Incorrect type. Expected one of {}.", types.join(", "))
                        });
                        result.problems.push(self.problem(node, message));
                    }
                }
                SchemaType::One(t) => {
                    if !matches_type(t) {
                        let message = schema
                            .vscode()
                            .error_message
                            .clone()
                            .unwrap_or_else(|| format!("Incorrect type. Expected \"{t}\"."));
                        result.problems.push(self.problem(node, message));
                    }
                }
            }
        }
        if let Some(all_of) = &schema.all_of
            && self.enabled("allOf")
        {
            for sub in all_of {
                let mut sub_result = ValidationResult::default();
                let mut sub_collector = collector.new_sub();
                self.validate(node, sub.as_schema(), &mut sub_result, &mut sub_collector);
                result.merge(sub_result);
                collector.merge(sub_collector);
            }
        }
        if let Some(not) = schema.not
            && self.enabled("not")
        {
            let mut sub_result = ValidationResult::default();
            let mut sub_collector = collector.new_sub();
            self.validate(node, not.as_schema(), &mut sub_result, &mut sub_collector);
            if !sub_result.has_problems() {
                let message = schema
                    .vscode()
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "Matches a schema that is not allowed.".to_owned());
                result.problems.push(self.problem(node, message));
            }
            for mut ms in sub_collector.schemas {
                ms.inverted = !ms.inverted;
                collector.add(ms);
            }
        }
        if let Some(any_of) = &schema.any_of
            && self.enabled("anyOf")
        {
            self.test_alternatives(node, any_of, false, result, collector);
        }
        if let Some(one_of) = &schema.one_of
            && self.enabled("oneOf")
        {
            self.test_alternatives(node, one_of, true, result, collector);
        }
        if let Some(if_) = schema.if_
            && self.enabled("if")
        {
            let mut sub_result = ValidationResult::default();
            let mut sub_collector = collector.new_sub();
            self.validate(node, if_.as_schema(), &mut sub_result, &mut sub_collector);
            collector.merge(sub_collector);
            let branch = if !sub_result.has_problems() {
                result
                    .processed_properties
                    .extend(sub_result.processed_properties);
                schema.then
            } else {
                schema.else_
            };
            if let Some(branch) = branch {
                let mut sub_result = ValidationResult::default();
                let mut sub_collector = collector.new_sub();
                self.validate(
                    node,
                    branch.as_schema(),
                    &mut sub_result,
                    &mut sub_collector,
                );
                result.merge(sub_result);
                collector.merge(sub_collector);
            }
        }
        if let Some(values) = &schema.enum_
            && self.enabled("enum")
        {
            let val = doc.node_value(node);
            let enum_value_match = values.iter().any(|e| js_equals(&val, e));
            result.enum_values = Some(values.clone());
            result.enum_value_match = enum_value_match;
            if !enum_value_match {
                let message = schema.vscode().error_message.clone().unwrap_or_else(|| {
                    format!(
                        "Value is not accepted. Valid values: {}.",
                        join_values(values)
                    )
                });
                result.problems.push(Problem {
                    code: Some(ErrorCode::ENUM_VALUE_MISMATCH),
                    ..self.problem(node, message)
                });
            }
        }
        if let Some(const_) = &schema.const_
            && self.enabled("const")
        {
            let val = doc.node_value(node);
            if !js_equals(&val, const_) {
                let message = schema
                    .vscode()
                    .error_message
                    .clone()
                    .unwrap_or_else(|| format!("Value must be {}.", js_stringify(const_)));
                result.problems.push(Problem {
                    code: Some(ErrorCode::ENUM_VALUE_MISMATCH),
                    ..self.problem(node, message)
                });
                result.enum_value_match = false;
            } else {
                result.enum_value_match = true;
            }
            result.enum_values = Some(Arc::new(vec![(**const_).clone()]));
        }
        if schema.vscode().deprecation_message.is_some() || schema.deprecated {
            let message = schema
                .vscode()
                .deprecation_message
                .clone()
                .unwrap_or_else(|| "Value is deprecated".to_owned());
            let target = match doc[node].parent {
                Some(p) if matches!(doc[p].kind, AstKind::Property { .. }) => p,
                _ => node,
            };
            result.problems.push(Problem {
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(ErrorCode::DEPRECATED),
                ..self.problem(target, message)
            });
        }
    }

    fn test_alternatives(
        &mut self,
        node: NodeId,
        alternatives: &[SchemaRef],
        max_one_match: bool,
        result: &mut ValidationResult,
        collector: &mut Collector,
    ) {
        let to_test = self
            .try_discriminator_optimization(node, alternatives)
            .unwrap_or_else(|| alternatives.to_vec());
        let mut matches = 0;
        let mut best: Option<(ValidationResult, Collector)> = None;
        for sub in to_test {
            let mut sub_result = ValidationResult::default();
            let mut sub_collector = collector.new_sub();
            self.validate(node, sub.as_schema(), &mut sub_result, &mut sub_collector);
            if !sub_result.has_problems() {
                matches += 1;
            }
            match &mut best {
                None => best = Some((sub_result, sub_collector)),
                Some((best_result, best_collector)) => {
                    if !max_one_match && !sub_result.has_problems() && !best_result.has_problems() {
                        best_collector.merge(sub_collector);
                        best_result.properties_matches += sub_result.properties_matches;
                        best_result.properties_value_matches += sub_result.properties_value_matches;
                        best_result
                            .processed_properties
                            .extend(sub_result.processed_properties);
                    } else {
                        let compare = sub_result.compare(best_result);
                        if compare > 0 {
                            best = Some((sub_result, sub_collector));
                        } else if compare == 0 {
                            best_collector.merge(sub_collector);
                            best_result.merge_enum_values(&sub_result);
                        }
                    }
                }
            }
        }
        if matches > 1 && max_one_match {
            result.problems.push(Problem {
                length: 1,
                ..self.problem(
                    node,
                    "Matches multiple schemas when only one must validate.".to_owned(),
                )
            });
        }
        if let Some((mut best_result, best_collector)) = best {
            best_result.update_enum_mismatch_problem_messages();
            result.merge(best_result);
            collector.merge(best_collector);
        }
    }

    fn try_discriminator_optimization(
        &self,
        node: NodeId,
        alternatives: &[SchemaRef],
    ) -> Option<Vec<SchemaRef>> {
        if alternatives.len() < 2 {
            return None;
        }
        let doc = self.doc;
        let schemas = self.schemas;
        let children = doc.children(node);
        if children.is_empty() {
            return None;
        }
        // Key to (value, alternative index), in insertion order.
        let mut const_map: IndexMap<String, Vec<(Value, usize)>> = IndexMap::new();
        for (i, alt) in alternatives.iter().enumerate() {
            let alt = &schemas[alt.as_schema()];
            let entries: Vec<(String, SchemaId)> = match doc[node].kind {
                AstKind::Object => alt
                    .properties
                    .as_ref()?
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_schema()))
                    .collect(),
                AstKind::Array => {
                    let items = match (&alt.prefix_items, &alt.items) {
                        (Some(p), _) => p,
                        (None, Some(Items::Many(items))) => items,
                        _ => return None,
                    };
                    items
                        .iter()
                        .enumerate()
                        .map(|(idx, v)| (idx.to_string(), v.as_schema()))
                        .collect()
                }
                _ => return None,
            };
            for (key, s) in entries {
                let s = &schemas[s];
                if let Some(c) = &s.const_ {
                    const_map.entry(key).or_default().push(((**c).clone(), i));
                } else if let Some(values) = &s.enum_ {
                    for v in values.iter() {
                        const_map
                            .entry(key.clone())
                            .or_default()
                            .push((v.clone(), i));
                    }
                }
            }
        }
        for (key, values) in &const_map {
            let covered: HashSet<usize> = values.iter().map(|(_, i)| *i).collect();
            if covered.len() == alternatives.len() {
                let value = match doc[node].kind {
                    AstKind::Object => children
                        .iter()
                        .find(|&&p| doc.key(p) == key)
                        .and_then(|&p| doc.value(p)),
                    _ => key
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| children.get(i).copied()),
                };
                let value = value.and_then(|v| doc.string(v))?;
                // A JS `Map` groups the indices by value. The first index for a value
                // fixes the position of the group.
                let matching: Vec<SchemaRef> = values
                    .iter()
                    .filter(|(v, _)| v.as_str() == Some(value))
                    .map(|(_, i)| alternatives[*i])
                    .collect();
                return (!matching.is_empty()).then_some(matching);
            }
        }
        None
    }

    fn validate_number(&self, node: NodeId, schema_id: SchemaId, result: &mut ValidationResult) {
        let limits = self.schemas[schema_id].limits();
        let AstKind::Number { value: val, .. } = self.doc[node].kind else {
            return;
        };
        if let Some(multiple_of) = limits.multiple_of
            && self.enabled("multipleOf")
        {
            let mut remainder = -1.0;
            if multiple_of.fract() == 0.0 {
                remainder = val % multiple_of;
            } else if let (Some(mut m), Some(mut v)) =
                (normalize_float(multiple_of), normalize_float(val))
            {
                let multiplier = 10f64.powf((v.1 - m.1).abs() as f64);
                if v.1 < m.1 {
                    v.0 *= multiplier;
                } else {
                    m.0 *= multiplier;
                }
                remainder = v.0 % m.0;
            }
            if remainder != 0.0 {
                result.problems.push(self.problem(
                    node,
                    format!("Value is not divisible by {}.", fmt_num(multiple_of)),
                ));
            }
        }
        let exclusive_limit = |limit: Option<f64>, exclusive: Option<Exclusive>| match exclusive {
            Some(Exclusive::Number(n)) => Some(n),
            Some(Exclusive::Bool(true)) => limit,
            _ => None,
        };
        let limit = |limit: Option<f64>, exclusive: Option<Exclusive>| match exclusive {
            Some(Exclusive::Bool(true)) => None,
            _ => limit,
        };
        if let Some(min) = exclusive_limit(limits.minimum, limits.exclusive_minimum)
            && val <= min
            && self.enabled("exclusiveMinimum")
        {
            result.problems.push(self.problem(
                node,
                format!("Value is below the exclusive minimum of {}.", fmt_num(min)),
            ));
        }
        if let Some(max) = exclusive_limit(limits.maximum, limits.exclusive_maximum)
            && val >= max
            && self.enabled("exclusiveMaximum")
        {
            result.problems.push(self.problem(
                node,
                format!("Value is above the exclusive maximum of {}.", fmt_num(max)),
            ));
        }
        if let Some(min) = limit(limits.minimum, limits.exclusive_minimum)
            && val < min
            && self.enabled("minimum")
        {
            result.problems.push(self.problem(
                node,
                format!("Value is below the minimum of {}.", fmt_num(min)),
            ));
        }
        if let Some(max) = limit(limits.maximum, limits.exclusive_maximum)
            && val > max
            && self.enabled("maximum")
        {
            result.problems.push(self.problem(
                node,
                format!("Value is above the maximum of {}.", fmt_num(max)),
            ));
        }
    }

    fn validate_string(&self, node: NodeId, schema_id: SchemaId, result: &mut ValidationResult) {
        let schema = &self.schemas[schema_id];
        let Some(value) = self.doc.string(node) else {
            return;
        };
        let length = value.chars().count() as f64;
        if let Some(min) = schema.limits().min_length
            && length < min
            && self.enabled("minLength")
        {
            result.problems.push(self.problem(
                node,
                format!(
                    "String is shorter than the minimum length of {}.",
                    fmt_num(min)
                ),
            ));
        }
        if let Some(max) = schema.limits().max_length
            && length > max
            && self.enabled("maxLength")
        {
            result.problems.push(self.problem(
                node,
                format!(
                    "String is longer than the maximum length of {}.",
                    fmt_num(max)
                ),
            ));
        }
        if let Some(pattern) = &schema.pattern
            && self.enabled("pattern")
            && let Some(regex) = &schema.pattern_regex
            && regex.find(value).is_none()
        {
            let message = schema
                .vscode()
                .pattern_error_message
                .clone()
                .or_else(|| schema.vscode().error_message.clone())
                .unwrap_or_else(|| format!("String does not match the pattern of \"{pattern}\"."));
            result.problems.push(self.problem(node, message));
        }
        if let Some(format) = &schema.format
            && self.enabled("format")
            && is_format_assertion_enabled(self.vocabularies, self.explicit_draft)
        {
            let custom = || {
                schema
                    .vscode()
                    .pattern_error_message
                    .clone()
                    .or_else(|| schema.vscode().error_message.clone())
            };
            match format.as_str() {
                "uri" | "uri-reference" => {
                    let error = if value.is_empty() {
                        Some("URI expected.")
                    } else if format == "uri" && !has_uri_scheme(value) {
                        Some("URI with a scheme is expected.")
                    } else {
                        None
                    };
                    if let Some(error) = error {
                        let message =
                            custom().unwrap_or_else(|| format!("String is not a URI: {error}"));
                        result.problems.push(self.problem(node, message));
                    }
                }
                _ => {
                    if let Some((regex, error)) = format_pattern(format)
                        && (value.is_empty() || regex.find(value).is_none())
                    {
                        let message = custom().unwrap_or_else(|| error.to_owned());
                        result.problems.push(self.problem(node, message));
                    }
                }
            }
        }
    }

    fn validate_array(
        &mut self,
        node: NodeId,
        schema_id: SchemaId,
        result: &mut ValidationResult,
        collector: &mut Collector,
    ) {
        let doc = self.doc;
        let schemas = self.schemas;
        let schema = &schemas[schema_id];
        let items = doc.children(node);

        let (prefix_items, additional): (Option<&[SchemaRef]>, Option<SchemaRef>) =
            if self.draft >= SchemaDraft::V2020_12 {
                (
                    schema.prefix_items.as_deref(),
                    match &schema.items {
                        Some(Items::One(s)) => Some(*s),
                        _ => None,
                    },
                )
            } else {
                match &schema.items {
                    Some(Items::Many(items)) => (Some(items.as_slice()), schema.additional_items),
                    Some(Items::One(s)) => (None, Some(*s)),
                    None => (None, None),
                }
            };
        let mut index = 0;
        if let Some(prefix_items) = prefix_items {
            let max = prefix_items.len().min(items.len());
            while index < max {
                let mut item_result = ValidationResult::default();
                self.validate(
                    items[index],
                    prefix_items[index].as_schema(),
                    &mut item_result,
                    collector,
                );
                result.merge_property_match(item_result);
                result.processed_properties.insert(index.to_string());
                index += 1;
            }
        }
        if let Some(additional) = additional
            && index < items.len()
        {
            match additional {
                SchemaRef::Bool(b) => {
                    if !b {
                        result.problems.push(self.problem(
                                node,
                                format!(
                                    "Array has too many items according to schema. Expected {index} or fewer."
                                ),
                            ));
                    }
                    while index < items.len() {
                        result.processed_properties.insert(index.to_string());
                        result.properties_value_matches += 1;
                        index += 1;
                    }
                }
                SchemaRef::Schema(s) => {
                    while index < items.len() {
                        let mut item_result = ValidationResult::default();
                        self.validate(items[index], s, &mut item_result, collector);
                        result.merge_property_match(item_result);
                        result.processed_properties.insert(index.to_string());
                        index += 1;
                    }
                }
            }
        }

        if let Some(contains) = schema.contains
            && self.enabled("contains")
        {
            let contains = contains.as_schema();
            let mut count = 0;
            for (i, &item) in items.iter().enumerate() {
                let mut item_result = ValidationResult::default();
                self.validate(item, contains, &mut item_result, &mut Collector::noop());
                if !item_result.has_problems() {
                    count += 1;
                    if self.draft >= SchemaDraft::V2020_12 {
                        result.processed_properties.insert(i.to_string());
                    }
                }
            }
            let count = count as f64;
            if count == 0.0 && schema.limits().min_contains.is_none() {
                let message = schema
                    .vscode()
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "Array does not contain required item.".to_owned());
                result.problems.push(self.problem(node, message));
            }
            if let Some(min) = schema.limits().min_contains
                && count < min
                && self.enabled("minContains")
            {
                let message = schema.vscode().error_message.clone().unwrap_or_else(|| format!("Array has too few items that match the contains contraint. Expected {} or more.", fmt_num(min)));
                result.problems.push(self.problem(node, message));
            }
            if let Some(max) = schema.limits().max_contains
                && count > max
                && self.enabled("maxContains")
            {
                let message = schema.vscode().error_message.clone().unwrap_or_else(|| format!("Array has too many items that match the contains contraint. Expected {} or less.", fmt_num(max)));
                result.problems.push(self.problem(node, message));
            }
        }

        if let Some(unevaluated) = schema.unevaluated_items
            && self.enabled("unevaluatedItems")
        {
            for (i, &item) in items.iter().enumerate() {
                let key = i.to_string();
                if !result.processed_properties.contains(&key) {
                    if unevaluated == SchemaRef::Bool(false) {
                        result.problems.push(self.problem(
                            node,
                            "Item does not match any validation rule from the array.".to_owned(),
                        ));
                    } else {
                        let mut item_result = ValidationResult::default();
                        self.validate(item, unevaluated.as_schema(), &mut item_result, collector);
                        result.merge_property_match(item_result);
                    }
                }
                result.processed_properties.insert(key);
                result.properties_value_matches += 1;
            }
        }

        let count = items.len() as f64;
        if let Some(min) = schema.limits().min_items
            && count < min
            && self.enabled("minItems")
        {
            result.problems.push(self.problem(
                node,
                format!(
                    "Array has too few items. Expected {} or more.",
                    fmt_num(min)
                ),
            ));
        }
        if let Some(max) = schema.limits().max_items
            && count > max
            && self.enabled("maxItems")
        {
            result.problems.push(self.problem(
                node,
                format!(
                    "Array has too many items. Expected {} or fewer.",
                    fmt_num(max)
                ),
            ));
        }
        if schema.unique_items && self.enabled("uniqueItems") {
            let values: Vec<Value> = items.iter().map(|&i| doc.node_value(i)).collect();
            let has_duplicates = values
                .iter()
                .enumerate()
                .any(|(i, a)| values[i + 1..].iter().any(|b| js_equals(a, b)));
            if has_duplicates {
                result
                    .problems
                    .push(self.problem(node, "Array has duplicate items.".to_owned()));
            }
        }
    }

    fn property_not_allowed(&self, schema_id: SchemaId, property: NodeId, name: &str) -> Problem {
        let key = self.doc.children(property)[0];
        let message = self.schemas[schema_id]
            .vscode()
            .error_message
            .clone()
            .unwrap_or_else(|| format!("Property {name} is not allowed."));
        self.problem(key, message)
    }

    fn validate_object(
        &mut self,
        node: NodeId,
        schema_id: SchemaId,
        result: &mut ValidationResult,
        collector: &mut Collector,
    ) {
        let doc = self.doc;
        let schemas = self.schemas;
        let schema = &schemas[schema_id];
        let properties = doc.children(node);

        // Key to (property node, value node). A later duplicate key wins.
        let mut seen_keys: IndexMap<&str, (NodeId, Option<NodeId>)> = IndexMap::new();
        let mut unprocessed: IndexMap<&str, ()> = IndexMap::new();
        for &p in properties {
            let key = doc.key(p);
            seen_keys.insert(key, (p, doc.value(p)));
            unprocessed.insert(key, ());
        }
        let child_of = |key: &str| seen_keys.get(key).and_then(|&(p, v)| Some((p, v?)));

        if let Some(required) = &schema.required
            && self.enabled("required")
        {
            for name in required {
                if child_of(name).is_none() {
                    result
                        .problems
                        .push(self.problem(node, format!("Missing property \"{name}\".")));
                }
            }
        }

        if let Some(props) = &schema.properties
            && self.enabled("properties")
        {
            for (name, &prop_schema) in props.iter() {
                unprocessed.shift_remove(name.as_str());
                result.processed_properties.insert(name.clone());
                if let Some((p, child)) = child_of(name) {
                    match prop_schema {
                        SchemaRef::Bool(false) => {
                            result
                                .problems
                                .push(self.property_not_allowed(schema_id, p, name));
                        }
                        SchemaRef::Bool(true) => {
                            result.properties_matches += 1;
                            result.properties_value_matches += 1;
                        }
                        SchemaRef::Schema(s) => {
                            let mut prop_result = ValidationResult::default();
                            self.validate(child, s, &mut prop_result, collector);
                            result.merge_property_match(prop_result);
                        }
                    }
                }
            }
        }

        if let Some(patterns) = &schema.pattern_properties
            && self.enabled("patternProperties")
        {
            let mut pattern_matches: IndexMap<&str, ()> = IndexMap::new();
            for pattern in patterns.iter() {
                let Some(regex) = &pattern.regex else {
                    continue;
                };
                for (&name, &(p, value)) in &seen_keys {
                    if regex.find(name).is_none() {
                        continue;
                    }
                    if unprocessed.contains_key(name) {
                        pattern_matches.insert(name, ());
                    }
                    let Some(child) = value else {
                        continue;
                    };
                    match pattern.schema {
                        SchemaRef::Bool(false) => {
                            result
                                .problems
                                .push(self.property_not_allowed(schema_id, p, name));
                        }
                        SchemaRef::Bool(true) => {
                            result.properties_matches += 1;
                            result.properties_value_matches += 1;
                        }
                        SchemaRef::Schema(s) => {
                            let mut prop_result = ValidationResult::default();
                            self.validate(child, s, &mut prop_result, collector);
                            result.merge_property_match(prop_result);
                        }
                    }
                }
            }
            for name in pattern_matches.keys() {
                unprocessed.shift_remove(name);
                result.processed_properties.insert((*name).to_owned());
            }
        }

        if let Some(additional) = schema.additional_properties
            && self.enabled("additionalProperties")
        {
            let names: Vec<&str> = unprocessed.keys().copied().collect();
            for name in names {
                unprocessed.shift_remove(name);
                result.processed_properties.insert(name.to_owned());
                if let Some((p, child)) = child_of(name) {
                    match additional {
                        SchemaRef::Bool(false) => {
                            result
                                .problems
                                .push(self.property_not_allowed(schema_id, p, name));
                        }
                        SchemaRef::Bool(true) => {}
                        SchemaRef::Schema(s) => {
                            let mut prop_result = ValidationResult::default();
                            self.validate(child, s, &mut prop_result, collector);
                            result.merge_property_match(prop_result);
                        }
                    }
                }
            }
        }

        if let Some(deps) = schema.dependent_required.as_deref()
            && self.enabled("dependentRequired")
        {
            for (key, required) in deps {
                if child_of(key).is_some() {
                    self.validate_dependency_names(node, key, required, &child_of, result);
                }
            }
        }
        if let Some(deps) = schema.dependent_schemas.as_deref()
            && self.enabled("dependentSchemas")
        {
            for (key, &dep) in deps {
                if child_of(key).is_some() {
                    self.validate_dependency_schema(node, dep, result, collector);
                }
            }
        }

        if let Some(unevaluated) = schema.unevaluated_properties
            && self.enabled("unevaluatedProperties")
        {
            let mut processed = Vec::new();
            for &name in unprocessed.keys() {
                if result.processed_properties.contains(name) {
                    continue;
                }
                processed.push(name);
                if let Some((p, child)) = child_of(name) {
                    match unevaluated {
                        SchemaRef::Bool(false) => {
                            result
                                .problems
                                .push(self.property_not_allowed(schema_id, p, name));
                        }
                        SchemaRef::Bool(true) => {}
                        SchemaRef::Schema(s) => {
                            let mut prop_result = ValidationResult::default();
                            self.validate(child, s, &mut prop_result, collector);
                            result.merge_property_match(prop_result);
                        }
                    }
                }
            }
            for name in processed {
                unprocessed.shift_remove(name);
                result.processed_properties.insert(name.to_owned());
            }
        }

        let count = properties.len() as f64;
        if let Some(max) = schema.limits().max_properties
            && self.enabled("maxProperties")
            && count > max
        {
            result.problems.push(self.problem(
                node,
                format!("Object has more properties than limit of {}.", fmt_num(max)),
            ));
        }
        if let Some(min) = schema.limits().min_properties
            && self.enabled("minProperties")
            && count < min
        {
            result.problems.push(self.problem(
                node,
                format!(
                    "Object has fewer properties than the required number of {}",
                    fmt_num(min)
                ),
            ));
        }

        if let Some(deps) = schema.dependencies.as_deref()
            && self.enabled("dependencies")
        {
            for (key, dep) in deps {
                if child_of(key).is_some() {
                    match dep {
                        Dependency::Required(names) => {
                            self.validate_dependency_names(node, key, names, &child_of, result)
                        }
                        Dependency::Schema(s) => {
                            self.validate_dependency_schema(node, *s, result, collector)
                        }
                    }
                }
            }
        }

        if let Some(names_schema) = schema.property_names
            && self.enabled("propertyNames")
        {
            for &p in properties {
                let key = doc.children(p)[0];
                self.validate(key, names_schema.as_schema(), result, collector);
            }
        }
    }

    fn validate_dependency_names(
        &self,
        node: NodeId,
        key: &str,
        names: &[String],
        child_of: &dyn Fn(&str) -> Option<(NodeId, NodeId)>,
        result: &mut ValidationResult,
    ) {
        for name in names {
            if child_of(name).is_none() {
                result.problems.push(self.problem(
                    node,
                    format!("Object is missing property {name} required by property {key}."),
                ));
            } else {
                result.properties_value_matches += 1;
            }
        }
    }

    fn validate_dependency_schema(
        &mut self,
        node: NodeId,
        dep: SchemaRef,
        result: &mut ValidationResult,
        collector: &mut Collector,
    ) {
        let mut prop_result = ValidationResult::default();
        self.validate(node, dep.as_schema(), &mut prop_result, collector);
        let processed = prop_result.processed_properties.clone();
        result.merge_property_match(prop_result);
        result.processed_properties.extend(processed);
    }
}

/// `float.toString()` split into digits and a decimal exponent.
fn normalize_float(n: f64) -> Option<(f64, i32)> {
    let s = js_number_to_string(n);
    let (mantissa, exp) = match s.split_once('e') {
        Some((m, e)) => (m, Some(e)),
        None => (s.as_str(), None),
    };
    let (int, frac) = match mantissa.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mantissa, ""),
    };
    let digits = int.strip_prefix('-').unwrap_or(int);
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let exp = match exp {
        Some(e) => e.parse::<i32>().ok()?,
        None => 0,
    };
    let value = format!("{int}{frac}").parse::<f64>().ok()?;
    Some((value, frac.len() as i32 - exp))
}

/// Whether the URI regex of the `uri` format finds a scheme.
fn has_uri_scheme(value: &str) -> bool {
    matches!(value.find([':', '/', '?', '#']), Some(i) if i > 0 && value.as_bytes()[i] == b':')
}

const IPV4_PART: &str = r"(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)";

static FORMATS: LazyLock<Vec<(&'static str, Regex, &'static str)>> = LazyLock::new(|| {
    let ipv4_tail = format!(r"({IPV4_PART}(\.{IPV4_PART}){{3}})");
    let ipv6 = format!(
        r"^((([0-9a-f]{{1,4}}:){{7}}([0-9a-f]{{1,4}}|:))|(([0-9a-f]{{1,4}}:){{6}}(:[0-9a-f]{{1,4}}|{v4}|:))|(([0-9a-f]{{1,4}}:){{5}}(((:[0-9a-f]{{1,4}}){{1,2}})|:{v4}|:))|(([0-9a-f]{{1,4}}:){{4}}(((:[0-9a-f]{{1,4}}){{1,3}})|((:[0-9a-f]{{1,4}})?:{v4})|:))|(([0-9a-f]{{1,4}}:){{3}}(((:[0-9a-f]{{1,4}}){{1,4}})|((:[0-9a-f]{{1,4}}){{0,2}}:{v4})|:))|(([0-9a-f]{{1,4}}:){{2}}(((:[0-9a-f]{{1,4}}){{1,5}})|((:[0-9a-f]{{1,4}}){{0,3}}:{v4})|:))|(([0-9a-f]{{1,4}}:){{1}}(((:[0-9a-f]{{1,4}}){{1,6}})|((:[0-9a-f]{{1,4}}){{0,4}}:{v4})|:))|(:(((:[0-9a-f]{{1,4}}){{1,7}})|((:[0-9a-f]{{1,4}}){{0,5}}:{v4})|:)))$",
        v4 = ipv4_tail
    );
    let entries: [(&str, String, &str, &str); 10] = [
        (
            "color-hex",
            r"^#([0-9A-Fa-f]{3,4}|([0-9A-Fa-f]{2}){3,4})$".into(),
            "",
            "Invalid color format. Use #RGB, #RGBA, #RRGGBB or #RRGGBBAA.",
        ),
        (
            "date-time",
            r"^(\d{4})-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])T([01][0-9]|2[0-3]):([0-5][0-9]):([0-5][0-9]|60)(\.[0-9]+)?(Z|(\+|-)([01][0-9]|2[0-3]):([0-5][0-9]))$".into(),
            "i",
            "String is not a RFC3339 date-time.",
        ),
        (
            "date",
            r"^(\d{4})-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$".into(),
            "i",
            "String is not a RFC3339 date.",
        ),
        (
            "time",
            r"^([01][0-9]|2[0-3]):([0-5][0-9]):([0-5][0-9]|60)(\.[0-9]+)?(Z|(\+|-)([01][0-9]|2[0-3]):([0-5][0-9]))$".into(),
            "i",
            "String is not a RFC3339 time.",
        ),
        (
            "duration",
            r"^P(?!$)(\d+Y)?(\d+M)?(\d+W)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+(\.\d+)?S)?)?$".into(),
            "",
            "String is not an ISO 8601 duration.",
        ),
        (
            "uuid",
            r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$".into(),
            "i",
            "String is not a valid UUID.",
        ),
        (
            "email",
            r#"^(([^<>()\[\]\\.,;:\s@"]+(\.[^<>()\[\]\\.,;:\s@"]+)*)|(".+"))@((\[[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}])|(([a-zA-Z0-9-]+\.)+[a-zA-Z]{2,}))$"#.into(),
            "",
            "String is not an e-mail address.",
        ),
        (
            "hostname",
            r"^(?=.{1,253}\.?$)[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[-0-9a-z]{0,61}[0-9a-z])?)*\.?$".into(),
            "i",
            "String is not a hostname.",
        ),
        (
            "ipv4",
            format!(r"^(?:{p}\.){{3}}{p}$", p = r"(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)"),
            "",
            "String is not an IPv4 address.",
        ),
        ("ipv6", ipv6, "i", "String is not an IPv6 address."),
    ];
    entries
        .into_iter()
        .map(|(name, pattern, flags, message)| {
            let regex =
                Regex::with_flags(&pattern, flags).unwrap_or_else(|e| panic!("format {name}: {e}"));
            (name, regex, message)
        })
        .collect()
});

fn format_pattern(format: &str) -> Option<(&'static Regex, &'static str)> {
    FORMATS
        .iter()
        .find(|(name, _, _)| *name == format)
        .map(|(_, regex, message)| (regex, *message))
}
