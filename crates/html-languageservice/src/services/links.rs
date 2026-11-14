//! Port of `htmlLinks.ts`.

use std::collections::HashMap;

use css_languageservice::services::links::{DocumentContext, DocumentLink};
use text_document::TextDocument;
use vscode_uri::Uri;

use crate::language_facts::HtmlDataManager;
use crate::parser::scanner::{Scanner, TokenType};
use crate::utils::is_js_whitespace;

fn normalize_ref(url: &str) -> &str {
    let b = url.as_bytes();
    if b.len() >= 2 && b[0] == b[b.len() - 1] && matches!(b[0], b'\'' | b'"') {
        return &url[1..url.len() - 1];
    }
    url
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `validateRef`. The regular expression
/// `/\b(w[\w\d+.-]*:\/\/)?[^\s()<>]+(?:\([\w\d]+\)|([^[:punct:]\s]|\/?))/`
/// matches if a word boundary is before a character that is not white space
/// or `()<>`.
fn validate_ref(url: &str, language_id: &str) -> bool {
    if url.is_empty() {
        return false;
    }
    if language_id == "handlebars" && (url.contains("{{") || url.contains("}}")) {
        return false;
    }
    let mut prev = None;
    for c in url.chars() {
        if is_word(prev) != is_word(Some(c))
            && !is_js_whitespace(c)
            && !matches!(c, '(' | ')' | '<' | '>')
        {
            return true;
        }
        prev = Some(c);
    }
    false
}

fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// `/^(\w[\w\d+.-]*):/`, group 1.
fn scheme_of(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if !b
        .first()
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        return None;
    }
    let n = 1 + b[1..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'+' | b'.' | b'-'))
        .count();
    (b.get(n) == Some(&b':')).then(|| &s[..n])
}

fn workspace_url(
    document_uri: &str,
    token_content: &str,
    context: &dyn DocumentContext,
    base: Option<&str>,
) -> Option<String> {
    if starts_with_ignore_ascii_case(
        token_content.trim_start_matches(is_js_whitespace),
        "javascript:",
    ) || token_content.contains(['\n', '\r'])
    {
        return None;
    }
    let token_content = token_content.trim_start_matches(is_js_whitespace);
    if let Some(scheme) = scheme_of(token_content) {
        // An absolute link.
        let scheme = scheme.to_ascii_lowercase();
        if scheme == "http" || scheme == "https" || scheme == "file" {
            return Some(token_content.to_owned());
        }
        return None;
    }
    if token_content.starts_with('#') {
        return Some(format!("{document_uri}{token_content}"));
    }
    if token_content.starts_with("//") {
        // An absolute link without scheme.
        let scheme = if document_uri.starts_with("https://") {
            "https"
        } else {
            "http"
        };
        return Some(format!("{scheme}:{token_content}"));
    }
    context.resolve_reference(
        token_content,
        base.filter(|b| !b.is_empty()).unwrap_or(document_uri),
    )
}

fn create_link(
    document: &TextDocument,
    context: &dyn DocumentContext,
    attribute_value: &str,
    mut start: usize,
    mut end: usize,
    base: Option<&str>,
) -> Option<DocumentLink> {
    let token_content = normalize_ref(attribute_value);
    if !validate_ref(token_content, document.language_id()) {
        return None;
    }
    if token_content.len() < attribute_value.len() {
        start += 1;
        end -= 1;
    }
    let url = workspace_url(document.uri().as_str(), token_content, context, base)
        .filter(|u| !u.is_empty())?;
    let target = validate_and_clean_uri(url, document.uri().as_str())?;
    Some(DocumentLink {
        target,
        range: document.range(start, end),
    })
}

/// `validateAndCleanURI`. Upstream returns a link without target where
/// `URI.parse` throws; here there is no link.
fn validate_and_clean_uri(mut uri_str: String, document_uri: &str) -> Option<String> {
    let mut uri = Uri::parse(&uri_str)?;
    if uri.scheme == "file" && !uri.query.is_empty() {
        // microsoft/vscode#194577 and microsoft/vscode#206238
        uri.query.clear();
        uri_str = uri.to_string_unencoded();
    }
    if uri.scheme == "file"
        && !uri.fragment.is_empty()
        && !(uri_str.starts_with(document_uri)
            && uri_str.as_bytes().get(document_uri.len()) == Some(&b'#'))
    {
        uri.fragment.clear();
        return Some(uri.to_string_unencoded());
    }
    Some(uri_str)
}

/// `findDocumentLinks`.
pub fn find_document_links(
    document: &TextDocument,
    context: &dyn DocumentContext,
) -> Vec<DocumentLink> {
    let text = document.text();
    let mut links: Vec<DocumentLink> = Vec::new();
    let mut scanner = Scanner::at(text, 0);
    let mut token = scanner.scan();
    let mut last_attribute_name: Option<String> = None;
    let mut last_tag_name: Option<String> = None;
    let mut after_base = false;
    let mut base: Option<String> = None;
    let mut id_locations: HashMap<&str, usize> = HashMap::new();

    while token != TokenType::Eos {
        match token {
            TokenType::StartTag => {
                let tag = scanner.token_text().to_lowercase();
                if base.as_deref().is_none_or(str::is_empty) {
                    after_base = tag == "base";
                }
                last_tag_name = Some(tag);
            }
            TokenType::AttributeName => {
                last_attribute_name = Some(scanner.token_text().to_lowercase());
            }
            TokenType::AttributeValue => {
                let value = scanner.token_text();
                if let (Some(tag), Some(attr)) = (&last_tag_name, &last_attribute_name)
                    && !tag.is_empty()
                    && !attr.is_empty()
                    && HtmlDataManager::is_path_attribute(tag, attr)
                {
                    if !after_base
                        && let Some(link) = create_link(
                            document,
                            context,
                            value,
                            scanner.token_offset(),
                            scanner.token_end(),
                            base.as_deref(),
                        )
                    {
                        links.push(link);
                    }
                    if after_base && base.is_none() {
                        let b = normalize_ref(value);
                        base = if b.is_empty() {
                            Some(String::new())
                        } else {
                            context.resolve_reference(b, document.uri().as_str())
                        };
                    }
                    after_base = false;
                    last_attribute_name = None;
                } else if last_attribute_name.as_deref() == Some("id") {
                    id_locations.insert(normalize_ref(value), scanner.token_offset());
                }
            }
            _ => {}
        }
        token = scanner.scan();
    }
    // Local links with ids point to the positions.
    let local_with_hash = format!("{}#", document.uri().as_str());
    for link in &mut links {
        if let Some(target) = link.target.strip_prefix(&local_with_hash) {
            link.target = match id_locations.get(target) {
                Some(&offset) => {
                    let pos = document.position_at(offset);
                    format!("{local_with_hash}{},{}", pos.line + 1, pos.character + 1)
                }
                None => document.uri().as_str().to_owned(),
            };
        }
    }
    links
}
