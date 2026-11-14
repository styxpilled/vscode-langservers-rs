//! Port of `services/jsonSchemaService.ts`.
//!
//! Upstream loads schemas with promises. Here the request service is called
//! synchronously, which gives the same order of operations: each resolution is one
//! promise chain upstream.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use indexmap::IndexMap;
use lsp_types::{DiagnosticRelatedInformation, Location, Range};
use regress::Regex;
use serde_json::Value;

use crate::glob::create_regex;
use crate::schema::graph::{AnchorMaps, Graph, Obj, ObjId, Val};
use crate::schema::{SchemaId, SchemaRef, Schemas, compile};
use crate::validator::{ApplicableSchema, get_schema_draft_from_id};
use crate::vocabularies::Vocabularies;
use crate::{AstKind, ErrorCode, JsonDocument, SchemaDraft};
use vscode_uri::{Uri, decode_uri_component};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaRequestError {
    pub message: String,
    /// Added to [`ErrorCode::SCHEMA_RESOLVE_ERROR`] if below `0x10000`. Can be negative.
    pub code: Option<i64>,
}

pub type SchemaRequestService = Box<dyn FnMut(&str) -> Result<String, SchemaRequestError> + Send>;
/// `WorkspaceContextService.resolveRelativePath(relativePath, resource)`.
pub type ResolveRelativePath = Box<dyn Fn(&str, &str) -> String + Send>;
pub type FallbackSchemaProvider = Box<dyn Fn(&str) -> Option<Vec<String>> + Send>;

#[derive(Clone, Debug, Default)]
pub struct SchemaConfiguration {
    pub uri: String,
    pub file_match: Option<Vec<String>>,
    pub schema: Option<Value>,
    pub folder_uri: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SchemaAssociation {
    pub pattern: Vec<String>,
    pub uris: Vec<String>,
    pub folder_uri: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SchemaContributions {
    pub schemas: serde_json::Map<String, Value>,
    pub schema_associations: Vec<SchemaAssociation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchemaDiagnostic {
    pub message: String,
    pub code: ErrorCode,
    pub related_information: Option<Vec<DiagnosticRelatedInformation>>,
}

fn to_diagnostic(message: String, code: ErrorCode, related_url: Option<&str>) -> SchemaDiagnostic {
    let related_information = related_url.and_then(|url| {
        Some(vec![DiagnosticRelatedInformation {
            location: Location {
                uri: url.parse().ok()?,
                range: Range::default(),
            },
            message: message.clone(),
        }])
    });
    SchemaDiagnostic {
        message,
        code,
        related_information,
    }
}

#[derive(Debug)]
pub struct ResolvedSchema {
    pub schemas: Schemas,
    pub root: SchemaId,
    pub errors: Vec<SchemaDiagnostic>,
    pub warnings: Vec<SchemaDiagnostic>,
    pub schema_draft: Option<SchemaDraft>,
    pub active_vocabularies: Option<Vocabularies>,
    graph_root: Option<ObjId>,
}

impl ResolvedSchema {
    /// `getSection`.
    pub fn get_section(&self, path: &[&str]) -> Option<SchemaId> {
        let mut schema = SchemaRef::Schema(self.root);
        for next in path {
            let SchemaRef::Schema(id) = schema else {
                break;
            };
            let s = &self.schemas[id];
            schema = if let Some(props) = &s.properties {
                // Upstream tests `typeof schema.properties[next]`, which is always truthy.
                *props.get(*next)?
            } else if let Some(patterns) = &s.pattern_properties {
                patterns
                    .iter()
                    .find(|p| p.regex.as_ref().is_some_and(|r| r.find(next).is_some()))?
                    .schema
            } else if let Some(SchemaRef::Schema(additional)) = s.additional_properties {
                SchemaRef::Schema(additional)
            } else if next.bytes().any(|b| b.is_ascii_digit()) {
                match &s.items {
                    Some(crate::schema::Items::Many(items)) => {
                        let digits: String = next
                            .trim_start()
                            .chars()
                            .take_while(char::is_ascii_digit)
                            .collect();
                        *items.get(digits.parse::<usize>().ok()?)?
                    }
                    Some(crate::schema::Items::One(item)) => *item,
                    None => return None,
                }
            } else {
                return None;
            };
        }
        Some(schema.as_schema())
    }
}

#[derive(Clone, Debug)]
struct Unresolved {
    schema: ObjId,
    errors: Vec<SchemaDiagnostic>,
}

#[derive(Debug)]
struct Handle {
    uri: String,
    dependencies: HashSet<String>,
    anchors: Option<IndexMap<String, ObjId>>,
    resolved: Option<Arc<ResolvedSchema>>,
    unresolved: Option<Unresolved>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct HandleId(usize);

struct FilePatternAssociation {
    globs: Vec<(Arc<Regex>, bool)>,
    folder_uri: Option<String>,
    uris: Vec<String>,
}

/// Compiled globs. Zed sends about 1500 associations, which take 10 ms to compile.
type GlobCache = HashMap<String, Option<Arc<Regex>>>;

impl FilePatternAssociation {
    fn new(
        patterns: &[String],
        folder_uri: Option<&str>,
        uris: Vec<String>,
        cache: &mut GlobCache,
    ) -> Self {
        let mut globs = Vec::new();
        for pattern in patterns {
            let (include, pattern) = match pattern.strip_prefix('!') {
                Some(p) => (false, p),
                None => (true, pattern.as_str()),
            };
            if pattern.is_empty() {
                continue;
            }
            let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
            let glob = format!("**/{pattern}");
            let regex = cache
                .entry(glob)
                .or_insert_with_key(|glob| create_regex(glob, true, true).map(Arc::new));
            match regex.clone() {
                Some(regex) => globs.push((regex, include)),
                None => {
                    return FilePatternAssociation {
                        globs: Vec::new(),
                        folder_uri: folder_uri.map(str::to_owned),
                        uris: Vec::new(),
                    };
                }
            }
        }
        let folder_uri = folder_uri.filter(|f| !f.is_empty()).map(|f| {
            let mut f = normalize_resource_for_matching(f);
            if !f.ends_with('/') {
                f.push('/');
            }
            f
        });
        FilePatternAssociation {
            globs,
            folder_uri,
            uris,
        }
    }

    fn matches_pattern(&self, file_name: &str) -> bool {
        if let Some(folder) = &self.folder_uri
            && !file_name.starts_with(folder.as_str())
        {
            return false;
        }
        let mut matched = false;
        for (regex, include) in &self.globs {
            if regex.find(file_name).is_some() {
                matched = *include;
            }
        }
        matched
    }
}

/// `normalizeId`.
pub fn normalize_id(id: &str) -> String {
    let id = match id.strip_prefix("http://json-schema.org/") {
        Some(rest) => format!("https://json-schema.org/{rest}"),
        None => id.to_owned(),
    };
    Uri::parse(&id).map_or(id, |u| u.to_string_unencoded())
}

fn normalize_resource_for_matching(resource: &str) -> String {
    match Uri::parse(resource) {
        Some(mut uri) => {
            uri.fragment.clear();
            uri.query.clear();
            uri.to_string_unencoded()
        }
        None => resource.to_owned(),
    }
}

fn to_display_string(url: &str) -> String {
    match Uri::parse(url) {
        Some(uri) if uri.scheme == "file" => uri.fs_path(),
        _ => url.to_owned(),
    }
}

/// `/^[A-Za-z][A-Za-z0-9+\-.+]*:\/.*/`
fn has_scheme(s: &str) -> bool {
    let b = s.as_bytes();
    if !b.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let end = b
        .iter()
        .position(|c| !(c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')))
        .unwrap_or(b.len());
    b[end..].starts_with(b":/")
}

/// JS `encodeURIComponent`.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// JS `str.split('#', 2)`.
fn split_ref(r: &str) -> (&str, Option<&str>) {
    let mut parts = r.split('#');
    let first = parts.next().unwrap_or("");
    (first, parts.next())
}

pub struct JsonSchemaService {
    graph: Graph,
    handles: Vec<Handle>,
    contribution_schemas: IndexMap<String, HandleId>,
    contribution_associations: Vec<usize>,
    schemas_by_id: HashMap<String, HandleId>,
    associations: Vec<FilePatternAssociation>,
    file_pattern_associations: Vec<usize>,
    registered_schema_ids: IndexMap<String, ()>,
    request_service: Option<SchemaRequestService>,
    resolve_relative_path: Option<ResolveRelativePath>,
    fallback_schema_provider: Option<FallbackSchemaProvider>,
    cached_schema_for_resource: Option<(String, Option<Arc<ResolvedSchema>>)>,
    id_counter: u32,
    glob_cache: GlobCache,
}

/// The state of one `resolveSchemaContent` call.
struct Resolution {
    handle: HandleId,
    errors: Vec<SchemaDiagnostic>,
    embedded_registered_for: HashSet<String>,
    embedded_schema_uris: HashSet<String>,
    draft: Option<SchemaDraft>,
}

impl Resolution {
    fn is_pre_201909(&self) -> bool {
        self.draft.is_some_and(|d| d < SchemaDraft::V2019_09)
    }

    fn allows_dynamic_ref(&self) -> bool {
        self.draft.is_none_or(|d| d >= SchemaDraft::V2020_12)
    }
}

const IGNORED_REF_SIBLINGS: [&str; 5] = ["$ref", "$schema", "$id", "id", "$comment"];
const RESERVED_KEYS: [&str; 6] = ["$ref", "$defs", "definitions", "$schema", "$id", "id"];
const KEEP_ON_WRAPPER_KEYS: [&str; 4] = [
    "$recursiveAnchor",
    "$dynamicAnchor",
    "unevaluatedItems",
    "unevaluatedProperties",
];

impl JsonSchemaService {
    pub fn new(
        request_service: Option<SchemaRequestService>,
        resolve_relative_path: Option<ResolveRelativePath>,
        fallback_schema_provider: Option<FallbackSchemaProvider>,
    ) -> Self {
        JsonSchemaService {
            graph: Graph::default(),
            handles: Vec::new(),
            contribution_schemas: IndexMap::new(),
            contribution_associations: Vec::new(),
            schemas_by_id: HashMap::new(),
            associations: Vec::new(),
            file_pattern_associations: Vec::new(),
            registered_schema_ids: IndexMap::new(),
            request_service,
            resolve_relative_path,
            fallback_schema_provider,
            cached_schema_for_resource: None,
            id_counter: 0,
            glob_cache: GlobCache::new(),
        }
    }

    pub fn get_registered_schema_ids(&self, filter: Option<&dyn Fn(&str) -> bool>) -> Vec<String> {
        self.registered_schema_ids
            .keys()
            .filter(|id| {
                let scheme = Uri::parse(id).map(|u| u.scheme).unwrap_or_default();
                scheme != "schemaservice" && filter.is_none_or(|f| f(&scheme))
            })
            .cloned()
            .collect()
    }

    /// `onResourceChange`. Returns whether a loaded schema was cleared.
    pub fn on_resource_change(&mut self, uri: &str) -> bool {
        self.cached_schema_for_resource = None;
        let mut has_changes = false;
        let mut to_walk = vec![normalize_id(uri)];
        let mut all: Vec<Option<HandleId>> =
            self.schemas_by_id.values().copied().map(Some).collect();
        while let Some(curr) = to_walk.pop() {
            for slot in &mut all {
                let Some(h) = *slot else {
                    continue;
                };
                let handle = &self.handles[h.0];
                if handle.uri == curr || handle.dependencies.contains(&curr) {
                    if handle.uri != curr {
                        to_walk.push(handle.uri.clone());
                    }
                    if self.clear_schema(h) {
                        has_changes = true;
                    }
                    *slot = None;
                }
            }
        }
        has_changes
    }

    pub fn set_schema_contributions(&mut self, contributions: &SchemaContributions) {
        for (id, schema) in &contributions.schemas {
            let id = normalize_id(id);
            let content = self.content_from_json(schema);
            let handle = self.add_schema_handle(&id, Some(content));
            self.contribution_schemas.insert(id, handle);
        }
        for association in &contributions.schema_associations {
            let uris = association.uris.iter().map(|u| normalize_id(u)).collect();
            let index = self.add_file_pattern_association(
                &association.pattern,
                association.folder_uri.as_deref(),
                uris,
            );
            self.contribution_associations.push(index);
        }
    }

    fn content_from_json(&mut self, schema: &Value) -> ObjId {
        match self.graph.add_json(schema) {
            Val::Obj(obj) => obj,
            _ => self.graph.new_obj(),
        }
    }

    fn add_schema_handle(&mut self, id: &str, content: Option<ObjId>) -> HandleId {
        let handle = HandleId(self.handles.len());
        self.handles.push(Handle {
            uri: id.to_owned(),
            dependencies: HashSet::new(),
            anchors: None,
            resolved: None,
            unresolved: content.map(|schema| Unresolved {
                schema,
                errors: Vec::new(),
            }),
        });
        self.schemas_by_id.insert(id.to_owned(), handle);
        handle
    }

    fn get_or_add_schema_handle(&mut self, id: &str) -> HandleId {
        match self.schemas_by_id.get(id) {
            Some(&h) => h,
            None => self.add_schema_handle(id, None),
        }
    }

    fn add_file_pattern_association(
        &mut self,
        pattern: &[String],
        folder_uri: Option<&str>,
        uris: Vec<String>,
    ) -> usize {
        self.associations.push(FilePatternAssociation::new(
            pattern,
            folder_uri,
            uris,
            &mut self.glob_cache,
        ));
        let index = self.associations.len() - 1;
        self.file_pattern_associations.push(index);
        index
    }

    pub fn register_external_schema(&mut self, config: &SchemaConfiguration) {
        let id = normalize_id(&config.uri);
        self.registered_schema_ids.insert(id.clone(), ());
        self.cached_schema_for_resource = None;
        if let Some(file_match) = config.file_match.as_ref().filter(|f| !f.is_empty()) {
            self.add_file_pattern_association(
                file_match,
                config.folder_uri.as_deref(),
                vec![id.clone()],
            );
        }
        match &config.schema {
            Some(schema) => {
                let content = self.content_from_json(schema);
                self.add_schema_handle(&id, Some(content));
            }
            None => {
                self.get_or_add_schema_handle(&id);
            }
        }
    }

    /// Like [`Self::clear_external_schemas`], but also frees the schema graph, which only
    /// grows. Registers `contributions` again, so loaded contributions are resolved again.
    pub fn reset(&mut self, contributions: &SchemaContributions) {
        let glob_cache = std::mem::take(&mut self.glob_cache);
        *self = JsonSchemaService::new(
            self.request_service.take(),
            self.resolve_relative_path.take(),
            self.fallback_schema_provider.take(),
        );
        self.glob_cache = glob_cache;
        self.set_schema_contributions(contributions);
        self.clear_external_schemas();
    }

    pub fn clear_external_schemas(&mut self) {
        self.schemas_by_id.clear();
        self.file_pattern_associations.clear();
        self.registered_schema_ids.clear();
        self.cached_schema_for_resource = None;
        for (id, &handle) in &self.contribution_schemas {
            self.schemas_by_id.insert(id.clone(), handle);
            self.registered_schema_ids.insert(id.clone(), ());
        }
        self.file_pattern_associations
            .extend(self.contribution_associations.iter().copied());
    }

    pub fn get_resolved_schema(&mut self, schema_id: &str) -> Option<Arc<ResolvedSchema>> {
        let id = normalize_id(schema_id);
        let handle = *self.schemas_by_id.get(&id)?;
        Some(self.resolved(handle))
    }

    fn clear_schema(&mut self, h: HandleId) -> bool {
        let handle = &mut self.handles[h.0];
        let has_changes = handle.unresolved.is_some();
        handle.resolved = None;
        handle.unresolved = None;
        handle.dependencies.clear();
        handle.anchors = None;
        has_changes
    }

    fn set_schema_content(&mut self, h: HandleId, content: ObjId) {
        let handle = &mut self.handles[h.0];
        handle.unresolved = Some(Unresolved {
            schema: content,
            errors: Vec::new(),
        });
        handle.resolved = None;
        handle.anchors = None;
    }

    fn unresolved(&mut self, h: HandleId) -> Unresolved {
        if let Some(u) = &self.handles[h.0].unresolved {
            return u.clone();
        }
        let uri = self.handles[h.0].uri.clone();
        let u = self.load_schema(&uri);
        self.handles[h.0].unresolved = Some(u.clone());
        u
    }

    fn resolved(&mut self, h: HandleId) -> Arc<ResolvedSchema> {
        if let Some(r) = &self.handles[h.0].resolved {
            return r.clone();
        }
        let unresolved = self.unresolved(h);
        let resolved = Arc::new(self.resolve_schema_content(unresolved, h));
        self.handles[h.0].resolved = Some(resolved.clone());
        resolved
    }

    fn load_schema(&mut self, url: &str) -> Unresolved {
        let empty = self.graph.new_obj();
        let fail = |message: String, code: ErrorCode| Unresolved {
            schema: empty,
            errors: vec![to_diagnostic(message, code, Some(url))],
        };
        let Some(request) = self.request_service.as_mut() else {
            return fail(
                format!(
                    "Unable to load schema from '{}'. No schema request service available",
                    to_display_string(url)
                ),
                ErrorCode::SCHEMA_RESOLVE_ERROR,
            );
        };
        match request(url) {
            Ok(content) => {
                if content.is_empty() {
                    return fail(
                        format!(
                            "Unable to load schema from '{}': No content.",
                            to_display_string(url)
                        ),
                        ErrorCode::SCHEMA_RESOLVE_ERROR,
                    );
                }
                let mut errors = Vec::new();
                let mut content = content.as_str();
                if content.starts_with('\u{feff}') {
                    errors.push(to_diagnostic(
                        format!(
                            "Problem reading content from '{}': UTF-8 with BOM detected, only UTF 8 is allowed.",
                            to_display_string(url)
                        ),
                        ErrorCode::SCHEMA_RESOLVE_ERROR,
                        Some(url),
                    ));
                    content =
                        content.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
                }
                let (value, first_error) = self.graph.parse(content);

                if let Some(offset) = first_error {
                    let offset = content[..offset.min(content.len())].encode_utf16().count();
                    errors.push(to_diagnostic(
                        format!(
                            "Unable to parse content from '{}': Parse error at offset {offset}.",
                            to_display_string(url)
                        ),
                        ErrorCode::SCHEMA_RESOLVE_ERROR,
                        Some(url),
                    ));
                }
                let schema = match value {
                    Some(Val::Obj(obj)) => obj,
                    _ => empty,
                };
                Unresolved { schema, errors }
            }
            Err(error) => {
                let mut code = ErrorCode::SCHEMA_RESOLVE_ERROR;
                if let Some(c) = error.code.filter(|&c| c < 0x10000) {
                    code.0 = code.0.wrapping_add_signed(c as i32);
                }
                fail(
                    format!(
                        "Unable to load schema from '{}': {}.",
                        to_display_string(url),
                        error.message
                    ),
                    code,
                )
            }
        }
    }

    fn resolve_path(&self, id: &str, base: &str) -> String {
        match &self.resolve_relative_path {
            Some(resolve) if !has_scheme(id) => resolve(id, base),
            _ => id.to_owned(),
        }
    }

    /// `$id || id || $originalId`.
    fn schema_id(&self, obj: ObjId) -> Option<Val> {
        let o = &self.graph[obj];
        o.get("$id")
            .filter(|v| v.is_truthy())
            .or_else(|| o.get("id").filter(|v| v.is_truthy()))
            .cloned()
            .or_else(|| {
                o.original_id
                    .clone()
                    .filter(|s| !s.is_empty())
                    .map(|s| Val::Str(s.into()))
            })
    }

    /// The id of a schema if it is a string that starts a new base URI.
    fn base_id(&self, obj: ObjId) -> Option<String> {
        match self.schema_id(obj) {
            Some(Val::Str(id)) if !id.starts_with('#') => Some(id.to_string()),
            _ => None,
        }
    }

    fn has_ref_sibling_keywords(&self, obj: ObjId) -> bool {
        self.graph[obj]
            .keys()
            .any(|k| !IGNORED_REF_SIBLINGS.contains(&k))
    }

    fn resolve_schema_content(
        &mut self,
        unresolved: Unresolved,
        handle: HandleId,
    ) -> ResolvedSchema {
        let schema = unresolved.schema;
        let draft = self
            .graph
            .get(schema, "$schema")
            .filter(|v| v.is_truthy())
            .and_then(|v| get_schema_draft_from_id(v.as_str()?));
        if draft == Some(SchemaDraft::V3) {
            return ResolvedSchema {
                schemas: Schemas::default(),
                root: SchemaId::TRUE,
                errors: vec![to_diagnostic(
                    "Draft-03 schemas are not supported.".to_owned(),
                    ErrorCode::SCHEMA_UNSUPPORTED_FEATURE,
                    None,
                )],
                warnings: Vec::new(),
                schema_draft: draft,
                active_vocabularies: None,
                graph_root: None,
            };
        }
        let mut res = Resolution {
            handle,
            errors: unresolved.errors,
            embedded_registered_for: HashSet::new(),
            embedded_schema_uris: HashSet::new(),
            draft,
        };

        let uri = self.handles[handle.0].uri.clone();
        self.register_embedded_schemas(&mut res, schema, &uri);

        let anchors = self.collect_anchors(&mut res, schema);

        self.handles[handle.0].anchors = Some(anchors);
        self.collect_dynamic_anchors(&res, schema);

        let active_vocabularies = self.resolve_metaschema_vocabularies(schema, &uri);
        self.resolve_refs(&mut res, schema, schema, handle);

        let (schemas, root) = compile(&self.graph, schema);

        ResolvedSchema {
            schemas,
            root,
            errors: res.errors,
            warnings: Vec::new(),
            schema_draft: draft,
            active_vocabularies,
            graph_root: Some(schema),
        }
    }

    /// The resolved schema as JSON, as upstream holds it. A cycle becomes `cycle`.
    /// For tests that compare with upstream.
    pub fn resolved_schema_json(&self, resolved: &ResolvedSchema, cycle: &Value) -> Value {
        match resolved.graph_root {
            Some(root) => self.graph.to_json_marked(&Val::Obj(root), cycle),
            None => Value::Object(Default::default()),
        }
    }

    fn resolve_metaschema_vocabularies(
        &mut self,
        schema: ObjId,
        base: &str,
    ) -> Option<Vocabularies> {
        let metaschema = self.graph.get_str(schema, "$schema")?.to_owned();
        let uri = normalize_id(&self.resolve_path(&metaschema, base));
        let handle = self.get_or_add_schema_handle(&uri);
        let meta = self.unresolved(handle).schema;
        let meta_draft = self
            .graph
            .get(meta, "$schema")
            .filter(|v| v.is_truthy())
            .and_then(|v| get_schema_draft_from_id(v.as_str()?));
        let is_2019_or_later = meta_draft.is_some_and(|d| d >= SchemaDraft::V2019_09);
        let vocabulary = self.graph.get(meta, "$vocabulary");
        let has_vocabulary = matches!(vocabulary, Some(Val::Obj(_) | Val::Arr(_)));
        if !has_vocabulary && !is_2019_or_later {
            return None;
        }
        let vocabs: Vocabularies = match vocabulary {
            Some(Val::Obj(v)) => self.graph[*v]
                .iter()
                .map(|(uri, required)| (uri.to_owned(), matches!(required, Val::Bool(true))))
                .collect(),
            Some(Val::Arr(items)) => items
                .iter()
                .enumerate()
                .map(|(i, required)| (i.to_string(), matches!(required, Val::Bool(true))))
                .collect(),
            _ => return None,
        };
        (!vocabs.is_empty()).then_some(vocabs)
    }

    fn find_section_and_base(
        &mut self,
        root: ObjId,
        path: &str,
        base_handle: HandleId,
    ) -> (Option<Val>, HandleId) {
        let Some(path) = decode_uri_component(path) else {
            return (None, base_handle);
        };
        let path = path.strip_prefix('/').unwrap_or(&path);
        let mut current = Val::Obj(root);
        let mut current_base = base_handle;
        for part in path.split('/') {
            let part = part.replace("~1", "/").replace("~0", "~");
            let next = match &current {
                Val::Obj(obj) => self.graph[*obj].get(&part).cloned(),
                Val::Arr(items) => crate::schema::graph::array_index(&part)
                    .and_then(|i| items.get(i as usize).cloned()),
                _ => None,
            };
            match next {
                None => return (None, current_base),
                Some(Val::Null) => return (Some(Val::Null), current_base),
                Some(v) => current = v,
            }
            if let Val::Obj(obj) = current
                && let Some(id) = self.base_id(obj)
            {
                let base_uri = self.handles[current_base.0].uri.clone();
                let resolved = normalize_id(&self.resolve_path(&id, &base_uri));
                current_base = self.get_or_add_schema_handle(&resolved);
            }
        }
        (Some(current), current_base)
    }

    fn find_schema_by_id(
        &mut self,
        res: &mut Resolution,
        root: ObjId,
        handle: HandleId,
        id: &str,
    ) -> Option<ObjId> {
        if self.handles[handle.0].anchors.is_none() {
            let anchors = self.collect_anchors(res, root);
            self.handles[handle.0].anchors = Some(anchors);
        }
        self.handles[handle.0].anchors.as_ref()?.get(id).copied()
    }

    fn union_anchor_maps(&mut self, target: ObjId, src: Option<Box<AnchorMaps>>) {
        let Some(src) = src else {
            return;
        };
        let maps = self.graph[target].anchor_maps.get_or_insert_default();
        for (name, node) in src.dynamic {
            maps.dynamic.entry(name).or_insert(node);
        }
    }

    fn merge(&mut self, target: ObjId, section: ObjId) {
        let target_has_siblings = self.has_ref_sibling_keywords(target);
        let entries: Vec<(String, Val)> = self.graph[section]
            .iter()
            .filter(|(k, _)| *k != "id" && *k != "$id")
            .map(|(k, v)| (k.to_owned(), v.clone()))
            .collect();
        for (key, value) in entries {
            let deep = matches!(key.as_str(), "properties" | "patternProperties");
            let value = match (&value, self.graph.get(target, &key)) {
                (Val::Obj(src), Some(Val::Obj(dst))) if deep => {
                    let mut merged = self.graph[*dst].spread();
                    let src_entries: Vec<(String, Val)> = self.graph[*src]
                        .iter()
                        .map(|(k, v)| (k.to_owned(), v.clone()))
                        .collect();
                    for (k, v) in src_entries {
                        merged.set(&k, v);
                    }
                    Val::Obj(self.graph.add(merged))
                }
                _ => value,
            };
            self.graph[target].set(&key, value);
        }
        if let Some(Val::Str(id)) = self.schema_id(section)
            && self.schema_id(target).is_none()
            && !target_has_siblings
        {
            self.graph[target].original_id = Some(id.to_string());
        }
        let src = &self.graph[section];
        let (src_info, src_maps) = (src.dynamic_ref_info, src.anchor_maps.clone());
        if src_info.is_some() && self.graph[target].dynamic_ref_info.is_none() {
            self.graph[target].dynamic_ref_info = src_info;
        }
        self.union_anchor_maps(target, src_maps);
    }

    fn needs_scope_isolation(
        &self,
        res: &Resolution,
        referenced: ObjId,
        referencing: ObjId,
    ) -> bool {
        let g = &self.graph;
        let has = |obj: ObjId, k: &str| g.has(obj, k);
        let has_any = |obj: ObjId, ks: &[&str]| ks.iter().any(|k| g.has(obj, k));
        let is_array = |obj: ObjId, k: &str| matches!(g.get(obj, k), Some(Val::Arr(_)));
        const OBJECT_PROPERTY: [&str; 2] = ["properties", "patternProperties"];
        const OBJECT_EVALUATOR: [&str; 10] = [
            "properties",
            "patternProperties",
            "additionalProperties",
            "dependentSchemas",
            "allOf",
            "anyOf",
            "oneOf",
            "if",
            "then",
            "else",
        ];
        const ARRAY_EVALUATOR: [&str; 9] = [
            "items",
            "additionalItems",
            "prefixItems",
            "allOf",
            "anyOf",
            "oneOf",
            "if",
            "then",
            "else",
        ];
        const CONTAINS_BOUND: [&str; 2] = ["minContains", "maxContains"];
        const BRANCH: [&str; 2] = ["then", "else"];
        const ARRAY_APPLICATOR: [&str; 3] = ["items", "prefixItems", "additionalItems"];
        let uses_2020_12 = res.draft.is_none_or(|d| d >= SchemaDraft::V2020_12);

        (has(referenced, "additionalProperties") && has_any(referencing, &OBJECT_PROPERTY))
            || (has(referenced, "additionalItems") && is_array(referencing, "items"))
            || (has(referenced, "unevaluatedProperties") && has_any(referencing, &OBJECT_EVALUATOR))
            || (has(referenced, "unevaluatedItems")
                && (has_any(referencing, &ARRAY_EVALUATOR)
                    || (uses_2020_12 && has(referencing, "contains"))))
            || (has(referencing, "unevaluatedItems") && has_any(referenced, &ARRAY_APPLICATOR))
            || (has(referenced, "contains") && has_any(referencing, &CONTAINS_BOUND))
            || (is_array(referenced, "items") && is_array(referencing, "items"))
            || (has_any(referenced, &CONTAINS_BOUND) && has(referencing, "contains"))
            || (has(referenced, "if") && has_any(referencing, &BRANCH))
            || (has_any(referenced, &BRANCH) && has(referencing, "if"))
            || (uses_2020_12 && has(referenced, "items") && has(referencing, "prefixItems"))
    }

    fn merge_ref(
        &mut self,
        res: &mut Resolution,
        target: ObjId,
        source_root: ObjId,
        source_handle: HandleId,
        ref_segment: Option<&str>,
        isolate_external_resource: bool,
    ) {
        let mut section_base = source_handle;
        let section = match ref_segment {
            None | Some("") => Some(Val::Obj(source_root)),
            Some(seg) if seg.starts_with('/') => {
                let (section, base) = self.find_section_and_base(source_root, seg, source_handle);
                section_base = base;
                section
            }
            Some(seg) => self
                .find_schema_by_id(res, source_root, source_handle, seg)
                .map(Val::Obj),
        };
        let section = match section {
            Some(Val::Bool(b)) => {
                let mut obj = Obj::default();
                if !b {
                    let empty = self.graph.new_obj();
                    obj.set("not", Val::Obj(empty));
                }
                Some(Val::Obj(self.graph.add(obj)))
            }
            s => s,
        };
        let mut section = match section {
            Some(Val::Obj(obj)) => obj,
            Some(v) if v.is_truthy() => return,
            _ => {
                let message = format!(
                    "$ref '{}' in '{}' can not be resolved.",
                    ref_segment.unwrap_or(""),
                    self.handles[source_handle.0].uri
                );
                res.errors.push(to_diagnostic(
                    message,
                    ErrorCode::SCHEMA_RESOLVE_ERROR,
                    None,
                ));
                return;
            }
        };

        if let Some(inner_ref) = self
            .graph
            .get(section, "$ref")
            .filter(|v| v.is_truthy())
            .cloned()
            && (section_base != source_handle || self.has_ref_sibling_keywords(target))
            && let Some(inner_ref) = inner_ref.as_str()
        {
            let (path, fragment) = split_ref(inner_ref);
            if !path.is_empty() && self.resolve_relative_path.is_some() && !has_scheme(path) {
                let base = self.handles[section_base.0].uri.clone();
                let mut new_ref = self.resolve_path(path, &base);
                if let Some(fragment) = fragment {
                    new_ref.push('#');
                    new_ref.push_str(fragment);
                }
                let mut copy = self.graph[section].spread();
                copy.set("$ref", Val::Str(new_ref.into()));
                section = self.graph.add(copy);
            }
        }

        if res.is_pre_201909() {
            let keys: Vec<String> = self.graph[target]
                .keys()
                .filter(|k| !RESERVED_KEYS.contains(k))
                .map(str::to_owned)
                .collect();
            for key in keys {
                self.graph[target].remove(&key);
            }
            self.merge(target, section);
        } else if isolate_external_resource || self.needs_scope_isolation(res, section, target) {
            let mut sibling = Obj::default();
            let mut ref_schema = self.graph[section].spread();
            if isolate_external_resource {
                let mut resource_uri = self.handles[section_base.0].uri.clone();
                let section_id = self
                    .graph
                    .get(section, "$id")
                    .filter(|v| v.is_truthy())
                    .or_else(|| self.graph.get(section, "id"));
                if let Some(Val::Str(id)) = section_id
                    && !id.starts_with('#')
                {
                    resource_uri = self.resolve_path(id, &resource_uri);
                }
                ref_schema.remove("$id");
                ref_schema.remove("id");
                ref_schema.original_id = Some(normalize_id(&resource_uri));
            }
            let keys: Vec<String> = self.graph[target]
                .keys()
                .filter(|k| !RESERVED_KEYS.contains(k) && !KEEP_ON_WRAPPER_KEYS.contains(k))
                .map(str::to_owned)
                .collect();
            for key in keys {
                if let Some(v) = self.graph[target].remove(&key) {
                    sibling.set(&key, v);
                }
            }
            let ref_schema = self.graph.add(ref_schema);
            let sibling = self.graph.add(sibling);
            self.graph[target].set(
                "allOf",
                Val::Arr(Arc::new(vec![Val::Obj(ref_schema), Val::Obj(sibling)])),
            );
        } else {
            self.merge(target, section);
        }
    }

    fn get_external_schema_handle(&mut self, uri: &str, parent: HandleId) -> HandleId {
        let parent_uri = self.handles[parent.0].uri.clone();
        let uri = normalize_id(&self.resolve_path(uri, &parent_uri));
        self.get_or_add_schema_handle(&uri)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_external_link(
        &mut self,
        res: &mut Resolution,
        node: ObjId,
        uri: &str,
        ref_segment: Option<&str>,
        parent: HandleId,
        continuation: Option<(ObjId, HandleId)>,
        followed: &mut HashSet<String>,
    ) {
        let referenced = self.get_external_schema_handle(uri, parent);
        let uri = self.handles[referenced.0].uri.clone();
        let unresolved = self.unresolved(referenced);
        self.handles[parent.0].dependencies.insert(uri.clone());
        // Upstream records only the parent, which is the handle of the nearest `$id`. Then
        // `on_resource_change(uri)` does not clear a root schema that has an `$id`.
        self.handles[res.handle.0].dependencies.insert(uri.clone());
        let already_followed = !followed.insert(uri.clone());
        if !already_followed && ref_segment.is_some() {
            let root = &self.graph[unresolved.schema];
            if let Some(root_ref) = self.graph.get_str(unresolved.schema, "$ref")
                && root.keys().all(|k| k == "$ref")
            {
                let (path, fragment) = split_ref(root_ref);
                if !path.is_empty() && fragment.is_none() {
                    let path = path.to_owned();
                    return self.resolve_external_link(
                        res,
                        node,
                        &path,
                        ref_segment,
                        referenced,
                        continuation,
                        followed,
                    );
                }
            }
        }
        if let Some(error) = unresolved.errors.first() {
            let message = match ref_segment.filter(|s| !s.is_empty()) {
                Some(seg) => format!("Problems loading reference '{seg}': {}", error.message),
                None => error.message.clone(),
            };
            res.errors
                .push(to_diagnostic(message, error.code, Some(&uri)));
        }
        if res.embedded_registered_for.insert(uri.clone()) {
            self.register_embedded_schemas(res, unresolved.schema, &uri);
        }
        if self.graph[unresolved.schema].anchor_maps.is_none() {
            self.collect_dynamic_anchors(res, unresolved.schema);
        }
        let isolate = continuation.is_some() && !res.embedded_schema_uris.contains(&uri);
        self.merge_ref(
            res,
            node,
            unresolved.schema,
            referenced,
            ref_segment,
            isolate,
        );
        let maps = self.graph[unresolved.schema].anchor_maps.clone();
        self.union_anchor_maps(node, maps);
        let (base, base_handle) = continuation.unwrap_or((unresolved.schema, referenced));
        self.resolve_refs(res, node, base, base_handle);
    }

    fn resolve_dynamic_ref(
        &mut self,
        res: &mut Resolution,
        schema: ObjId,
        new_base: ObjId,
        new_base_handle: HandleId,
        current_base_handle: HandleId,
    ) {
        let info = self.graph.info_of(schema);
        let Some(Val::Str(r)) = self.graph[schema].remove("$dynamicRef") else {
            return;
        };
        let (path, fragment) = split_ref(&r);
        let name = fragment.filter(|f| !f.is_empty() && !f.starts_with('/'));
        if let Some(name) = name {
            self.graph.info_mut(info).name = Some(name.to_owned());
        }
        if !path.is_empty() {
            let target = self.graph.new_obj();
            self.graph.info_mut(info).target = Some(target);
            let ref_base = if new_base == schema {
                current_base_handle
            } else {
                new_base_handle
            };
            self.resolve_external_link(
                res,
                target,
                path,
                fragment,
                ref_base,
                None,
                &mut HashSet::new(),
            );
            return;
        }
        let mut target = None;
        if let Some(name) = name
            && let Some(scope) = self.graph.info(info).scope
        {
            target = self.graph[scope]
                .anchor_maps
                .as_ref()
                .and_then(|m| m.local.get(name).copied());
        }
        let target = match target {
            Some(t) => t,
            None => {
                let container = self.graph.new_obj();
                self.merge_ref(res, container, new_base, new_base_handle, fragment, false);
                container
            }
        };
        self.graph.info_mut(info).target = Some(target);
    }

    fn resolve_refs(
        &mut self,
        res: &mut Resolution,
        node: ObjId,
        parent_schema: ObjId,
        parent_handle: HandleId,
    ) {
        self.traverse_with_base_tracking(
            res,
            node,
            parent_schema,
            parent_handle,
            &mut HashSet::new(),
        );
    }

    fn traverse_with_base_tracking(
        &mut self,
        res: &mut Resolution,
        schema: ObjId,
        current_base: ObjId,
        current_base_handle: HandleId,
        seen: &mut HashSet<ObjId>,
    ) {
        if !seen.insert(schema) {
            return;
        }
        let mut new_base = current_base;
        let mut new_base_handle = current_base_handle;
        if let Some(id) = self.base_id(schema) {
            let base_uri = self.handles[current_base_handle.0].uri.clone();
            let resolved = normalize_id(&self.resolve_path(&id, &base_uri));
            new_base = schema;
            new_base_handle = self.get_or_add_schema_handle(&resolved);
        }

        let mut seen_refs = HashSet::new();
        while let Some(r) = self
            .graph
            .get(schema, "$ref")
            .filter(|v| v.is_truthy())
            .cloned()
        {
            let is_pre = res.is_pre_201909();
            let has_siblings = !is_pre && self.has_ref_sibling_keywords(schema);
            let preserves_own_base = has_siblings && (new_base == schema || res.draft.is_some());
            self.graph[schema].remove("$ref");
            let Val::Str(r) = r else {
                continue;
            };
            let (path, fragment) = split_ref(&r);
            if !path.is_empty() {
                let ref_base = if is_pre && new_base == schema {
                    current_base_handle
                } else {
                    new_base_handle
                };
                let continuation_handle = if new_base == schema {
                    current_base_handle
                } else {
                    new_base_handle
                };
                let continuation = preserves_own_base.then_some((new_base, continuation_handle));
                self.resolve_external_link(
                    res,
                    schema,
                    path,
                    fragment,
                    ref_base,
                    continuation,
                    &mut HashSet::new(),
                );
                return;
            } else if seen_refs.insert(r.to_string()) {
                self.merge_ref(res, schema, new_base, new_base_handle, fragment, false);
            }
        }

        if self
            .graph
            .get(schema, "$dynamicRef")
            .is_some_and(Val::is_truthy)
            && res.allows_dynamic_ref()
        {
            self.resolve_dynamic_ref(res, schema, new_base, new_base_handle, current_base_handle);
        }
        for child in self.graph.schema_children(schema) {
            self.traverse_with_base_tracking(res, child, new_base, new_base_handle, seen);
        }
    }

    fn collect_anchors(&mut self, res: &mut Resolution, root: ObjId) -> IndexMap<String, ObjId> {
        let draft = match self.graph.get(root, "$schema").filter(|v| v.is_truthy()) {
            Some(v) => v.as_str().and_then(get_schema_draft_from_id),
            None => res.draft,
        };
        let mut result = IndexMap::new();
        let mut seen = HashSet::new();
        let mut stack = vec![(root, true)];
        // Depth-first in the same order as the recursive upstream traversal.
        while let Some((node, is_root)) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            let id = self.schema_id(node);
            let id = id.as_ref().and_then(Val::as_str);
            if !is_root && id.is_some_and(|id| !id.starts_with('#')) {
                continue;
            }
            let o = &self.graph[node];
            let fragment_anchor = if draft.is_none_or(|d| d < SchemaDraft::V2019_09) {
                id.and_then(|id| id.strip_prefix('#'))
            } else {
                None
            };
            let dollar_anchor = if draft.is_none_or(|d| d >= SchemaDraft::V2019_09) {
                o.get("$anchor")
            } else {
                None
            };
            let dynamic_anchor = if draft.is_none_or(|d| d >= SchemaDraft::V2020_12) {
                o.get("$dynamicAnchor")
            } else {
                None
            };
            let first = match fragment_anchor {
                Some(f) => Some(f.to_owned()),
                None => dollar_anchor
                    .filter(|v| v.is_truthy())
                    .and_then(|v| Some(v.as_str()?.to_owned())),
            };
            let second = dynamic_anchor
                .filter(|v| v.is_truthy())
                .and_then(|v| Some(v.as_str()?.to_owned()));
            for anchor in [first, second].into_iter().flatten() {
                if anchor.is_empty() {
                    continue;
                }
                match result.get(&anchor) {
                    Some(&existing) if existing != node => res.errors.push(to_diagnostic(
                        format!("Duplicate anchor declaration: '{anchor}'"),
                        ErrorCode::SCHEMA_RESOLVE_ERROR,
                        None,
                    )),
                    _ => {
                        result.insert(anchor, node);
                    }
                }
            }
            let children = self.graph.schema_children(node);
            stack.extend(children.into_iter().rev().map(|c| (c, false)));
        }
        result
    }

    fn collect_dynamic_anchors(&mut self, res: &Resolution, root: ObjId) {
        let draft = match self.graph.get(root, "$schema").filter(|v| v.is_truthy()) {
            Some(v) => v.as_str().and_then(get_schema_draft_from_id),
            None => res.draft,
        };
        if !draft.is_none_or(|d| d >= SchemaDraft::V2020_12) {
            return;
        }
        let mut seen = HashSet::new();
        let mut stack = vec![(root, root)];
        while let Some((node, resource_root)) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            let current_root = if node == root || self.base_id(node).is_some() {
                self.graph[node].anchor_maps.get_or_insert_default();
                node
            } else {
                resource_root
            };
            let o = &self.graph[node];
            let dynamic_anchor = o
                .get("$dynamicAnchor")
                .and_then(Val::as_str)
                .map(str::to_owned);
            let anchor = o.get("$anchor").and_then(Val::as_str).map(str::to_owned);
            let has_dynamic_ref = o.get("$dynamicRef").and_then(Val::as_str).is_some();
            let maps = self.graph[current_root]
                .anchor_maps
                .as_mut()
                .expect("resource roots have anchor maps");
            if let Some(name) = dynamic_anchor {
                maps.dynamic.entry(name.clone()).or_insert(node);
                maps.local.entry(name).or_insert(node);
            }
            if let Some(name) = anchor {
                maps.local.entry(name).or_insert(node);
            }
            if has_dynamic_ref {
                let info = self.graph.info_of(node);
                self.graph.info_mut(info).scope = Some(current_root);
            }
            let children = self.graph.schema_children(node);
            stack.extend(children.into_iter().rev().map(|c| (c, current_root)));
        }
    }

    fn register_embedded_schemas(&mut self, res: &mut Resolution, root: ObjId, base_uri: &str) {
        let mut seen = HashSet::new();
        let mut stack = vec![(root, base_uri.to_owned(), true)];
        while let Some((node, current_base, is_document_root)) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            let mut new_base = current_base.clone();
            if let Some(id) = self.base_id(node) {
                let resolved = normalize_id(&self.resolve_path(&id, &current_base));
                if !is_document_root {
                    res.embedded_schema_uris.insert(resolved.clone());
                }
                match self.schemas_by_id.get(&resolved).copied() {
                    None => {
                        self.add_schema_handle(&resolved, Some(node));
                    }
                    Some(existing) if existing != res.handle => {
                        self.set_schema_content(existing, node)
                    }
                    Some(_) => {}
                }
                new_base = resolved;
            }
            let children = self.graph.schema_children(node);
            stack.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|c| (c, new_base.clone(), false)),
            );
        }
    }

    fn get_schema_from_property(&self, resource: &str, document: &JsonDocument) -> Option<String> {
        let root = document.root()?;
        if document[root].kind != AstKind::Object {
            return None;
        }
        for &p in document.children(root) {
            if document.key(p) != "$schema" {
                continue;
            }
            if let Some(id) = document.value(p).and_then(|v| document.string(v)) {
                let has_scheme = {
                    let b = id.as_bytes();
                    let word = |c: &u8| c.is_ascii_alphanumeric() || *c == b'_';
                    b.first().is_some_and(word) && {
                        let end = b
                            .iter()
                            .position(|c| !(word(c) || matches!(c, b'+' | b'.' | b'-')))
                            .unwrap_or(b.len());
                        b.get(end) == Some(&b':')
                    }
                };
                return Some(match &self.resolve_relative_path {
                    Some(resolve) if !has_scheme => resolve(id, resource),
                    _ => id.to_owned(),
                });
            }
        }
        None
    }

    fn get_associated_schemas(&self, resource: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut schemas = Vec::new();
        let normalized = normalize_resource_for_matching(resource);
        for &i in &self.file_pattern_associations {
            let entry = &self.associations[i];
            if entry.matches_pattern(&normalized) {
                for id in &entry.uris {
                    if seen.insert(id.clone()) {
                        schemas.push(id.clone());
                    }
                }
            }
        }
        schemas
    }

    fn get_schema_uris_for_resource_impl(&self, resource: &str) -> Vec<String> {
        let mut schemas = self.get_associated_schemas(resource);
        if schemas.is_empty()
            && let Some(fallback) = self
                .fallback_schema_provider
                .as_ref()
                .and_then(|f| f(resource))
        {
            schemas.extend(fallback.iter().map(|s| normalize_id(s)));
        }
        schemas
    }

    pub fn get_schema_uris_for_resource(
        &self,
        resource: &str,
        document: Option<&JsonDocument>,
    ) -> Vec<String> {
        if let Some(id) = document.and_then(|d| self.get_schema_from_property(resource, d))
            && !id.is_empty()
        {
            return vec![id];
        }
        self.get_schema_uris_for_resource_impl(resource)
    }

    pub fn get_schema_for_resource(
        &mut self,
        resource: &str,
        document: Option<&JsonDocument>,
    ) -> Option<Arc<ResolvedSchema>> {
        if let Some(id) = document.and_then(|d| self.get_schema_from_property(resource, d))
            && !id.is_empty()
        {
            let handle = self.get_or_add_schema_handle(&normalize_id(&id));
            return Some(self.resolved(handle));
        }
        if let Some((cached, schema)) = &self.cached_schema_for_resource
            && cached == resource
        {
            return schema.clone();
        }
        let schemas = self.get_schema_uris_for_resource_impl(resource);
        let resolved = if schemas.is_empty() {
            None
        } else {
            let handle = self.create_combined_schema(resource, &schemas);
            Some(self.resolved(handle))
        };
        self.cached_schema_for_resource = Some((resource.to_owned(), resolved.clone()));
        resolved
    }

    fn create_combined_schema(&mut self, resource: &str, schema_ids: &[String]) -> HandleId {
        if let [id] = schema_ids {
            return self.get_or_add_schema_handle(id);
        }
        let id = format!(
            "schemaservice://combinedSchema/{}",
            encode_uri_component(resource)
        );
        let all_of = schema_ids
            .iter()
            .map(|s| {
                let mut r = Obj::default();
                r.set("$ref", Val::Str(s.as_str().into()));
                Val::Obj(self.graph.add(r))
            })
            .collect();
        let mut combined = Obj::default();
        combined.set("allOf", Val::Arr(Arc::new(all_of)));
        let combined = self.graph.add(combined);
        self.add_schema_handle(&id, Some(combined))
    }

    /// Registers an inline schema and resolves it. `id` is the URI to use if the
    /// schema has no `$id` or `id`; `None` gives a new `schemaservice://untitled/` URI.
    pub fn resolve_inline_schema(
        &mut self,
        schema: &Value,
        untitled_prefix: &str,
    ) -> Arc<ResolvedSchema> {
        let id = match schema
            .get("$id")
            .filter(|v| crate::is_truthy(v))
            .or_else(|| schema.get("id").filter(|v| crate::is_truthy(v)))
        {
            Some(Value::String(id)) => id.clone(),
            Some(other) => other.to_string(),
            None => {
                let id = format!("{untitled_prefix}{}", self.id_counter);
                self.id_counter += 1;
                id
            }
        };
        self.register_external_schema(&SchemaConfiguration {
            uri: id.clone(),
            schema: Some(schema.clone()),
            ..SchemaConfiguration::default()
        });
        let handle = self.schemas_by_id[&normalize_id(&id)];
        self.resolved(handle)
    }

    /// `getMatchingSchemas`. Inverted matches are removed.
    pub fn get_matching_schemas(
        &mut self,
        resource: &str,
        document: &JsonDocument,
        schema: Option<&Value>,
    ) -> Option<(Arc<ResolvedSchema>, Vec<ApplicableSchema>)> {
        let resolved = match schema {
            Some(schema) => {
                let id = match schema.get("id").filter(|v| crate::is_truthy(v)) {
                    Some(Value::String(id)) => id.clone(),
                    _ => {
                        let id = format!(
                            "schemaservice://untitled/matchingSchemas/{}",
                            self.id_counter
                        );
                        self.id_counter += 1;
                        id
                    }
                };
                let content = self.content_from_json(schema);
                let handle = self.add_schema_handle(&id, Some(content));
                self.resolved(handle)
            }
            None => self.get_schema_for_resource(resource, Some(document))?,
        };
        let matching = document
            .get_matching_schemas(&resolved.schemas, resolved.root, None, None, None)
            .into_iter()
            .filter(|s| !s.inverted)
            .collect();
        Some((resolved, matching))
    }
}
