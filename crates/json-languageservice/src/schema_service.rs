//! Port of `services/jsonSchemaService.ts`.
//!
//! Upstream loads a schema with all the schemas that it references, and resolves `$ref` by
//! merging the target into the referencing schema. Here each schema document is loaded on
//! first use and kept as is; `$ref` is resolved when a schema node is compiled
//! ([`crate::schema::Schemas`]). An operation that meets a `$ref` to a document that is not
//! loaded runs again after the load ([`JsonSchemaService::run`]).

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use lsp_types::{DiagnosticRelatedInformation, Location, Range};
use regress::Regex;
use serde_json::Value;

use crate::glob::create_regex;
use crate::schema::doc::Doc;
use crate::schema::tree::{Kind, Tree};
use crate::schema::{ResolveFn, SchemaId, SchemaRef, Schemas};
use crate::validator::{ApplicableSchema, get_schema_draft_from_id};
use crate::vocabularies::Vocabularies;
use crate::{AstKind, ErrorCode, JsonDocument, SchemaDraft};
use vscode_uri::Uri;
pub(crate) use vscode_uri::decode_uri_component;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaRequestError {
    pub message: String,
    /// Added to [`ErrorCode::SCHEMA_RESOLVE_ERROR`] if below `0x10000`. Can be negative.
    pub code: Option<i64>,
}

pub type SchemaRequestService = Box<dyn FnMut(&str) -> Result<String, SchemaRequestError> + Send>;
/// `WorkspaceContextService.resolveRelativePath(relativePath, resource)`.
pub type ResolveRelativePath = Box<dyn Fn(&str, &str) -> String + Send + Sync>;
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

pub(crate) fn to_diagnostic(
    message: String,
    code: ErrorCode,
    related_url: Option<&str>,
) -> SchemaDiagnostic {
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
    pub schemas: Arc<Schemas>,
    pub root: SchemaId,
    /// The errors of the root document. `Schemas::errors` has the `$ref` errors.
    pub errors: Vec<SchemaDiagnostic>,
    pub warnings: Vec<SchemaDiagnostic>,
    pub schema_draft: Option<SchemaDraft>,
    pub active_vocabularies: Option<Vocabularies>,
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
pub(crate) fn has_scheme(s: &str) -> bool {
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
pub(crate) fn split_ref(r: &str) -> (&str, Option<&str>) {
    let mut parts = r.split('#');
    let first = parts.next().unwrap_or("");
    (first, parts.next())
}

pub struct JsonSchemaService {
    /// The loaded documents with their base ids, in the order of the base ids.
    docs: Vec<(Arc<Doc>, u32)>,
    next_base: u32,
    snapshot: Option<Arc<Schemas>>,
    resolved: HashMap<String, Arc<ResolvedSchema>>,
    /// Schema content given as JSON: contributions, settings, combined and inline schemas.
    contents: HashMap<String, Arc<Value>>,
    contribution_ids: IndexMap<String, ()>,
    contribution_associations: Vec<usize>,
    associations: Vec<FilePatternAssociation>,
    file_pattern_associations: Vec<usize>,
    registered_schema_ids: IndexMap<String, ()>,
    request_service: Option<SchemaRequestService>,
    resolve: Option<Arc<ResolveFn>>,
    fallback_schema_provider: Option<FallbackSchemaProvider>,
    cached_schema_for_resource: Option<(String, Option<Arc<ResolvedSchema>>)>,
    id_counter: u32,
    glob_cache: GlobCache,
}

impl JsonSchemaService {
    pub fn new(
        request_service: Option<SchemaRequestService>,
        resolve_relative_path: Option<ResolveRelativePath>,
        fallback_schema_provider: Option<FallbackSchemaProvider>,
    ) -> Self {
        JsonSchemaService {
            docs: Vec::new(),
            next_base: 2,
            snapshot: None,
            resolved: HashMap::new(),
            contents: HashMap::new(),
            contribution_ids: IndexMap::new(),
            contribution_associations: Vec::new(),
            associations: Vec::new(),
            file_pattern_associations: Vec::new(),
            registered_schema_ids: IndexMap::new(),
            request_service,
            resolve: resolve_relative_path.map(Arc::from),
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

    /// The current snapshot of the loaded documents.
    pub fn schemas(&mut self) -> Arc<Schemas> {
        self.snapshot
            .get_or_insert_with(|| Arc::new(Schemas::new(&self.docs, self.resolve.clone())))
            .clone()
    }

    fn changed(&mut self) {
        self.snapshot = None;
        self.resolved.clear();
        self.cached_schema_for_resource = None;
    }

    /// Removes the loaded documents for which `drop` is true. Returns whether one was removed.
    fn drop_docs(&mut self, drop: impl Fn(&Doc) -> bool) -> bool {
        let before = self.docs.len();
        self.docs.retain(|(doc, _)| !drop(doc));
        let removed = self.docs.len() < before;
        if removed {
            self.changed();
        }
        removed
    }

    /// `onResourceChange`. Returns whether a loaded schema was removed. Documents that
    /// reference it need no change: `$ref` is resolved again in the next snapshot.
    pub fn on_resource_change(&mut self, uri: &str) -> bool {
        self.cached_schema_for_resource = None;
        let uri = normalize_id(uri);
        self.drop_docs(|doc| doc.uri == uri)
    }

    pub fn set_schema_contributions(&mut self, contributions: &SchemaContributions) {
        for (id, schema) in &contributions.schemas {
            let id = normalize_id(id);
            self.set_content(&id, schema);
            self.contribution_ids.insert(id, ());
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

    fn set_content(&mut self, id: &str, schema: &Value) {
        if self.contents.get(id).is_some_and(|c| **c == *schema) {
            return;
        }
        self.contents
            .insert(id.to_owned(), Arc::new(schema.clone()));
        self.drop_docs(|doc| doc.uri == id);
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
        if let Some(schema) = &config.schema {
            self.set_content(&id, schema);
        }
    }

    /// Removes all schemas and configuration, then registers `contributions`.
    pub fn reset(&mut self, contributions: &SchemaContributions) {
        let glob_cache = std::mem::take(&mut self.glob_cache);
        *self = JsonSchemaService::new(
            self.request_service.take(),
            None,
            self.fallback_schema_provider.take(),
        )
        .with_resolve(self.resolve.take());
        self.glob_cache = glob_cache;
        self.set_schema_contributions(contributions);
        self.clear_external_schemas();
    }

    fn with_resolve(mut self, resolve: Option<Arc<ResolveFn>>) -> Self {
        self.resolve = resolve;
        self
    }

    pub fn clear_external_schemas(&mut self) {
        self.file_pattern_associations.clear();
        self.registered_schema_ids.clear();
        let contributions = &self.contribution_ids;
        self.contents.retain(|id, _| contributions.contains_key(id));
        for id in self.contribution_ids.keys() {
            self.registered_schema_ids.insert(id.clone(), ());
        }
        self.file_pattern_associations
            .extend(self.contribution_associations.iter().copied());
        let contributions = self.contribution_ids.clone();
        self.drop_docs(|doc| !contributions.contains_key(&doc.uri));
        self.changed();
    }

    pub fn get_resolved_schema(&mut self, schema_id: &str) -> Option<Arc<ResolvedSchema>> {
        let id = normalize_id(schema_id);
        let known = self.registered_schema_ids.contains_key(&id)
            || self.contents.contains_key(&id)
            || self.schemas().resource(&id).is_some();
        known.then(|| self.resolve(&id))
    }

    /// Loads the documents that the last operation needed. Returns whether one was loaded.
    pub fn load_missing(&mut self) -> bool {
        let Some(snapshot) = self.snapshot.clone() else {
            return false;
        };
        let mut loaded = false;
        for uri in snapshot.missing() {
            if snapshot.resource(&uri).is_none() && !self.docs.iter().any(|(d, _)| d.uri == uri) {
                self.load(&uri);
                loaded = true;
            }
        }
        loaded
    }

    /// Runs `f`, and again while it needs documents that are not loaded.
    pub fn run<T>(&mut self, mut f: impl FnMut(&mut Self) -> T) -> T {
        loop {
            let result = f(self);
            if !self.load_missing() {
                return result;
            }
        }
    }

    fn load(&mut self, uri: &str) {
        let (tree, errors) = match self.contents.get(uri) {
            Some(value) => (Tree::from_json(value), Vec::new()),
            None => self.fetch(uri),
        };
        let doc = Doc::new(uri.to_owned(), tree, errors, self.resolve.as_deref());
        let len = doc.tree.len().max(1) as u32;
        self.docs.push((Arc::new(doc), self.next_base));
        self.next_base += len;
        self.changed();
    }

    fn fetch(&mut self, url: &str) -> (Tree, Vec<SchemaDiagnostic>) {
        let fail = |message: String, code: ErrorCode| {
            (
                Tree::default(),
                vec![to_diagnostic(message, code, Some(url))],
            )
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
                let (tree, first_error) = Tree::parse(content);
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
                (tree, errors)
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

    /// The resolved schema for a normalized URI. Loads its document if needed.
    pub fn resolve(&mut self, uri: &str) -> Arc<ResolvedSchema> {
        if let Some(r) = self.resolved.get(uri) {
            return r.clone();
        }
        let mut set = self.schemas();
        let (d, n) = match set.resource(uri) {
            Some(found) => found,
            None => {
                self.load(uri);
                set = self.schemas();
                set.resource(uri).expect("a loaded document is a resource")
            }
        };
        let doc = set.doc(d);
        let tree = &doc.tree;
        let draft = tree
            .get_truthy(n, "$schema")
            .and_then(|v| get_schema_draft_from_id(tree.str(v)?))
            .or(if n == 0 { doc.draft } else { None });
        let resolved = if draft == Some(SchemaDraft::V3) {
            ResolvedSchema {
                schemas: set.clone(),
                root: SchemaId::TRUE,
                errors: vec![to_diagnostic(
                    "Draft-03 schemas are not supported.".to_owned(),
                    ErrorCode::SCHEMA_UNSUPPORTED_FEATURE,
                    None,
                )],
                warnings: Vec::new(),
                schema_draft: draft,
                active_vocabularies: None,
            }
        } else {
            let mut errors = doc.load_errors.clone();
            errors.extend(doc.errors.iter().cloned());
            let metaschema = tree
                .get(n, "$schema")
                .and_then(|v| tree.str(v))
                .map(|m| (m.to_owned(), doc.base(n, false).to_owned()));
            let active_vocabularies =
                metaschema.and_then(|(m, base)| self.resolve_metaschema_vocabularies(&m, &base));
            // Loads append documents, so `d` stays valid.
            let set = self.schemas();
            ResolvedSchema {
                root: set.node_ref(d, n, false).as_schema(),
                schemas: set,
                errors,
                warnings: Vec::new(),
                schema_draft: draft,
                active_vocabularies,
            }
        };
        let resolved = Arc::new(resolved);
        self.resolved.insert(uri.to_owned(), resolved.clone());
        resolved
    }

    fn resolve_metaschema_vocabularies(
        &mut self,
        metaschema: &str,
        base: &str,
    ) -> Option<Vocabularies> {
        let uri = normalize_id(&match &self.resolve {
            Some(resolve) if !has_scheme(metaschema) => resolve(metaschema, base),
            _ => metaschema.to_owned(),
        });
        let mut set = self.schemas();
        if set.resource(&uri).is_none() {
            self.load(&uri);
            set = self.schemas();
        }
        let (d, n) = set.resource(&uri)?;
        let tree = &set.doc(d).tree;
        let meta_draft = tree
            .get_truthy(n, "$schema")
            .and_then(|v| get_schema_draft_from_id(tree.str(v)?));
        let is_2019_or_later = meta_draft.is_some_and(|d| d >= SchemaDraft::V2019_09);
        let vocabulary = tree.get(n, "$vocabulary");
        let has_vocabulary =
            vocabulary.is_some_and(|v| matches!(tree.kind(v), Kind::Obj(..) | Kind::Arr(..)));
        if !has_vocabulary && !is_2019_or_later {
            return None;
        }
        let vocabulary = vocabulary?;
        let vocabs: Vocabularies = match tree.kind(vocabulary) {
            Kind::Obj(..) => tree
                .entries(vocabulary)
                .map(|(uri, required)| {
                    (
                        uri.to_owned(),
                        matches!(tree.kind(required), Kind::Bool(true)),
                    )
                })
                .collect(),
            Kind::Arr(..) => tree
                .items(vocabulary)
                .iter()
                .enumerate()
                .map(|(i, &required)| {
                    (
                        i.to_string(),
                        matches!(tree.kind(required), Kind::Bool(true)),
                    )
                })
                .collect(),
            _ => return None,
        };
        (!vocabs.is_empty()).then_some(vocabs)
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
                return Some(match &self.resolve {
                    Some(resolve) if !has_scheme => resolve(id, resource),
                    _ => id.to_owned(),
                });
            }
        }
        None
    }

    fn get_associated_schemas(&self, resource: &str) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
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
            return Some(self.resolve(&normalize_id(&id)));
        }
        if let Some((cached, schema)) = &self.cached_schema_for_resource
            && cached == resource
        {
            return schema.clone();
        }
        let schemas = self.get_schema_uris_for_resource_impl(resource);
        let resolved = match schemas.as_slice() {
            [] => None,
            [id] => Some(self.resolve(id)),
            ids => {
                let id = format!(
                    "schemaservice://combinedSchema/{}",
                    encode_uri_component(resource)
                );
                let all_of: Vec<Value> = ids
                    .iter()
                    .map(|s| serde_json::json!({ "$ref": s }))
                    .collect();
                self.set_content(&id, &serde_json::json!({ "allOf": all_of }));
                Some(self.resolve(&id))
            }
        };
        self.cached_schema_for_resource = Some((resource.to_owned(), resolved.clone()));
        resolved
    }

    /// Registers an inline schema and resolves it. `untitled_prefix` and a counter give the
    /// URI if the schema has no `$id` or `id`.
    pub fn resolve_inline_schema(
        &mut self,
        schema: &Value,
        untitled_prefix: &str,
    ) -> Arc<ResolvedSchema> {
        let uri = self.register_inline_schema(schema, untitled_prefix);
        self.run(|s| s.resolve(&uri))
    }

    /// Registers an inline schema. Returns its normalized URI.
    pub fn register_inline_schema(&mut self, schema: &Value, untitled_prefix: &str) -> String {
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
        normalize_id(&id)
    }

    /// `getMatchingSchemas`. Inverted matches are removed.
    pub fn get_matching_schemas(
        &mut self,
        resource: &str,
        document: &JsonDocument,
        schema: Option<&Value>,
    ) -> Option<(Arc<ResolvedSchema>, Vec<ApplicableSchema>)> {
        let id = schema.map(
            |schema| match schema.get("id").filter(|v| crate::is_truthy(v)) {
                Some(Value::String(id)) => normalize_id(id),
                _ => {
                    let id = format!(
                        "schemaservice://untitled/matchingSchemas/{}",
                        self.id_counter
                    );
                    self.id_counter += 1;
                    id
                }
            },
        );
        if let (Some(id), Some(schema)) = (&id, schema) {
            self.set_content(id, schema);
        }
        self.run(|s| {
            let resolved = match &id {
                Some(id) => s.resolve(id),
                None => s.get_schema_for_resource(resource, Some(document))?,
            };
            let matching = document
                .get_matching_schemas(&resolved.schemas, resolved.root, None, None, None)
                .into_iter()
                .filter(|s| !s.inverted)
                .collect();
            Some((resolved, matching))
        })
    }
}
