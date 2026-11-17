//! Port of the file system parts of `cssNavigation.ts` and of
//! `scssNavigation.ts`: document links and module resolution. Upstream is
//! async; here the file system calls are synchronous.

use std::collections::HashMap;

use lsp_types::Range;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use text_document::TextDocument;

use crate::parser::nodes::{Ast, NodeId, NodeType};
use crate::parser::{Dialect, Stylesheet};
use crate::utils::{Thrown, dirname, join_path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Unknown,
    File,
    Directory,
    SymbolicLink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub file_type: FileType,
    pub size: i64,
}

/// `FileSystemProvider`. `None` where the upstream provider throws.
pub trait FileSystemProvider {
    fn stat(&self, uri: &str) -> Option<FileStat>;
    fn read_directory(&self, uri: &str) -> Option<Vec<(String, FileType)>>;
    fn get_content(&self, uri: &str) -> Option<String>;
}

/// `DocumentContext`.
pub trait DocumentContext {
    fn resolve_reference(&self, reference: &str, base_url: &str) -> Option<String>;
}

/// `AliasSettings` (`importAliases`).
pub type AliasSettings = HashMap<String, String>;

/// `DocumentLink`. The target is a string: it can be an invalid URI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentLink {
    pub target: String,
    pub range: Range,
}

/// The file system parts of `CSSNavigation` and `SCSSNavigation`.
/// `resolveModuleReferences` is `true` for all dialects.
pub struct LinkResolver<'a> {
    /// `Dialect::Scss` selects `SCSSNavigation`.
    pub dialect: Dialect,
    pub file_system: Option<&'a dyn FileSystemProvider>,
    pub aliases: Option<&'a AliasSettings>,
}

impl LinkResolver<'_> {
    /// `findDocumentLinks`.
    pub fn find_document_links(
        &self,
        document: &TextDocument,
        stylesheet: &Stylesheet,
        context: &dyn DocumentContext,
    ) -> Vec<DocumentLink> {
        let mut resolved = Vec::new();
        for (mut link, _) in self.find_unresolved_links(document, stylesheet) {
            if link.target.is_empty() || link.target.starts_with("data:") {
                continue;
            }
            if !starts_with_scheme(&link.target)
                && let Some(target) =
                    context.resolve_reference(&link.target, document.uri().as_str())
            {
                link.target = target;
            }
            resolved.push(link);
        }
        resolved
    }

    /// `findDocumentLinks2`. A link whose resolution throws upstream is left
    /// out; upstream rejects and gives no links.
    pub fn find_document_links2(
        &self,
        document: &TextDocument,
        stylesheet: &Stylesheet,
        context: &dyn DocumentContext,
    ) -> Vec<DocumentLink> {
        let mut resolved = Vec::new();
        for (mut link, is_raw_link) in self.find_unresolved_links(document, stylesheet) {
            if link.target.is_empty() || link.target.starts_with("data:") {
                continue;
            }
            if starts_with_scheme(&link.target) {
                resolved.push(link);
                continue;
            }
            let uri = document.uri().as_str();
            if let Ok(Some(target)) =
                self.resolve_reference(&link.target, uri, context, is_raw_link)
            {
                link.target = target;
                resolved.push(link);
            }
        }
        resolved
    }

    fn is_raw_string_document_link_node(&self, ty: NodeType) -> bool {
        ty == NodeType::Import
            || (self.dialect == Dialect::Scss && matches!(ty, NodeType::Use | NodeType::Forward))
    }

    fn find_unresolved_links(
        &self,
        document: &TextDocument,
        stylesheet: &Stylesheet,
    ) -> Vec<(DocumentLink, bool)> {
        let ast = &stylesheet.ast;
        let mut result = Vec::new();
        let mut collect = |ast: &Ast, node: NodeId| {
            let start = ast.offset(node).max(0) as usize;
            let end = ast.end(node).max(0) as usize;
            let range = document.range(start, end);
            if range.start == range.end {
                return;
            }
            let mut raw_uri = ast.get_text(node);
            if raw_uri.starts_with(['\'', '"']) {
                raw_uri = strip_first_and_last(raw_uri);
            }
            let is_raw_link = ast
                .parent(node)
                .is_some_and(|p| self.is_raw_string_document_link_node(ast.node_type(p)));
            result.push((
                DocumentLink {
                    target: raw_uri.to_owned(),
                    range,
                },
                is_raw_link,
            ));
        };
        ast.accept(stylesheet.root, &mut |candidate| {
            if ast.node_type(candidate) == NodeType::URILiteral {
                if let Some(first) = ast.child(candidate, 0) {
                    collect(ast, first);
                }
                return false;
            }
            // `@import 'foo.css';` is a link without `url()`.
            if ast
                .parent(candidate)
                .is_some_and(|p| self.is_raw_string_document_link_node(ast.node_type(p)))
            {
                if ast.get_text(candidate).starts_with(['\'', '"']) {
                    collect(ast, candidate);
                }
                return false;
            }
            true
        });
        result
    }

    fn resolve_reference(
        &self,
        target: &str,
        document_uri: &str,
        context: &dyn DocumentContext,
        is_raw_link: bool,
    ) -> Result<Option<String>, Thrown> {
        if self.dialect == Dialect::Scss {
            if target.starts_with("sass:") {
                return Ok(None);
            }
            // The Sass package importer: `pkg:module`.
            if target.starts_with("pkg:") {
                return self.resolve_pkg_module_path(target, document_uri, context);
            }
        }

        // css-loader and sass-loader: `~` uses node module resolution,
        // except `~/` (the home directory).
        if target.starts_with('~')
            && target.as_bytes().get(1) != Some(&b'/')
            && self.file_system.is_some()
        {
            let module = self.resolve_module_reference(&target[1..], document_uri, context)?;
            return self.map_reference(module, is_raw_link);
        }

        let reference =
            self.map_reference(context.resolve_reference(target, document_uri), is_raw_link)?;
        let non_empty = reference.as_deref().filter(|r| !r.is_empty());

        // less-loader and sass-loader: try the relative path first, then
        // `node_modules`.
        if non_empty.is_some_and(|r| self.file_exists(r)) {
            return Ok(reference);
        }
        let module = self.resolve_module_reference(target, document_uri, context)?;
        if let Some(module) = self
            .map_reference(module, is_raw_link)?
            .filter(|m| !m.is_empty())
        {
            return Ok(Some(module));
        }

        // The `importAliases` settings.
        if let Some(r) = non_empty
            && !self.file_exists(r)
            && let Some(settings) = self.aliases
            && let Some(root) = context
                .resolve_reference("/", document_uri)
                .filter(|r| !r.is_empty())
        {
            if let Some(path) = settings.get(target) {
                return self.map_reference(Some(join_path(&root, &[path])?), is_raw_link);
            }
            // A folder: `"@dir/": "src/dir/"`.
            let first_slash = target.find('/');
            let prefix = format!("{}/", &target[..first_slash.unwrap_or(0)]);
            if let Some(alias) = settings.get(&prefix) {
                let mut alias_chars = alias.chars();
                alias_chars.next_back();
                let new_path = join_path(&root, &[alias_chars.as_str()])?;
                let rest = &target[first_slash.unwrap_or(0)..];
                return self.map_reference(Some(join_path(&new_path, &[rest])?), is_raw_link);
            }
        }

        // The fallback; it might not exist.
        Ok(reference)
    }

    fn map_reference(
        &self,
        target: Option<String>,
        is_raw_link: bool,
    ) -> Result<Option<String>, Thrown> {
        if self.dialect == Dialect::Scss
            && self.file_system.is_some()
            && is_raw_link
            && let Some(t) = target.as_deref().filter(|t| !t.is_empty())
        {
            for variation in to_path_variations(t)? {
                if self.file_exists(&variation) {
                    return Ok(Some(variation));
                }
            }
        }
        Ok(target)
    }

    fn resolve_module_reference(
        &self,
        reference: &str,
        document_uri: &str,
        context: &dyn DocumentContext,
    ) -> Result<Option<String>, Thrown> {
        if !document_uri.starts_with("file://") {
            return Ok(None);
        }
        let module_name = get_module_name_from_path(reference);
        if module_name.is_empty() || module_name == "." || module_name == ".." {
            return Ok(None);
        }
        let root = context.resolve_reference("/", document_uri);
        let document_folder = dirname(document_uri)?;
        let Some(module_path) =
            self.resolve_path_to_module(module_name, document_folder, root.as_deref())?
        else {
            return Ok(None);
        };
        let path_within_module = reference.get(module_name.len() + 1..).unwrap_or("");
        Ok(Some(join_path(&module_path, &[path_within_module])?))
    }

    /// `resolvePathToModule`: the `node_modules` folders from the document
    /// folder up to the root folder.
    fn resolve_path_to_module(
        &self,
        module_name: &str,
        mut document_folder: String,
        root: Option<&str>,
    ) -> Result<Option<String>, Thrown> {
        loop {
            let pack_path = join_path(
                &document_folder,
                &["node_modules", module_name, "package.json"],
            )?;
            if self.file_exists(&pack_path) {
                return Ok(Some(dirname(&pack_path)?));
            }
            match root {
                Some(root)
                    if !root.is_empty()
                        && document_folder.starts_with(root)
                        && document_folder != root =>
                {
                    document_folder = dirname(&document_folder)?;
                }
                _ => return Ok(None),
            }
        }
    }

    fn file_exists(&self, uri: &str) -> bool {
        self.file_system
            .and_then(|fs| fs.stat(uri))
            .is_some_and(|stat| !(stat.file_type == FileType::Unknown && stat.size == -1))
    }

    fn resolve_pkg_module_path(
        &self,
        target: &str,
        document_uri: &str,
        context: &dyn DocumentContext,
    ) -> Result<Option<String>, Thrown> {
        let bare_target = &target["pkg:".len()..];
        let module_name = if bare_target.contains('/') {
            get_module_name_from_path(bare_target)
        } else {
            bare_target
        };
        let root = context.resolve_reference("/", document_uri);
        let document_folder = dirname(document_uri)?;
        let Some(module_path) =
            self.resolve_path_to_module(module_name, document_folder, root.as_deref())?
        else {
            return Ok(None);
        };
        // Submodule export paths do not match the file system: read `package.json`.
        let package_path = join_path(&module_path, &["package.json"])?;
        let content = self
            .file_system
            .and_then(|fs| fs.get_content(&package_path));
        let Some(content) = content.filter(|c| !c.is_empty()) else {
            return Ok(None);
        };
        // Upstream throws for `null` content and for an entry that is not a string.
        let Ok(package_json @ Value::Object(_)) = serde_json::from_str::<Value>(&content) else {
            return Ok(None);
        };
        let entry_path = |entry: Option<&Value>| -> Result<Option<String>, Thrown> {
            match entry {
                Some(Value::String(entry)) if entry.ends_with(".scss") => {
                    Ok(Some(join_path(&module_path, &[entry])?))
                }
                _ => Ok(None),
            }
        };
        let subpath = bare_target.get(module_name.len() + 1..).unwrap_or("");
        let exports = package_json.get("exports").filter(|e| truthy(e));
        if let Some(exports) = exports {
            if subpath.is_empty() {
                // `{ "sass": "./_index.scss" }` or `{ ".": { "sass": "./_index.scss" } }`.
                let root_export = exports.get(".").filter(|e| truthy(e)).unwrap_or(exports);
                return entry_path(first_truthy(root_export, &["sass", "style", "default"]));
            }
            // With or without `.scss`, in the import and in the exports.
            let lookup_subpath = match subpath.strip_suffix(".scss") {
                Some(_) => format!("./{}", subpath.replacen(".scss", "", 1)),
                None => format!("./{subpath}"),
            };
            let lookup_subpath_scss = match subpath.strip_suffix(".scss") {
                Some(_) => format!("./{subpath}"),
                None => format!("./{subpath}.scss"),
            };
            let subpath_object = exports
                .get(&lookup_subpath_scss)
                .filter(|e| truthy(e))
                .or_else(|| exports.get(&lookup_subpath).filter(|e| truthy(e)));
            if let Some(subpath_object) = subpath_object {
                return entry_path(first_truthy(subpath_object, &["sass", "styles", "default"]));
            }
            // A subpath pattern: https://nodejs.org/api/packages.html#subpath-patterns.
            let Value::Object(map) = exports else {
                return Ok(None);
            };
            for (pattern, subpath_object) in map {
                if !pattern.contains('*') {
                    continue;
                }
                let source = convert_simple_to_regexp_pattern(&pattern.replacen(".scss", "", 1))
                    .replace(".*", "(.*)");
                let Ok(re) = regress::Regex::new(&source) else {
                    continue;
                };
                let Some(m) = re.find(&lookup_subpath) else {
                    continue;
                };
                let captured = m.group(1).map_or("", |r| &lookup_subpath[r]);
                if let Some(Value::String(entry)) =
                    first_truthy(subpath_object, &["sass", "styles", "default"])
                    && entry.ends_with(".scss")
                {
                    // The right side of a subpath pattern is a pattern too.
                    let expanded = entry.replacen('*', captured, 1);
                    return Ok(Some(join_path(&module_path, &[&expanded])?));
                }
            }
        } else if subpath.is_empty() {
            // `sass` and `style` at the root of `package.json`.
            if let Some(Value::String(entry)) = first_truthy(&package_json, &["sass", "style"]) {
                return Ok(Some(join_path(&module_path, &[entry])?));
            }
        }
        Ok(None)
    }
}

/// `startsWithSchemeRegex`: `/^\w+:\/\//`.
fn starts_with_scheme(s: &str) -> bool {
    let word = s
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    word > 0 && s[word..].starts_with("://")
}

/// `slice(1, -1)`.
fn strip_first_and_last(s: &str) -> &str {
    let mut chars = s.chars();
    chars.next();
    chars.next_back();
    chars.as_str()
}

/// JS truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `v[keys[0]] || v[keys[1]] || ...`. Only objects have these properties.
fn first_truthy<'v>(v: &'v Value, keys: &[&str]) -> Option<&'v Value> {
    keys.iter().filter_map(|k| v.get(k)).find(|e| truthy(e))
}

/// `convertSimple2RegExpPattern`.
fn convert_simple_to_regexp_pattern(pattern: &str) -> String {
    let mut out = String::new();
    for c in pattern.chars() {
        if c == '*' {
            out.push_str(".*");
            continue;
        }
        if "-\\{}+?|^$.,[]()#".contains(c) || crate::utils::is_js_whitespace(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `toPathVariations`: the Sass file names for an import.
fn to_path_variations(target: &str) -> Result<Vec<String>, Thrown> {
    // No variation for a `.css` link.
    if target.ends_with(".css") {
        return Ok(vec![target.to_owned()]);
    }
    // `a/`: `a/index.scss` and `a/_index.scss`.
    if target.ends_with('/') {
        return Ok(vec![
            format!("{target}index.scss"),
            format!("{target}_index.scss"),
        ]);
    }
    let target_uri =
        vscode_uri::Uri::parse(target.strip_suffix(".scss").unwrap_or(target)).ok_or(Thrown)?;
    let basename = vscode_uri::basename(&target_uri);
    let dirname = vscode_uri::dirname(&target_uri).ok_or(Thrown)?;
    let join = |name: String| -> Result<String, Thrown> {
        Ok(vscode_uri::join_path(&dirname, &[&name])
            .ok_or(Thrown)?
            .to_string_unencoded())
    };
    if basename.starts_with('_') {
        // No variation for `_a`.
        return Ok(vec![join(format!("{basename}.scss"))?]);
    }
    Ok(vec![
        join(format!("{basename}.scss"))?,
        join(format!("_{basename}.scss"))?,
        format!("{target}/index.scss"),
        format!("{target}/_index.scss"),
        join(format!("{basename}.css"))?,
    ])
}

/// `getModuleNameFromPath`.
pub fn get_module_name_from_path(path: &str) -> &str {
    let Some(first_slash) = path.find('/') else {
        return path;
    };
    // A scoped module (`@scope/name`): up to the second `/`.
    if path.starts_with('@') {
        return match path[first_slash + 1..].find('/') {
            Some(i) => &path[..first_slash + 1 + i],
            None => path,
        };
    }
    &path[..first_slash]
}
