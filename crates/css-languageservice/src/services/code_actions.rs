//! Port of `cssCodeActions.ts`.

use std::cmp::Ordering;

use lsp_types::{
    CodeAction, CodeActionKind, Command, Diagnostic, DocumentChanges, NumberOrString, OneOf,
    OptionalVersionedTextDocumentIdentifier, TextDocumentEdit, TextEdit, WorkspaceEdit,
};
use text_document::TextDocument;

use super::lint_rules::UNKNOWN_PROPERTY;
use crate::language_facts::CssDataManager;
use crate::parser::Stylesheet;
use crate::parser::nodes::{Field, Kind};

/// `doCodeActions`: the code actions as `_css.applyCodeAction` commands.
pub fn do_code_actions(
    document: &TextDocument,
    diagnostics: &[Diagnostic],
    stylesheet: &Stylesheet,
    data: &CssDataManager,
) -> Vec<Command> {
    do_code_actions2(document, diagnostics, stylesheet, data)
        .into_iter()
        .map(|ca| {
            let edits = match ca.edit.and_then(|e| e.document_changes) {
                Some(DocumentChanges::Edits(mut e)) if !e.is_empty() => {
                    serde_json::to_value(e.swap_remove(0).edits).ok()
                }
                _ => None,
            };
            let mut arguments = vec![
                serde_json::to_value(document.uri()).unwrap_or_default(),
                document.version().into(),
            ];
            arguments.extend(edits);
            Command::new(ca.title, "_css.applyCodeAction".to_owned(), Some(arguments))
        })
        .collect()
}

/// `doCodeActions2`.
pub fn do_code_actions2(
    document: &TextDocument,
    diagnostics: &[Diagnostic],
    stylesheet: &Stylesheet,
    data: &CssDataManager,
) -> Vec<CodeAction> {
    let mut result = Vec::new();
    for diagnostic in diagnostics {
        append_fixes_for_marker(document, stylesheet, diagnostic, data, &mut result);
    }
    result
}

fn append_fixes_for_marker(
    document: &TextDocument,
    stylesheet: &Stylesheet,
    marker: &Diagnostic,
    data: &CssDataManager,
    result: &mut Vec<CodeAction>,
) {
    if marker.code != Some(NumberOrString::String(UNKNOWN_PROPERTY.id.to_owned())) {
        return;
    }
    let ast = &stylesheet.ast;
    let offset = document.offset_at(marker.range.start) as isize;
    let end = document.offset_at(marker.range.end) as isize;
    for node in ast.get_node_path(stylesheet.root, offset).into_iter().rev() {
        if matches!(
            ast.kind(node),
            Kind::Declaration | Kind::CustomPropertyDeclaration
        ) && let Some(property) = ast.field(node, Field::Property)
            && ast.offset(property) == offset
            && ast.end(property) == end
        {
            get_fixes_for_unknown_property(
                document,
                ast.property_name(property),
                marker,
                data,
                result,
            );
            return;
        }
    }
}

fn get_fixes_for_unknown_property(
    document: &TextDocument,
    property_name: &str,
    marker: &Diagnostic,
    data: &CssDataManager,
    result: &mut Vec<CodeAction>,
) {
    let name: Vec<char> = property_name.chars().collect();
    let mut candidates: Vec<(&str, f64)> = data
        .get_properties()
        .iter()
        .filter_map(|p| {
            let score = difference(&name, &p.name.chars().collect::<Vec<_>>(), 4);
            (score >= name.len() as f64 / 2.0).then_some((p.name.as_str(), score))
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });

    for (candidate, _) in candidates.into_iter().take(3) {
        let edit = TextEdit::new(marker.range, candidate.to_owned());
        let edit = WorkspaceEdit {
            document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: document.uri().clone(),
                    version: Some(document.version()),
                },
                edits: vec![OneOf::Left(edit)],
            }])),
            ..Default::default()
        };
        result.push(CodeAction {
            title: format!("Rename to '{candidate}'"),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![marker.clone()]),
            edit: Some(edit),
            ..Default::default()
        });
    }
}

/// `difference` of `utils/strings.ts`: the length of the longest common
/// subsequence of the characters, minus the square root of the length
/// difference. 0 if the lengths differ by more than `max_len_delta`.
///
/// Upstream shares one row array for all rows of the LCS matrix, which can
/// count a character more than once. Here each row has its own array.
fn difference(first: &[char], second: &[char], max_len_delta: usize) -> f64 {
    let length_difference = first.len().abs_diff(second.len());
    if length_difference > max_len_delta {
        return 0.0;
    }
    let mut prev = vec![0usize; second.len() + 1];
    let mut row = vec![0usize; second.len() + 1];
    for &f in first {
        for (j, &s) in second.iter().enumerate() {
            row[j + 1] = if f == s {
                prev[j] + 1
            } else {
                prev[j + 1].max(row[j])
            };
            // Upstream parity: replace the statement above with this line,
            // and the swap below with `prev.clone_from(&row);`.
            // row[j + 1] = if f == s { row[j] + 1 } else { row[j + 1].max(row[j]) };
        }
        std::mem::swap(&mut prev, &mut row);
    }
    prev[second.len()] as f64 - (length_difference as f64).sqrt()
}
