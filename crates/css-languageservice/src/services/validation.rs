//! Port of `cssValidation.ts`.

use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
use text_document::TextDocument;

use super::lint::lint_entries;
use super::lint_rules::LintConfigurationSettings;
use crate::LanguageSettings;
use crate::language_facts::CssDataManager;
use crate::parser::Stylesheet;
use crate::parser::nodes::Level;

/// `CSSValidation.doValidation`.
pub fn do_validation(
    document: &TextDocument,
    stylesheet: &Stylesheet,
    settings: Option<&LanguageSettings>,
    data: &CssDataManager,
) -> Vec<Diagnostic> {
    if settings.and_then(|s| s.validate) == Some(false) {
        return Vec::new();
    }
    let (ast, root) = (&stylesheet.ast, stylesheet.root);
    let lint = LintConfigurationSettings::new(settings.and_then(|s| s.lint.as_ref()));
    let lint_entries = lint_entries(
        ast,
        root,
        lint,
        data,
        Level::Warning as u8 | Level::Error as u8,
    );
    let position = |offset: isize| document.position_at(offset.max(0) as usize);
    ast.parse_errors(root)
        .into_iter()
        .chain(&lint_entries)
        .filter(|m| m.level != Level::Ignore)
        .map(|m| Diagnostic {
            range: lsp_types::Range::new(position(m.offset), position(m.offset + m.length)),
            severity: Some(if m.level == Level::Warning {
                DiagnosticSeverity::WARNING
            } else {
                DiagnosticSeverity::ERROR
            }),
            code: Some(NumberOrString::String(m.rule.id.to_owned())),
            source: Some(document.language_id().to_owned()),
            message: m.message.clone(),
            ..Default::default()
        })
        .collect()
}
