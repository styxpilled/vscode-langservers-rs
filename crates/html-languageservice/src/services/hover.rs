//! Port of `htmlHover.ts`.

use lsp_types::{Hover, HoverContents, MarkedString, MarkupContent, MarkupKind, Position, Range};
use text_document::TextDocument;

use crate::LanguageService;
use crate::language_facts::{Description, HoverSettings, generate_documentation};
use crate::parser::HtmlDocument;
use crate::parser::entities::ENTITIES;
use crate::parser::scanner::{Scanner, TokenType};
use crate::utils::{is_letter_or_digit, utf16_len};

impl LanguageService {
    /// `doHover`.
    pub fn do_hover(
        &self,
        document: &TextDocument,
        position: Position,
        html_document: &HtmlDocument,
        options: Option<HoverSettings>,
    ) -> Option<Hover> {
        let options = options.unwrap_or_default();
        let markdown = self.hover_markdown;
        let offset = document.offset_at(position);
        let node = &html_document[html_document.find_node_at(offset)];
        let text = document.text();
        let node_tag = node.tag.as_deref()?;
        let providers = self.data_manager.data_providers();

        let convert = |mut contents: MarkupContent| {
            if !markdown {
                contents.kind = MarkupKind::PlainText;
            }
            HoverContents::Markup(contents)
        };

        let tag_hover = |curr_tag: &str, range: Range| -> Option<Hover> {
            for provider in providers {
                let mut hover = None;
                for tag in provider.provide_tags() {
                    if tag.name.to_lowercase() == curr_tag.to_lowercase() {
                        let markup = generate_documentation(tag, options, markdown).unwrap_or(
                            MarkupContent {
                                kind: if markdown {
                                    MarkupKind::Markdown
                                } else {
                                    MarkupKind::PlainText
                                },
                                value: String::new(),
                            },
                        );
                        hover = Some(markup);
                    }
                }
                if let Some(m) = hover {
                    return Some(Hover {
                        contents: convert(m),
                        range: Some(range),
                    });
                }
            }
            None
        };

        let has_description = |d: Option<&Description>| match d {
            Some(Description::Text(s)) => !s.is_empty(),
            Some(Description::Markup(_)) => true,
            None => false,
        };

        // `getTagNameRange`.
        let tag_name_range = |token_type: TokenType, start_offset: usize| -> Option<Range> {
            let mut scanner = Scanner::at(text, start_offset);
            let mut token = scanner.scan();
            while token != TokenType::Eos
                && (scanner.token_end() < offset
                    || scanner.token_end() == offset && token != token_type)
            {
                token = scanner.scan();
            }
            (token == token_type && offset <= scanner.token_end()).then(|| {
                Range::new(
                    document.position_at(scanner.token_offset()),
                    document.position_at(scanner.token_end()),
                )
            })
        };

        if let Some(end_tag_start) = node.end_tag_start_truthy()
            && offset >= end_tag_start
        {
            return tag_hover(node_tag, tag_name_range(TokenType::EndTag, end_tag_start)?);
        }

        if let Some(range) = tag_name_range(TokenType::StartTag, node.start) {
            return tag_hover(node_tag, range);
        }

        if let Some(range) = tag_name_range(TokenType::AttributeName, node.start) {
            let attr = document.text_in(range);
            for provider in providers {
                let mut hover = None;
                for a in provider.provide_attributes(node_tag) {
                    if attr == a.name && has_description(a.description.as_ref()) {
                        hover = generate_documentation(a, options, markdown);
                    }
                }
                if let Some(m) = hover {
                    return Some(Hover {
                        contents: convert(m),
                        range: Some(range),
                    });
                }
            }
            return None;
        }

        if let Some(range) = entity_range(text, offset, position) {
            let current = filter_entity(text, offset);
            for (entity, value) in ENTITIES {
                if current.len() == entity.len() + 1 && current[1..] == **entity {
                    let code = format!("{:X}", value.encode_utf16().next().unwrap_or(0));
                    let hex = format!("U+{code:0>4}");
                    let contents = format!(
                        "Character entity representing '{value}', unicode equivalent '{hex}'"
                    );
                    return Some(Hover {
                        contents: HoverContents::Scalar(MarkedString::String(contents)),
                        range: Some(range),
                    });
                }
            }
            return None;
        }

        let range = tag_name_range(TokenType::AttributeValue, node.start)?;
        let attr_value = trim_quotes(document.text_in(range));
        let match_attr = scan_attr(text, node.start, document.offset_at(range.start))?;
        for provider in providers {
            let mut hover = None;
            for v in provider.provide_values(node_tag, match_attr) {
                if attr_value == v.name && has_description(v.description.as_ref()) {
                    hover = generate_documentation(v, options, markdown);
                }
            }
            if let Some(m) = hover {
                return Some(Hover {
                    contents: convert(m),
                    range: Some(range),
                });
            }
        }
        None
    }
}

/// `scanAttrAndAttrValue`.
fn scan_attr(text: &str, node_start: usize, attr_value_start: usize) -> Option<&str> {
    let mut scanner = Scanner::at(text, node_start);
    let mut token = scanner.scan();
    let mut prev_attr = None;
    while token != TokenType::Eos && scanner.token_end() <= attr_value_start {
        token = scanner.scan();
        if token == TokenType::AttributeName {
            prev_attr = Some(scanner.token_text());
        }
    }
    prev_attr.filter(|a| !a.is_empty())
}

/// `getEntityRange`. The range starts after the `&`.
fn entity_range(text: &str, offset: usize, position: Position) -> Option<Range> {
    let mut k = offset;
    let mut character_start = position.character;
    while k > 0 && is_letter_or_digit(text, k - 1) {
        k -= 1;
        character_start = character_start.wrapping_sub(1);
    }
    let mut n = k;
    let mut character_end = character_start;
    while is_letter_or_digit(text, n) {
        n += 1;
        character_end = character_end.wrapping_add(1);
    }
    if k > 0 && text.as_bytes()[k - 1] == b'&' {
        let end = if text.as_bytes().get(n) == Some(&b';') {
            character_end + 1
        } else {
            character_end
        };
        return Some(Range::new(
            Position::new(position.line, character_start),
            Position::new(position.line, end),
        ));
    }
    None
}

/// `filterEntity`.
fn filter_entity(text: &str, offset: usize) -> String {
    let mut k = offset;
    while k > 0 && is_letter_or_digit(text, k - 1) {
        k -= 1;
    }
    let mut n = k;
    while is_letter_or_digit(text, n) {
        n += 1;
    }
    format!("&{};", &text[k..n])
}

/// `trimQuotes`.
fn trim_quotes(s: &str) -> &str {
    if utf16_len(s) <= 1 {
        return if s == "'" || s == "\"" { "" } else { s };
    }
    let s = s.strip_prefix(['\'', '"']).unwrap_or(s);
    s.strip_suffix(['\'', '"']).unwrap_or(s)
}
