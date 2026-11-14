//! Port of `cssHover.ts`.

use lsp_types::{
    ClientCapabilities, Hover, HoverContents, MarkedString, MarkupContent, MarkupKind, Position,
    Range,
};
use text_document::TextDocument;

use super::selector_printing::SelectorPrinting;
use crate::language_facts::CssDataManager;
use crate::language_facts::entry::{Entry, HoverSettings, get_entry_description};
use crate::parser::Stylesheet;
use crate::parser::nodes::{Field, Kind, NodeType};

/// `doesSupportMarkdown`: `true` without client capabilities.
pub fn supports_markdown(capabilities: Option<&ClientCapabilities>) -> bool {
    let Some(capabilities) = capabilities else {
        return true;
    };
    capabilities
        .text_document
        .as_ref()
        .and_then(|t| t.hover.as_ref())
        .and_then(|h| h.content_format.as_ref())
        .is_some_and(|f| f.contains(&MarkupKind::Markdown))
}

/// `CSSHover.doHover`.
pub fn do_hover(
    document: &TextDocument,
    position: Position,
    stylesheet: &Stylesheet,
    data: &CssDataManager,
    markdown: bool,
    settings: Option<&HoverSettings>,
) -> Option<Hover> {
    let ast = &stylesheet.ast;
    let range = |node| {
        let n = ast.node(node);
        Range::new(
            document.position_at(n.offset().max(0) as usize),
            document.position_at(n.end().max(0) as usize),
        )
    };
    let entry_hover = |entry: Option<&dyn Entry>, node| -> Option<Option<Hover>> {
        let entry = entry?;
        Some(
            get_entry_description(entry, markdown, settings).map(|contents| Hover {
                contents: HoverContents::Markup(contents),
                range: Some(range(node)),
            }),
        )
    };
    let printing = SelectorPrinting::new(data);
    let offset = document.offset_at(position) as isize;

    let mut hover: Option<Hover> = None;
    let mut selector_contexts: Vec<String> = Vec::new();
    for node in ast.get_node_path(stylesheet.root, offset) {
        let kind = ast.kind(node);
        if kind == Kind::Scope
            && let Some(limits) = ast.child(node, 0)
            && ast.kind(limits) == Kind::ScopeLimits
        {
            let start = ast.field(limits, Field::ScopeStart);
            let mut name = start.map_or(String::new(), |s| ast.get_text(s).to_owned());
            if let Some(end) = ast.field(limits, Field::ScopeEnd) {
                let sep = if start.is_some() { " " } else { "" };
                name += &format!("{sep}\u{2192} {}", ast.get_text(end));
            }
            selector_contexts.push(if name.is_empty() {
                "@scope".to_owned()
            } else {
                format!("@scope {name}")
            });
        }
        if kind == Kind::Media
            && let Some(list) = ast.child(node, 0)
            && ast.kind(list) == Kind::Medialist
        {
            selector_contexts.push(format!("@media {}", ast.get_text(list)));
        }
        if kind == Kind::Selector {
            hover = Some(Hover {
                contents: HoverContents::Array(printing.selector_to_marked_string(
                    ast,
                    node,
                    &selector_contexts,
                )),
                range: Some(range(node)),
            });
            break;
        }
        if kind == Kind::SimpleSelector {
            // Some Sass at-rules (`@at-root`) are simple selectors.
            if !ast.get_text(node).starts_with('@') {
                hover = Some(Hover {
                    contents: HoverContents::Array(
                        printing.simple_selector_to_marked_string(ast, node),
                    ),
                    range: Some(range(node)),
                });
            }
            break;
        }
        let entry_result = if kind.is_declaration() {
            let name = ast.full_property_name(node);
            entry_hover(data.get_property(&name).map(|e| e as &dyn Entry), node)
        } else if kind == Kind::UnknownAtRule
            && let Some(name) = ast.child(node, 0)
            && offset <= ast.end(name)
        {
            let entry = data.get_at_directive(ast.get_text(name));
            entry_hover(entry.map(|e| e as &dyn Entry), name)
        } else if ast.node_type(node) == NodeType::PseudoSelector {
            let name = ast.get_text(node);
            let entry = if name.starts_with("::") {
                data.get_pseudo_element(name)
            } else {
                data.get_pseudo_class(name)
            };
            entry_hover(entry.map(|e| e as &dyn Entry), node)
        } else {
            None
        };
        if let Some(h) = entry_result {
            hover = h;
        }
    }

    if !markdown && let Some(h) = hover.as_mut() {
        h.contents = convert_contents(std::mem::replace(
            &mut h.contents,
            HoverContents::Array(Vec::new()),
        ));
    }
    hover
}

/// `convertContents` for clients without markdown.
fn convert_contents(contents: HoverContents) -> HoverContents {
    let plain = |m: MarkedString| match m {
        MarkedString::String(s) => s,
        MarkedString::LanguageString(l) => l.value,
    };
    match contents {
        HoverContents::Markup(m) => HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: m.value,
        }),
        HoverContents::Array(a) => HoverContents::Array(
            a.into_iter()
                .map(|m| MarkedString::String(plain(m)))
                .collect(),
        ),
        HoverContents::Scalar(m) => HoverContents::Scalar(MarkedString::String(plain(m))),
    }
}
