//! Port of `requests.ts`, `node/nodeFs.ts` and `utils/documentContext.ts`.

use std::io::ErrorKind;

use css_languageservice::services::links::{
    DocumentContext, FileStat, FileSystemProvider, FileType,
};
use vscode_uri::{Uri, dirname, resolve_path};

/// `getRequestService` with the Node file system for the `file` scheme. Upstream sends
/// `fs/stat`, `fs/readDir` and `fs/content` to the client for other schemes. Here these
/// calls fail, as when the client rejects them.
pub struct RequestService {
    pub handled_schemas: Vec<String>,
}

impl RequestService {
    fn file_path(&self, uri: &str) -> Option<String> {
        let scheme = uri.find(':').map_or("", |i| &uri[..i]);
        if scheme != "file" || !self.handled_schemas.iter().any(|s| s == "file") {
            return None;
        }
        // `ensureFileUri`.
        if !uri.starts_with("file://") {
            return None;
        }
        Some(Uri::parse(uri)?.fs_path())
    }
}

impl FileSystemProvider for RequestService {
    fn stat(&self, uri: &str) -> Option<FileStat> {
        let path = self.file_path(uri)?;
        match std::fs::metadata(path) {
            Ok(m) => Some(FileStat {
                file_type: if m.is_file() {
                    FileType::File
                } else if m.is_dir() {
                    FileType::Directory
                } else {
                    FileType::Unknown
                },
                size: m.len() as i64,
            }),
            Err(e) if e.kind() == ErrorKind::NotFound => Some(FileStat {
                file_type: FileType::Unknown,
                size: -1,
            }),
            Err(_) => None,
        }
    }

    fn read_directory(&self, uri: &str) -> Option<Vec<(String, FileType)>> {
        let path = self.file_path(uri)?;
        let mut result = Vec::new();
        for entry in std::fs::read_dir(path).ok()? {
            let entry = entry.ok()?;
            let file_type = match entry.file_type() {
                Ok(t) if t.is_symlink() => FileType::SymbolicLink,
                Ok(t) if t.is_dir() => FileType::Directory,
                Ok(t) if t.is_file() => FileType::File,
                _ => FileType::Unknown,
            };
            result.push((entry.file_name().to_string_lossy().into_owned(), file_type));
        }
        Some(result)
    }

    fn get_content(&self, uri: &str) -> Option<String> {
        let bytes = std::fs::read(self.file_path(uri)?).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// `getDocumentContext`.
pub struct WorkspaceContext<'a> {
    pub document_uri: &'a str,
    pub workspace_folders: &'a [String],
}

impl WorkspaceContext<'_> {
    fn root_folder(&self) -> Option<String> {
        self.workspace_folders.iter().find_map(|folder| {
            let mut folder = folder.clone();
            if !folder.ends_with('/') {
                folder.push('/');
            }
            self.document_uri.starts_with(&folder).then_some(folder)
        })
    }
}

impl DocumentContext for WorkspaceContext<'_> {
    /// `None` also where upstream throws (a `base` that `URI.parse` rejects).
    fn resolve_reference(&self, reference: &str, base: &str) -> Option<String> {
        if let Some(path) = reference.strip_prefix('/')
            && let Some(folder) = self.root_folder()
        {
            return Some(folder + path);
        }
        let base = Uri::parse(base)?;
        let dir = if base.path.ends_with('/') {
            base
        } else {
            dirname(&base)?
        };
        Some(resolve_path(&dir, reference)?.to_string_unencoded())
    }
}

/// Port of the server tests `links.test.ts` and `completion.test.ts`. The fixtures are a
/// copy of the upstream `test` folder.
#[cfg(test)]
mod tests {
    use css_languageservice::language_facts::CssDataManager;
    use css_languageservice::parser::{Dialect, parse_stylesheet};
    use css_languageservice::services::completion::Completion;
    use css_languageservice::services::links::LinkResolver;
    use lsp_types::CompletionTextEdit;
    use text_document::TextDocument;

    use super::*;

    fn resource(path: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        Uri::file(root.join(path).to_str().unwrap()).to_string_unencoded()
    }

    fn file_system() -> RequestService {
        RequestService {
            handled_schemas: vec!["file".into()],
        }
    }

    #[test]
    fn stat() {
        let fs = file_system();
        let stat = |path: &str| fs.stat(&resource(path)).map(|s| (s.file_type, s.size));
        assert_eq!(stat("missing.css"), Some((FileType::Unknown, -1)));
        assert_eq!(
            stat("pathCompletionFixtures/index.html"),
            Some((FileType::File, 0))
        );
        assert_eq!(
            stat("pathCompletionFixtures").map(|s| s.0),
            Some(FileType::Directory)
        );
        assert_eq!(fs.stat("untitled:x"), None);
        let other = RequestService {
            handled_schemas: vec!["http".into()],
        };
        assert_eq!(other.stat(&resource("pathCompletionFixtures")), None);
    }

    /// `(offset, value, target)`.
    fn assert_links(value: &str, expected: &[(usize, &str, &str)], uri: &str, folder: &str) {
        let offset = value.find('|').unwrap();
        let value = format!("{}{}", &value[..offset], &value[offset + 1..]);
        let document = TextDocument::new(uri.parse().unwrap(), "css", 0, value);
        let folders = [folder.to_owned()];
        let context = WorkspaceContext {
            document_uri: uri,
            workspace_folders: &folders,
        };
        let stylesheet = parse_stylesheet(Dialect::Css, document.text());
        let fs = file_system();
        let resolver = LinkResolver {
            dialect: Dialect::Css,
            file_system: Some(&fs),
            aliases: None,
        };
        let links = resolver.find_document_links2(&document, &stylesheet, &context);
        assert_eq!(links.len(), expected.len(), "{links:?}");
        for &(offset, value, target) in expected {
            let matches: Vec<_> = links
                .iter()
                .filter(|l| document.offset_at(l.range.start) == offset)
                .collect();
            assert_eq!(matches.len(), 1, "{offset}: {links:?}");
            assert_eq!(document.text_in(matches[0].range), value);
            assert_eq!(matches[0].target, target);
        }
    }

    #[test]
    fn links() {
        let uri = resource("linksTestFixtures/about.css");
        let folder = resource("linksTestFixtures");
        let hello = resource("linksTestFixtures/hello.html");
        let value = r#"html { background-image: url("hello.html|")"#;
        assert_links(value, &[(29, "\"hello.html\"", &hello)], &uri, &folder);

        let value = r#"@import url("base.css|");")"#;
        let expected = [(12, "\"base.css\"", "untitled:base.css")];
        assert_links(value, &expected, "untitled:untitled-1", &folder);

        let target = resource("linksTestFixtures/node_modules/foo/hello.html");
        let value = r#"html { background-image: url("~foo/hello.html|")"#;
        let expected = [(29, "\"~foo/hello.html\"", target.as_str())];
        assert_links(value, &expected, &uri, &folder);
        let uri = resource("linksTestFixtures/subdir/about.css");
        assert_links(value, &expected, &uri, &folder);
    }

    /// `(label, result text)`.
    fn assert_completions(
        value: &str,
        count: Option<usize>,
        expected: &[(&str, &str)],
        uri: &str,
        language: &str,
    ) {
        let offset = value.find('|').unwrap();
        let value = format!("{}{}", &value[..offset], &value[offset + 1..]);
        let document = TextDocument::new(uri.parse().unwrap(), language, 0, value);
        let dialect = if language == "scss" {
            Dialect::Scss
        } else {
            Dialect::Css
        };
        let folders = [resource("")];
        let context = WorkspaceContext {
            document_uri: uri,
            workspace_folders: &folders,
        };
        let stylesheet = parse_stylesheet(dialect, document.text());
        let data = CssDataManager::new(true, &[]);
        let completion = Completion {
            dialect,
            data: &data,
            markdown: true,
        };
        let fs = file_system();
        let list = completion
            .do_complete2(
                &document,
                document.position_at(offset),
                &stylesheet,
                &context,
                Some(&fs),
                None,
                &mut [],
            )
            .unwrap();
        if let Some(count) = count {
            assert_eq!(list.items.len(), count);
        }
        for &(label, result) in expected {
            let matches: Vec<_> = list.items.iter().filter(|i| i.label == label).collect();
            let labels: Vec<_> = list.items.iter().map(|i| &i.label).collect();
            assert_eq!(matches.len(), 1, "{label}: {labels:?}");
            if let Some(CompletionTextEdit::Edit(edit)) = &matches[0].text_edit {
                assert_eq!(document.apply_edits(std::slice::from_ref(edit)), result);
            }
        }
    }

    #[test]
    fn path_completion() {
        let uri = resource("pathCompletionFixtures/about/about.css");
        let check = |value: &str, expected: &[(&str, &str)]| {
            assert_completions(value, None, expected, &uri, "css")
        };
        let p = |url: &str| format!("html {{ background-image: url({url})");
        check(&p("\"./|\""), &[("about.html", &p("\"./about.html\""))]);
        check(
            &p("'../|'"),
            &[
                ("about/", &p("'../about/'")),
                ("index.html", &p("'../index.html'")),
                ("src/", &p("'../src/'")),
            ],
        );
        check(
            &p("'../src/a|'"),
            &[
                ("feature.js", &p("'../src/feature.js'")),
                ("data/", &p("'../src/data/'")),
                ("test.js", &p("'../src/test.js'")),
            ],
        );
        check(
            &p("'../src/data/f|.asar'"),
            &[("foo.asar", &p("'../src/data/foo.asar'"))],
        );
        check(&p("'|'"), &[("about.html", &p("'about.html'"))]);
        check(
            &p("'/|'"),
            &[("pathCompletionFixtures/", &p("'/pathCompletionFixtures/'"))],
        );
        check(
            &p("'/pathCompletionFixtures/|'"),
            &[
                ("about/", &p("'/pathCompletionFixtures/about/'")),
                ("index.html", &p("'/pathCompletionFixtures/index.html'")),
                ("src/", &p("'/pathCompletionFixtures/src/'")),
            ],
        );
        check(
            &p("\"/|\""),
            &[(
                "pathCompletionFixtures/",
                &p("\"/pathCompletionFixtures/\""),
            )],
        );

        // Unquoted.
        check(&p("./|"), &[("about.html", &p("./about.html"))]);
        check(&p("./a|"), &[("about.html", &p("./about.html"))]);
        check(&p("../|src/"), &[("about/", &p("../about/"))]);
        check(&p("../s|rc/"), &[("about/", &p("../about/"))]);

        // `@import`.
        check("@import './|'", &[("about.html", "@import './about.html'")]);
        check(
            "@import '../|'",
            &[
                ("about/", "@import '../about/'"),
                ("scss/", "@import '../scss/'"),
                ("index.html", "@import '../index.html'"),
                ("src/", "@import '../src/'"),
            ],
        );
        // A CSS file: no special case for SCSS partials.
        check(
            "@import '../scss/|'",
            &[
                ("main.scss", "@import '../scss/main.scss'"),
                ("_foo.scss", "@import '../scss/_foo.scss'"),
            ],
        );
        let scss = resource("pathCompletionFixtures/scss/main.scss");
        let expected = [("_foo.scss", "@import './foo'")];
        assert_completions("@import './|'", None, &expected, &scss, "scss");

        // Files and folders that start with a dot are ignored.
        assert_completions(&p("\"../|\""), Some(4), &[], &uri, "css");
    }
}
