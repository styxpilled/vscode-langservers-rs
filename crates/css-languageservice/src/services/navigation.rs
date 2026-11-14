//! Port of the parts of `cssNavigation.ts` without a file system.

use std::collections::HashMap;

use lsp_types::{
    ColorPresentation, DocumentHighlight, DocumentHighlightKind, DocumentSymbol, Location,
    Position, Range, SymbolInformation, SymbolKind, TextEdit, WorkspaceEdit,
};
use serde::Serialize;
use text_document::TextDocument;

use crate::language_facts::colors::{
    Color, get_color_value, hsl_from_color, hwb_from_color, js_round, lab_from_color,
    lch_from_color, oklab_from_color, oklch_from_color,
};
use crate::parser::Stylesheet;
use crate::parser::nodes::{Ast, Field, Kind, NodeId, NodeType};
use crate::parser::symbol_scope::Symbols;
use crate::utils::{js_number_to_string, js_to_radix16};

/// `ColorInformation` with `f64` components.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ColorInformation {
    pub range: Range,
    pub color: Color,
}

fn get_range(ast: &Ast, node: NodeId, document: &TextDocument) -> Range {
    Range::new(
        position_at(document, ast.offset(node)),
        position_at(document, ast.end(node)),
    )
}

fn position_at(document: &TextDocument, offset: isize) -> Position {
    document.position_at(offset.max(0) as usize)
}

/// `findDefinition`.
pub fn find_definition(
    document: &TextDocument,
    position: Position,
    stylesheet: &Stylesheet,
) -> Option<Location> {
    let (ast, root) = (&stylesheet.ast, stylesheet.root);
    let symbols = Symbols::new(ast, root);
    let offset = document.offset_at(position) as isize;
    let node = ast.get_node_at_offset(root, offset)?;
    let symbol = symbols.find_symbol_from_node(ast, node)?;
    Some(Location::new(
        document.uri().clone(),
        get_range(ast, symbols.symbol(symbol).node, document),
    ))
}

/// `findReferences`.
pub fn find_references(
    document: &TextDocument,
    position: Position,
    stylesheet: &Stylesheet,
) -> Vec<Location> {
    find_document_highlights(document, position, stylesheet)
        .into_iter()
        .map(|h| Location::new(document.uri().clone(), h.range))
        .collect()
}

fn get_highlight_node(
    document: &TextDocument,
    position: Position,
    stylesheet: &Stylesheet,
) -> Option<NodeId> {
    let ast = &stylesheet.ast;
    let offset = document.offset_at(position) as isize;
    let node = ast.get_node_at_offset(stylesheet.root, offset)?;
    if matches!(
        ast.node_type(node),
        NodeType::Stylesheet | NodeType::Declarations | NodeType::ModuleConfig
    ) {
        return None;
    }
    if ast.node_type(node) == NodeType::Identifier
        && let Some(parent) = ast.parent(node)
        && ast.node_type(parent) == NodeType::ClassSelector
    {
        return Some(parent);
    }
    Some(node)
}

/// `findDocumentHighlights`.
pub fn find_document_highlights(
    document: &TextDocument,
    position: Position,
    stylesheet: &Stylesheet,
) -> Vec<DocumentHighlight> {
    let mut result = Vec::new();
    let Some(node) = get_highlight_node(document, position, stylesheet) else {
        return result;
    };
    let (ast, root) = (&stylesheet.ast, stylesheet.root);
    let symbols = Symbols::new(ast, root);
    let symbol = symbols.find_symbol_from_node(ast, node);
    let name = ast.get_text(node);
    let ty = ast.node_type(node);
    let mut push = |candidate| {
        result.push(DocumentHighlight {
            range: get_range(ast, candidate, document),
            kind: Some(get_highlight_kind(ast, candidate)),
        })
    };
    ast.accept(root, &mut |candidate| {
        if let Some(symbol) = symbol {
            if symbols.matches_symbol(ast, candidate, symbol) {
                push(candidate);
                return false;
            }
        } else if ty == ast.node_type(candidate) && ast.matches(candidate, name) {
            push(candidate);
        }
        true
    });
    result
}

fn get_highlight_kind(ast: &Ast, node: NodeId) -> DocumentHighlightKind {
    if ast.node_type(node) == NodeType::Selector {
        return DocumentHighlightKind::WRITE;
    }
    let parent = ast.parent(node);
    if ast.kind(node) == Kind::Identifier
        && parent.is_some_and(|p| ast.kind(p) == Kind::Property)
        && ast.node(node).is_custom_property
    {
        return DocumentHighlightKind::WRITE;
    }
    if let Some(parent) = parent
        && matches!(
            ast.node_type(parent),
            NodeType::FunctionDeclaration
                | NodeType::MixinDeclaration
                | NodeType::Keyframe
                | NodeType::VariableDeclaration
                | NodeType::FunctionParameter
        )
    {
        return DocumentHighlightKind::WRITE;
    }
    DocumentHighlightKind::READ
}

/// A symbol from `collectDocumentSymbols`: the name, the kind, the range, the
/// range of the name and the range of the body.
struct Collected {
    name: String,
    kind: SymbolKind,
    range: Range,
    name_range: Option<Range>,
    body_range: Option<Range>,
}

fn collect_document_symbols(document: &TextDocument, stylesheet: &Stylesheet) -> Vec<Collected> {
    let (ast, root) = (&stylesheet.ast, stylesheet.root);
    let range = |n| get_range(ast, n, document);
    let declarations = |n| ast.field(n, Field::Declarations).map(range);
    let mut result = Vec::new();
    ast.accept(root, &mut |node| {
        match ast.kind(node) {
            Kind::RuleSet => {
                for selector in ast.list_children(node, Field::Selectors) {
                    if ast.kind(selector) == Kind::Selector {
                        result.push(Collected {
                            name: ast.get_text(selector).to_owned(),
                            kind: SymbolKind::CLASS,
                            range: Range::new(
                                position_at(document, ast.offset(selector)),
                                position_at(document, ast.end(node)),
                            ),
                            name_range: Some(range(selector)),
                            body_range: declarations(node),
                        });
                    }
                }
            }
            Kind::VariableDeclaration => {
                // Not for assignments to a variable of another module (`lib.$color: red`).
                if let Some(variable) = ast.field(node, Field::Variable) {
                    result.push(Collected {
                        name: ast.get_text(variable).to_owned(),
                        kind: SymbolKind::VARIABLE,
                        range: range(node),
                        name_range: Some(range(variable)),
                        body_range: None,
                    });
                }
            }
            Kind::MixinDeclaration | Kind::FunctionDeclaration | Kind::Keyframe => {
                let name = ast.identifier_name(node);
                let (name, kind) = match ast.kind(node) {
                    Kind::MixinDeclaration => (name.to_owned(), SymbolKind::METHOD),
                    Kind::FunctionDeclaration => (name.to_owned(), SymbolKind::FUNCTION),
                    _ => (format!("@keyframes {name}"), SymbolKind::CLASS),
                };
                result.push(Collected {
                    name,
                    kind,
                    range: range(node),
                    name_range: ast.field(node, Field::Identifier).map(range),
                    body_range: declarations(node),
                });
            }
            Kind::FontFace => result.push(Collected {
                name: "@font-face".to_owned(),
                kind: SymbolKind::CLASS,
                range: range(node),
                name_range: None,
                body_range: declarations(node),
            }),
            Kind::Media => {
                if let Some(list) = ast.child(node, 0)
                    && ast.kind(list) == Kind::Medialist
                {
                    result.push(Collected {
                        name: format!("@media {}", ast.get_text(list)),
                        kind: SymbolKind::MODULE,
                        range: range(node),
                        name_range: Some(range(list)),
                        body_range: declarations(node),
                    });
                }
            }
            Kind::Scope => {
                let limits = ast.child(node, 0);
                let scope_name = match limits {
                    Some(l) if ast.kind(l) == Kind::ScopeLimits => scope_limits_name(ast, l),
                    _ => String::new(),
                };
                result.push(Collected {
                    name: if scope_name.is_empty() {
                        "@scope".to_owned()
                    } else {
                        format!("@scope {scope_name}")
                    },
                    kind: SymbolKind::MODULE,
                    range: range(node),
                    name_range: limits.map(range),
                    body_range: declarations(node),
                });
            }
            _ => {}
        }
        true
    });
    result
}

/// `ScopeLimits.getName`.
fn scope_limits_name(ast: &Ast, node: NodeId) -> String {
    let start = ast.field(node, Field::ScopeStart);
    let mut name = start.map_or(String::new(), |s| ast.get_text(s).to_owned());
    if let Some(end) = ast.field(node, Field::ScopeEnd) {
        let sep = if start.is_some() { " " } else { "" };
        name += &format!("{sep}\u{2192} {}", ast.get_text(end));
    }
    name
}

fn symbol_name(name: String) -> String {
    if name.is_empty() {
        "<undefined>".to_owned()
    } else {
        name
    }
}

/// `findSymbolInformations`.
pub fn find_symbol_informations(
    document: &TextDocument,
    stylesheet: &Stylesheet,
) -> Vec<SymbolInformation> {
    collect_document_symbols(document, stylesheet)
        .into_iter()
        .map(|s| {
            #[allow(deprecated)]
            SymbolInformation {
                name: symbol_name(s.name),
                kind: s.kind,
                tags: None,
                deprecated: None,
                location: Location::new(document.uri().clone(), s.range),
                container_name: None,
            }
        })
        .collect()
}

/// `findDocumentSymbols`.
pub fn find_document_symbols(
    document: &TextDocument,
    stylesheet: &Stylesheet,
) -> Vec<DocumentSymbol> {
    // The tree is built in an arena: each symbol has the indexes of its children.
    let mut symbols: Vec<(DocumentSymbol, Vec<usize>)> = Vec::new();
    let mut roots = Vec::new();
    let mut parents: Vec<(usize, Range)> = Vec::new();
    for s in collect_document_symbols(document, stylesheet) {
        let range = s.range;
        let selection_range = s
            .name_range
            .filter(|r| contains_range(&range, r))
            .unwrap_or(Range::new(range.start, range.start));
        #[allow(deprecated)]
        let entry = DocumentSymbol {
            name: symbol_name(s.name),
            detail: None,
            kind: s.kind,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: None,
        };
        let index = symbols.len();
        symbols.push((entry, Vec::new()));
        let mut top = parents.pop();
        while let Some(t) = top
            && !contains_range(&t.1, &range)
        {
            top = parents.pop();
        }
        match top {
            Some(t) => {
                symbols[t.0].1.push(index);
                parents.push(t);
            }
            None => roots.push(index),
        }
        if let Some(body) = s.body_range {
            parents.push((index, body));
        }
    }

    // Children have larger indexes than their parent: build from the end.
    let mut built: Vec<Option<DocumentSymbol>> = vec![None; symbols.len()];
    for (index, (mut symbol, children)) in symbols.into_iter().enumerate().rev() {
        if !children.is_empty() {
            symbol.children = Some(
                children
                    .iter()
                    .map(|&c| built[c].take().expect("child built"))
                    .collect(),
            );
        }
        built[index] = Some(symbol);
    }
    roots
        .iter()
        .map(|&r| built[r].take().expect("root built"))
        .collect()
}

/// `containsRange`: `true` if `other` is in `range` or equal to it.
fn contains_range(range: &Range, other: &Range) -> bool {
    let (other_start, other_end) = (other.start.line, other.end.line);
    let (start, end) = (range.start.line, range.end.line);
    if other_start < start || other_end < start {
        return false;
    }
    if other_start > end || other_end > end {
        return false;
    }
    if other_start == start && other.start.character < range.start.character {
        return false;
    }
    if other_end == end && other.end.character > range.end.character {
        return false;
    }
    true
}

/// `findDocumentColors`.
pub fn find_document_colors(
    document: &TextDocument,
    stylesheet: &Stylesheet,
) -> Vec<ColorInformation> {
    let ast = &stylesheet.ast;
    let mut result = Vec::new();
    ast.accept(stylesheet.root, &mut |node| {
        if let Some(color) = get_color_value(ast, node) {
            result.push(ColorInformation {
                range: get_range(ast, node, document),
                color,
            });
        }
        true
    });
    result
}

/// `getColorPresentations`.
pub fn get_color_presentations(color: Color, range: Range) -> Vec<ColorPresentation> {
    let n = js_number_to_string;
    let round = js_round;
    let pct = |x: f64| n(round(x * 100.0));
    let (red256, green256, blue256) = (
        round(color.red * 255.0),
        round(color.green * 255.0),
        round(color.blue * 255.0),
    );
    let (r, g, b) = (n(red256), n(green256), n(blue256));
    let hex = |x: f64| {
        let s = if x < 0.0 {
            format!("-{}", js_to_radix16(-x))
        } else {
            js_to_radix16(x)
        };
        if s.len() != 2 { format!("0{s}") } else { s }
    };

    let mut labels = Vec::with_capacity(8);
    labels.push(if color.alpha == 1.0 {
        format!("rgb({r}, {g}, {b})")
    } else {
        format!("rgba({r}, {g}, {b}, {})", n(color.alpha))
    });
    let rgb_hex = format!("#{}{}{}", hex(red256), hex(green256), hex(blue256));
    labels.push(if color.alpha == 1.0 {
        rgb_hex
    } else {
        format!("{rgb_hex}{}", hex(round(color.alpha * 255.0)))
    });

    let hsl = hsl_from_color(color);
    let (h, s, l) = (n(hsl.h), pct(hsl.s), pct(hsl.l));
    labels.push(if hsl.a == 1.0 {
        format!("hsl({h}, {s}%, {l}%)")
    } else {
        format!("hsla({h}, {s}%, {l}%, {})", n(hsl.a))
    });

    let hwb = hwb_from_color(color);
    let (h, w, b) = (n(hwb.h), pct(hwb.w), pct(hwb.b));
    labels.push(if hwb.a == 1.0 {
        format!("hwb({h} {w}% {b}%)")
    } else {
        format!("hwb({h} {w}% {b}% / {})", n(hwb.a))
    });

    let alpha = |a: Option<f64>| a.map_or("undefined".to_owned(), n);
    let lab = lab_from_color(color, true);
    let body = format!("{}% {} {}", n(lab.l), n(lab.a), n(lab.b));
    labels.push(if lab.alpha == Some(1.0) {
        format!("lab({body})")
    } else {
        format!("lab({body} / {})", alpha(lab.alpha))
    });
    let lch = lch_from_color(color);
    let body = format!("{}% {} {}", n(lch.l), n(lch.c), n(lch.h));
    labels.push(if lch.alpha == Some(1.0) {
        format!("lch({body})")
    } else {
        format!("lch({body} / {})", alpha(lch.alpha))
    });
    let oklab = oklab_from_color(color, true);
    let body = format!("{}% {} {}", n(oklab.l), n(oklab.a), n(oklab.b));
    labels.push(if oklab.alpha == Some(1.0) {
        format!("oklab({body})")
    } else {
        format!("oklab({body} / {})", alpha(oklab.alpha))
    });
    let oklch = oklch_from_color(color);
    let body = format!("{}% {} {}", n(oklch.l), n(oklch.c), n(oklch.h));
    labels.push(if oklch.alpha == Some(1.0) {
        format!("oklch({body})")
    } else {
        format!("oklch({body} / {})", alpha(oklch.alpha))
    });

    labels
        .into_iter()
        .map(|label| ColorPresentation {
            text_edit: Some(TextEdit::new(range, label.clone())),
            label,
            additional_text_edits: None,
        })
        .collect()
}

/// `prepareRename`.
pub fn prepare_rename(
    document: &TextDocument,
    position: Position,
    stylesheet: &Stylesheet,
) -> Option<Range> {
    let node = get_highlight_node(document, position, stylesheet)?;
    Some(get_range(&stylesheet.ast, node, document))
}

/// `doRename`.
pub fn do_rename(
    document: &TextDocument,
    position: Position,
    new_name: &str,
    stylesheet: &Stylesheet,
) -> WorkspaceEdit {
    let edits = find_document_highlights(document, position, stylesheet)
        .into_iter()
        .map(|h| TextEdit::new(h.range, new_name.to_owned()))
        .collect();
    WorkspaceEdit {
        changes: Some(HashMap::from([(document.uri().clone(), edits)])),
        ..Default::default()
    }
}
