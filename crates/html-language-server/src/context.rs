//! Port of `utils/documentContext.ts`.

use css_languageservice::services::links::DocumentContext;
use vscode_uri::{Uri, dirname, resolve_path};

/// `getDocumentContext`.
pub struct WorkspaceContext<'a> {
    pub document_uri: &'a str,
    pub workspace_folders: &'a [String],
}

/// `/^\w[\w\d+.-]*:/`.
fn has_scheme(reference: &str) -> bool {
    let b = reference.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    if !b.first().is_some_and(|&c| word(c)) {
        return false;
    }
    b[1..]
        .iter()
        .find(|&&c| !(word(c) || matches!(c, b'+' | b'.' | b'-')))
        == Some(&b':')
}

fn scheme_of(uri: &str) -> Option<String> {
    Some(Uri::parse(uri)?.scheme.to_lowercase())
}

impl DocumentContext for WorkspaceContext<'_> {
    /// `None` also where upstream throws (a `base` that `URI.parse` rejects).
    fn resolve_reference(&self, reference: &str, base: &str) -> Option<String> {
        if has_scheme(reference) {
            return Some(reference.to_owned());
        }
        if let Some(path) = reference.strip_prefix('/') {
            let folder = self.workspace_folders.iter().find_map(|folder| {
                let mut folder = folder.clone();
                if !folder.ends_with('/') {
                    folder.push('/');
                }
                self.document_uri.starts_with(&folder).then_some(folder)
            });
            if let Some(folder) = folder {
                return Some(folder + path);
            }
        }
        let mut base = Uri::parse(base)?;
        base.scheme = base.scheme.to_lowercase();
        // `isHierarchicalBase`: a `<base href>` with another scheme, for
        // example `command:`, does not resolve relative references.
        if !matches!(base.scheme.as_str(), "http" | "https" | "file")
            && Some(&base.scheme) != scheme_of(self.document_uri).as_ref()
        {
            return None;
        }
        let dir = if base.path.ends_with('/') {
            base
        } else {
            dirname(&base)?
        };
        Some(resolve_path(&dir, reference)?.to_string_unencoded())
    }
}

/// Port of `documentContext.test.ts`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context() {
        let doc = "file:///users/test/folder/test.html";
        let folders = ["file:///users/test/".to_owned()];
        let c = WorkspaceContext {
            document_uri: doc,
            workspace_folders: &folders,
        };
        let r = |a: &str, b: &str| c.resolve_reference(a, b);
        assert_eq!(r("/", doc).as_deref(), Some("file:///users/test/"));
        assert_eq!(
            r("/message.html", doc).as_deref(),
            Some("file:///users/test/message.html")
        );
        assert_eq!(
            r("message.html", doc).as_deref(),
            Some("file:///users/test/folder/message.html")
        );
        assert_eq!(
            r("message.html", "file:///users/test/").as_deref(),
            Some("file:///users/test/message.html")
        );
    }

    #[test]
    fn hierarchical_bases() {
        let doc = "file:///users/test/folder/test.html";
        let folders = ["file:///users/test/".to_owned()];
        let c = WorkspaceContext {
            document_uri: doc,
            workspace_folders: &folders,
        };
        let actual: Vec<Option<String>> = [
            (
                "workbench.action.quit",
                "command:workbench.action.quit?%5B%5D",
            ),
            ("foo", "COMMAND:foo?%5B%5D"),
            ("foo", "javascript:alert(1)"),
            ("page.html", "https://example.com/docs/index.html"),
            ("page.html", "http://example.com/docs/"),
            ("page.html", "file:///users/test/other/"),
            ("page.html", "HTTPS://example.com/docs/"),
            ("page.html", "Http://example.com/docs/"),
            ("page.html", "FILE:///users/test/other/"),
        ]
        .iter()
        .map(|(a, b)| c.resolve_reference(a, b))
        .collect();
        let expected = [
            None,
            None,
            None,
            Some("https://example.com/docs/page.html"),
            Some("http://example.com/docs/page.html"),
            Some("file:///users/test/other/page.html"),
            Some("https://example.com/docs/page.html"),
            Some("http://example.com/docs/page.html"),
            Some("file:///users/test/other/page.html"),
        ];
        assert_eq!(actual, expected.map(|e| e.map(str::to_owned)));

        let untitled = WorkspaceContext {
            document_uri: "untitled:Untitled-1",
            workspace_folders: &[],
        };
        for base in ["untitled:Untitled-1", "UNTITLED:Untitled-1"] {
            assert_eq!(
                untitled.resolve_reference("page.html", base).as_deref(),
                Some("untitled:page.html")
            );
        }
    }
}
