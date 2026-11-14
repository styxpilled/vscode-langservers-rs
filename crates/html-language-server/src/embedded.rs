//! Port of `modes/embeddedSupport.ts`. Offsets are byte offsets; the embedded
//! documents keep the lines and the UTF-16 columns of the HTML document.

use html_languageservice::parser::scanner::{Scanner, TokenType};
use lsp_types::{Position, Range, Uri};
use text_document::TextDocument;

pub const CSS_STYLE_RULE: &str = "__";

#[derive(Clone, Debug)]
struct Region {
    language_id: Option<&'static str>,
    start: usize,
    end: usize,
    attribute_value: bool,
    #[cfg_attr(not(any(test, feature = "javascript")), expect(dead_code))]
    is_module: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LanguageRange {
    pub range: Range,
    pub language_id: Option<&'static str>,
    pub attribute_value: bool,
}

/// `HTMLDocumentRegions`.
#[derive(Debug)]
pub struct DocumentRegions {
    regions: Vec<Region>,
}

/// Whether `text` contains `body` between two quotes (`["']body["']`).
fn has_quoted(text: &str, body: &str) -> bool {
    let b = text.as_bytes();
    (0..b.len()).any(|i| {
        matches!(b[i], b'"' | b'\'')
            && text[i + 1..].starts_with(body)
            && matches!(b.get(i + 1 + body.len()), Some(b'"' | b'\''))
    })
}

/// The language of a script `type` attribute value and whether it is a
/// module.
fn script_language(token: &str) -> (Option<&'static str>, bool) {
    let lower = token.to_lowercase();
    if has_quoted(&lower, "module") || lower == "module" {
        return (Some("javascript"), true);
    }
    let javascript = ["text/javascript", "text/ecmascript"]
        .iter()
        .chain(&["application/javascript", "application/ecmascript"])
        .any(|t| has_quoted(token, t));
    let language = if javascript || has_quoted(token, "text/babel") {
        Some("javascript")
    } else if has_quoted(token, "text/typescript") {
        Some("typescript")
    } else {
        None
    };
    (language, false)
}

/// `getAttributeLanguage`: `/^(style)$|^(on\w+)$/i`.
fn attribute_language(name: &str) -> Option<&'static str> {
    if name.eq_ignore_ascii_case("style") {
        return Some("css");
    }
    let b = name.as_bytes();
    (b.len() > 2
        && b[..2].eq_ignore_ascii_case(b"on")
        && b[2..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_'))
    .then_some("javascript")
}

impl DocumentRegions {
    /// `getDocumentRegions`.
    pub fn new(document: &TextDocument) -> Self {
        let text = document.text();
        let mut regions = Vec::new();
        let mut scanner = Scanner::at(text, 0);
        let mut last_tag_name = "";
        let mut last_attribute_name: Option<&str> = None;
        let mut language_from_type = Some("javascript");
        let mut is_module = false;
        let mut token = scanner.scan();
        while token != TokenType::Eos {
            match token {
                TokenType::StartTag => {
                    last_tag_name = scanner.token_text();
                    last_attribute_name = None;
                    language_from_type = Some("javascript");
                    is_module = false;
                }
                TokenType::Styles => regions.push(Region {
                    language_id: Some("css"),
                    start: scanner.token_offset(),
                    end: scanner.token_end(),
                    attribute_value: false,
                    is_module: false,
                }),
                TokenType::Script => regions.push(Region {
                    language_id: language_from_type,
                    start: scanner.token_offset(),
                    end: scanner.token_end(),
                    attribute_value: false,
                    is_module,
                }),
                TokenType::AttributeName => last_attribute_name = Some(scanner.token_text()),
                TokenType::AttributeValue => {
                    let is_script = last_tag_name.eq_ignore_ascii_case("script");
                    if last_attribute_name == Some("src") && is_script {
                        // `importedScripts`: upstream does not read them.
                    } else if last_attribute_name == Some("type") && is_script {
                        (language_from_type, is_module) = script_language(scanner.token_text());
                    } else if let Some(language_id) =
                        last_attribute_name.and_then(attribute_language)
                    {
                        let (mut start, mut end) = (scanner.token_offset(), scanner.token_end());
                        if matches!(text.as_bytes()[start], b'\'' | b'"') {
                            start += 1;
                            // Without the closing quote, the last character.
                            end -= text[..end].chars().next_back().map_or(1, char::len_utf8);
                        }
                        regions.push(Region {
                            language_id: Some(language_id),
                            start,
                            end,
                            attribute_value: true,
                            is_module: false,
                        });
                    }
                    last_attribute_name = None;
                }
                _ => {}
            }
            token = scanner.scan();
        }
        DocumentRegions { regions }
    }

    /// `getLanguageRanges`.
    pub fn language_ranges(&self, document: &TextDocument, range: Range) -> Vec<LanguageRange> {
        let mut result = Vec::new();
        let mut current_pos = range.start;
        let mut current_offset = document.offset_at(range.start);
        let end_offset = document.offset_at(range.end);
        for region in &self.regions {
            if region.end > current_offset && region.start < end_offset {
                let start = region.start.max(current_offset);
                let start_pos = document.position_at(start);
                if current_offset < region.start {
                    result.push(LanguageRange {
                        range: Range::new(current_pos, start_pos),
                        language_id: Some("html"),
                        attribute_value: false,
                    });
                }
                let end = region.end.min(end_offset);
                let end_pos = document.position_at(end);
                if end > region.start {
                    result.push(LanguageRange {
                        range: Range::new(start_pos, end_pos),
                        language_id: region.language_id,
                        attribute_value: region.attribute_value,
                    });
                }
                current_offset = end;
                current_pos = end_pos;
            }
        }
        if current_offset < end_offset {
            result.push(LanguageRange {
                range: Range::new(current_pos, range.end),
                language_id: Some("html"),
                attribute_value: false,
            });
        }
        result
    }

    /// `getLanguagesInDocument`: at most three embedded languages, then `html`
    /// unless three were found.
    pub fn languages_in_document(&self) -> Vec<&'static str> {
        let mut result = Vec::new();
        for id in self.regions.iter().filter_map(|r| r.language_id) {
            if !result.contains(&id) {
                result.push(id);
                if result.len() == 3 {
                    return result;
                }
            }
        }
        result.push("html");
        result
    }

    /// `getLanguageAtPosition`.
    pub fn language_at_position(
        &self,
        document: &TextDocument,
        position: Position,
    ) -> Option<&'static str> {
        let offset = document.offset_at(position);
        for region in &self.regions {
            if region.start > offset {
                break;
            }
            if offset <= region.end {
                return region.language_id;
            }
        }
        Some("html")
    }

    /// `getEmbeddedDocument`.
    pub fn embedded_document(
        &self,
        document: &TextDocument,
        language_id: &str,
        ignore_attribute_values: bool,
    ) -> TextDocument {
        let text = embedded_text(
            document.text(),
            self.regions.iter(),
            language_id,
            ignore_attribute_values,
        );
        TextDocument::new(
            document.uri().clone(),
            language_id,
            document.version(),
            text,
        )
    }

    /// `getEmbeddedDocuments`: without module scripts, one document. Else the
    /// document of the other regions, then one document for each module.
    #[cfg_attr(not(any(test, feature = "javascript")), expect(dead_code))]
    pub fn embedded_documents(
        &self,
        document: &TextDocument,
        language_id: &str,
        ignore_attribute_values: bool,
    ) -> Vec<TextDocument> {
        let modules = || {
            self.regions
                .iter()
                .filter(|r| r.language_id == Some(language_id) && r.is_module)
        };
        if modules().next().is_none() {
            return vec![self.embedded_document(document, language_id, ignore_attribute_values)];
        }
        let new = |uri: Uri, text| TextDocument::new(uri, language_id, document.version(), text);
        let old = document.text();
        let text = embedded_text(
            old,
            self.regions.iter().filter(|r| !r.is_module),
            language_id,
            ignore_attribute_values,
        );
        let mut result = vec![new(document.uri().clone(), text)];
        for (i, region) in modules().enumerate() {
            let mut text = embedded_text(
                old,
                std::iter::once(region),
                language_id,
                ignore_attribute_values,
            );
            text.push_str("\nexport {};");
            let uri = format!("{}.module-{i}", document.uri().as_str());
            result.push(new(uri.parse().unwrap(), text));
        }
        result
    }
}

/// The text of `getEmbeddedDocument` for the regions `contents`.
fn embedded_text<'a>(
    old: &str,
    contents: impl Iterator<Item = &'a Region>,
    language_id: &str,
    ignore_attribute_values: bool,
) -> String {
    let mut result = String::with_capacity(old.len());
    let mut current = 0;
    let mut last_suffix = "";
    for c in contents.filter(|c| {
        c.language_id == Some(language_id) && !(ignore_attribute_values && c.attribute_value)
    }) {
        substitute_with_whitespace(&mut result, &old[current..c.start], last_suffix, prefix(c));
        // `substring` swaps its arguments: a value that is only a quote
        // gives `start == end + 1`.
        let content = &old[c.start.min(c.end)..c.start.max(c.end)];
        update_content(&mut result, c, content);
        current = c.end;
        last_suffix = suffix(c);
    }
    substitute_with_whitespace(&mut result, &old[current..], last_suffix, "");
    result
}

fn prefix(c: &Region) -> &'static str {
    if c.attribute_value && c.language_id == Some("css") {
        "__{"
    } else {
        ""
    }
}

fn suffix(c: &Region) -> &'static str {
    match (c.attribute_value, c.language_id) {
        (true, Some("css")) => "}",
        (true, Some("javascript")) => ";",
        _ => "",
    }
}

/// `updateContent`, appended to `result`.
fn update_content(result: &mut String, c: &Region, content: &str) {
    match c.language_id {
        Some("javascript") if !c.attribute_value => replace_html_comments(result, content),
        Some("css") => replace_quote_escapes(result, content),
        _ => result.push_str(content),
    }
}

/// `<!--x-->` on one line becomes `/* x */`, one character shorter.
fn replace_html_comments(result: &mut String, content: &str) {
    let mut rest = content;
    while let Some(i) = rest.find("<!--") {
        let body = &rest[i + 4..];
        let line_end = body
            .find(['\r', '\n', '\u{2028}', '\u{2029}'])
            .unwrap_or(body.len());
        let Some(end) = body[..line_end].find("-->") else {
            result.push_str(&rest[..i + 4]);
            rest = body;
            continue;
        };
        result.push_str(&rest[..i]);
        result.push_str("/* ");
        result.push_str(&body[..end]);
        result.push_str(" */");
        rest = &body[end + 3..];
    }
    result.push_str(rest);
}

fn replace_quote_escapes(result: &mut String, content: &str) {
    let mut rest = content;
    while let Some(i) = rest.find('&') {
        let tail = &rest[i..];
        let Some(len) = ["&quot;", "&#34;"]
            .iter()
            .find(|e| tail.starts_with(**e))
            .map(|e| e.len())
        else {
            result.push_str(&rest[..=i]);
            rest = &rest[i + 1..];
            continue;
        };
        result.push_str(&rest[..i]);
        let spaces = " ".repeat(len - 1);
        if tail[len..].chars().next().is_none_or(|c| c == ' ') {
            result.push_str(&spaces);
            result.push('"');
        } else {
            result.push('"');
            result.push_str(&spaces);
        }
        rest = &tail[len..];
    }
    result.push_str(rest);
}

/// `substituteWithWhitespace`: keeps the line breaks of `old`, other
/// characters become spaces (one for each UTF-16 code unit).
fn substitute_with_whitespace(result: &mut String, old: &str, before: &str, after: &str) {
    result.push_str(before);
    let mut accumulated = -(before.encode_utf16().count() as isize);
    for c in old.chars() {
        if c == '\n' || c == '\r' {
            accumulated = 0;
            result.push(c);
        } else {
            accumulated += c.len_utf16() as isize;
        }
    }
    let n = accumulated - after.encode_utf16().count() as isize;
    if n > 0 {
        result.extend(std::iter::repeat_n(' ', n as usize));
    }
    result.push_str(after);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(text: &str) -> TextDocument {
        TextDocument::new(
            "test://test.html".parse().unwrap(),
            "html",
            0,
            text.to_owned(),
        )
    }

    #[test]
    fn script_types() {
        for (token, expected) in [
            ("\"MODULE\"", (Some("javascript"), true)),
            ("module", (Some("javascript"), true)),
            ("'text/babel'", (Some("javascript"), false)),
            ("\"application/ecmascript'", (Some("javascript"), false)),
            ("\"Text/JavaScript\"", (None, false)),
            ("\"text/typescript\"", (Some("typescript"), false)),
            ("text/javascript", (None, false)),
        ] {
            assert_eq!(script_language(token), expected, "{token}");
        }
    }

    /// `embedded.test.ts`: script content and module documents.
    #[test]
    fn script_content() {
        for (value, expected) in [
            (
                "<html><script>var i = 0;</script></html>",
                "              var i = 0;                ",
            ),
            (
                "<script type=\"text/javascript\">var i = 0;</script>",
                "                               var i = 0;         ",
            ),
            (
                "<script><!--this comment should not give error--></script>",
                "        /* this comment should not give error */         ",
            ),
            (
                "<script><!--this comment should not give error--> console.log(\"logging\");</script>",
                "        /* this comment should not give error */ console.log(\"logging\");         ",
            ),
            (
                "<script>var data=100; <!--this comment should not give error--> </script>",
                "        var data=100; /* this comment should not give error */          ",
            ),
            ("<script>/* --> */</script>", "        /* --> */         "),
            (
                "<script>/* --> ----------------------------- */</script>",
                "        /* --> ----------------------------- */         ",
            ),
            (
                "<script><!--commnet1-->\n<!--conment2--></script>",
                "        /* commnet1 */\n/* conment2 */         ",
            ),
            (
                "<script><!--a--> foo(); <!--b--></script>",
                "        /* a */ foo(); /* b */         ",
            ),
            (
                "<div onKeyUp=\"foo()\" onkeydown=\"bar()\"/>",
                "              foo();            bar();  ",
            ),
            ("<div onKeyUp=\"return\"/>", "              return;  "),
            (
                "<div onKeyUp=return\n/><script>foo();</script>",
                "             return;\n          foo();         ",
            ),
            (
                "<script>let a = 1;</script><script type=\"module\">let a = 2;</script>",
                "        let a = 1;                               let a = 2;         ",
            ),
        ] {
            let document = document(value);
            let regions = DocumentRegions::new(&document);
            let embedded = regions.embedded_document(&document, "javascript", false);
            assert_eq!(embedded.text(), expected, "{value}");
        }
        let document =
            document("<script>let a = 1;</script><script type=\"module\">let a = 2;</script>");
        let documents =
            DocumentRegions::new(&document).embedded_documents(&document, "javascript", false);
        let texts: Vec<_> = documents.iter().map(|d| d.text()).collect();
        assert_eq!(
            texts,
            [
                "        let a = 1;                                                  ",
                "                                                 let a = 2;         \nexport {};"
            ]
        );
        assert_eq!(documents[1].uri().as_str(), "test://test.html.module-0");
    }

    #[test]
    fn attribute_languages() {
        assert_eq!(attribute_language("STYLE"), Some("css"));
        assert_eq!(attribute_language("onClick"), Some("javascript"));
        assert_eq!(attribute_language("on"), None);
        assert_eq!(attribute_language("on-x"), None);
    }

    #[test]
    fn quote_escapes() {
        let mut result = String::new();
        replace_quote_escapes(&mut result, "a: &quot;x&quot; &#34;&amp");
        assert_eq!(result, "a: \"     x     \" \"    &amp");
    }

    #[test]
    fn html_comments() {
        for (content, expected) in [
            ("a<!--b-->c<!---->", "a/* b */c/*  */"),
            ("<!--a\n-->", "<!--a\n-->"),
            ("<!--<!--a-->", "/* <!--a */"),
            ("<!--a\u{2028}--><!--b-->", "<!--a\u{2028}-->/* b */"),
        ] {
            let mut result = String::new();
            replace_html_comments(&mut result, content);
            assert_eq!(result, expected, "{content:?}");
        }
    }

    /// Results of upstream.
    #[test]
    fn unterminated_attribute_values() {
        for (text, css) in [
            (
                "<b style=color:red\n>\n<style>a{}</style>",
                "      __{color:red}\n\n       a{}        ",
            ),
            ("<b style=\"", "       __{\"}"),
            ("<b style=\"ab\n", "       __{ab}\n"),
            // Upstream keeps the high surrogate: "       __{a\u{D83D}}".
            ("<b style=\"a😀", "       __{a} "),
        ] {
            let document = document(text);
            let embedded =
                DocumentRegions::new(&document).embedded_document(&document, "css", false);
            assert_eq!(embedded.text(), css, "{text:?}");
        }
    }

    /// `embedded.test.ts`, without the JavaScript content tests.
    #[test]
    fn language_at_position() {
        for (value, expected) in [
            ("|<html><style>foo { }</style></html>", Some("html")),
            ("<html|><style>foo { }</style></html>", Some("html")),
            ("<html><st|yle>foo { }</style></html>", Some("html")),
            ("<html><style>|foo { }</style></html>", Some("css")),
            ("<html><style>foo| { }</style></html>", Some("css")),
            ("<html><style>foo { }|</style></html>", Some("css")),
            ("<html><style>foo { }</sty|le></html>", Some("html")),
            ("|<html><style>foo { }", Some("html")),
            ("<html><style>fo|o { }", Some("css")),
            ("<html><style>foo { }|", Some("css")),
            ("<div id=\"xy\" |style=\"color: red\"/>", Some("html")),
            ("<div id=\"xy\" styl|e=\"color: red\"/>", Some("html")),
            ("<div id=\"xy\" style=|\"color: red\"/>", Some("html")),
            ("<div id=\"xy\" style=\"|color: red\"/>", Some("css")),
            ("<div id=\"xy\" style=\"color|: red\"/>", Some("css")),
            ("<div id=\"xy\" style=\"color: red|\"/>", Some("css")),
            ("<div id=\"xy\" style=\"color: red\"|/>", Some("html")),
            ("<div id=\"xy\" style=\'color: r|ed\'/>", Some("css")),
            ("<div id=\"xy\" style|=color:red/>", Some("html")),
            ("<div id=\"xy\" style=|color:red/>", Some("css")),
            ("<div id=\"xy\" style=color:r|ed/>", Some("css")),
            ("<div id=\"xy\" style=color:red|/>", Some("css")),
            ("<div id=\"xy\" style=color:red/|>", Some("html")),
            ("|<html><script>var i = 0;</script></html>", Some("html")),
            ("<html|><script>var i = 0;</script></html>", Some("html")),
            ("<html><scr|ipt>var i = 0;</script></html>", Some("html")),
            (
                "<html><script>|var i = 0;</script></html>",
                Some("javascript"),
            ),
            (
                "<html><script>var| i = 0;</script></html>",
                Some("javascript"),
            ),
            (
                "<html><script>var i = 0;|</script></html>",
                Some("javascript"),
            ),
            ("<html><script>var i = 0;</scr|ipt></html>", Some("html")),
            (
                "<script type=\"text/javascript\">var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<script type=\"text/ecmascript\">var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<script type=\"application/javascript\">var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<script type=\"application/ecmascript\">var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<script type=\"application/typescript\">var| i = 0;</script>",
                None,
            ),
            (
                "<script type=\'text/javascript\'>var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<div |onKeyUp=\"foo()\" onkeydown=\'bar()\'/>",
                Some("html"),
            ),
            (
                "<div onKeyUp=|\"foo()\" onkeydown=\'bar()\'/>",
                Some("html"),
            ),
            (
                "<div onKeyUp=\"|foo()\" onkeydown=\'bar()\'/>",
                Some("javascript"),
            ),
            (
                "<div onKeyUp=\"foo(|)\" onkeydown=\'bar()\'/>",
                Some("javascript"),
            ),
            (
                "<div onKeyUp=\"foo()|\" onkeydown=\'bar()\'/>",
                Some("javascript"),
            ),
            (
                "<div onKeyUp=\"foo()\"| onkeydown=\'bar()\'/>",
                Some("html"),
            ),
            (
                "<div onKeyUp=\"foo()\" onkeydown=|\'bar()\'/>",
                Some("html"),
            ),
            (
                "<div onKeyUp=\"foo()\" onkeydown=\'|bar()\'/>",
                Some("javascript"),
            ),
            (
                "<div onKeyUp=\"foo()\" onkeydown=\'bar()|\'/>",
                Some("javascript"),
            ),
            (
                "<div onKeyUp=\"foo()\" onkeydown=\'bar()\'|/>",
                Some("html"),
            ),
            ("<DIV ONKEYUP|=foo()</DIV>", Some("html")),
            ("<DIV ONKEYUP=|foo()</DIV>", Some("javascript")),
            ("<DIV ONKEYUP=f|oo()</DIV>", Some("javascript")),
            ("<DIV ONKEYUP=foo(|)</DIV>", Some("javascript")),
            ("<DIV ONKEYUP=foo()|</DIV>", Some("javascript")),
            ("<DIV ONKEYUP=foo()<|/DIV>", Some("html")),
            ("<label data-content=\"|Checkbox\"/>", Some("html")),
            ("<label on=\"|Checkbox\"/>", Some("html")),
            (
                "<script type=\"module\">var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<script type=module>var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<script type=\"Module\">var| i = 0;</script>",
                Some("javascript"),
            ),
            (
                "<script type=MODULE>var| i = 0;</script>",
                Some("javascript"),
            ),
        ] {
            let offset = value.find('|').unwrap();
            let text = value.replacen('|', "", 1);
            let document = document(&text);
            let position = document.position_at(offset);
            let regions = DocumentRegions::new(&document);
            assert_eq!(
                regions.language_at_position(&document, position),
                expected,
                "{value}"
            );
        }
    }

    #[test]
    fn style_content() {
        for (value, expected) in [
            (
                "<html><style>foo { }</style></html>",
                "             foo { }               ",
            ),
            (
                "<html><script>var i = 0;</script></html>",
                "                                        ",
            ),
            (
                "<html><style>foo { }</style>Hello<style>foo { }</style></html>",
                "             foo { }                    foo { }               ",
            ),
            (
                "<html>\n  <style>\n    foo { }  \n  </style>\n</html>\n",
                "\n         \n    foo { }  \n  \n\n",
            ),
            (
                "<div style=\"color: red\"></div>",
                "         __{color: red}       ",
            ),
            ("<div style=color:red></div>", "        __{color:red}      "),
            (
                "<div style=\"font-family: &quot;Arial&quot;\"></div>",
                "         __{font-family: \"     Arial     \"}       ",
            ),
            (
                "<div style=\"font-family: &#34;Arial&#34;\"></div>",
                "         __{font-family: \"    Arial    \"}       ",
            ),
            (
                "<div style=\"font-family: &quot;Arial&#34;\"></div>",
                "         __{font-family: \"     Arial    \"}       ",
            ),
            (
                "<div style=\"font-family:&quot; Arial &quot; \"></div>",
                "         __{font-family:     \" Arial      \" }       ",
            ),
            (
                "<div style=\"font-family: Arial\"></div>",
                "         __{font-family: Arial}       ",
            ),
        ] {
            let document = document(value);
            let regions = DocumentRegions::new(&document);
            assert_eq!(
                regions.embedded_document(&document, "css", false).text(),
                expected,
                "{value}"
            );
        }
    }
}
