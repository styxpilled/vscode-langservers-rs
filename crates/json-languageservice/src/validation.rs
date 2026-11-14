//! Port of `services/jsonValidation.ts`.

use std::collections::HashSet;

use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
use serde_json::Value;

use crate::schema::{SchemaId, SchemaRef, Schemas};
use crate::schema_service::{JsonSchemaService, ResolvedSchema};
use crate::{AstKind, ErrorCode, JsonDocument, SchemaDraft, TextDocument};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeverityLevel {
    Error,
    Warning,
    Ignore,
}

fn to_severity(level: SeverityLevel) -> Option<DiagnosticSeverity> {
    match level {
        SeverityLevel::Error => Some(DiagnosticSeverity::ERROR),
        SeverityLevel::Warning => Some(DiagnosticSeverity::WARNING),
        SeverityLevel::Ignore => None,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DocumentLanguageSettings {
    pub comments: Option<SeverityLevel>,
    pub trailing_commas: Option<SeverityLevel>,
    pub schema_validation: Option<SeverityLevel>,
    pub schema_request: Option<SeverityLevel>,
    pub schema_draft: Option<SchemaDraft>,
}

pub struct JsonValidation {
    validation_enabled: bool,
    comment_severity: Option<DiagnosticSeverity>,
}

impl Default for JsonValidation {
    fn default() -> Self {
        JsonValidation {
            validation_enabled: true,
            comment_severity: None,
        }
    }
}

impl JsonValidation {
    pub fn configure(&mut self, validate: bool, allow_comments: bool) {
        self.validation_enabled = validate;
        self.comment_severity = if allow_comments {
            None
        } else {
            Some(DiagnosticSeverity::ERROR)
        };
    }

    pub fn do_validation(
        &self,
        service: &mut JsonSchemaService,
        text_document: &TextDocument,
        json_document: &JsonDocument,
        settings: Option<&DocumentLanguageSettings>,
        schema: Option<&Value>,
    ) -> Vec<Diagnostic> {
        if !self.validation_enabled {
            return Vec::new();
        }
        let resolved = match schema {
            Some(schema) => {
                Some(service.resolve_inline_schema(schema, "schemaservice://untitled/"))
            }
            None => {
                service.get_schema_for_resource(text_document.uri().as_str(), Some(json_document))
            }
        };
        self.get_diagnostics(text_document, json_document, settings, resolved.as_deref())
    }

    fn get_diagnostics(
        &self,
        text_document: &TextDocument,
        json_document: &JsonDocument,
        settings: Option<&DocumentLanguageSettings>,
        schema: Option<&ResolvedSchema>,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut added = HashSet::new();
        let mut add = |problem: Diagnostic| {
            let signature = (
                problem.range.start.line,
                problem.range.start.character,
                problem.message.clone(),
            );
            if added.insert(signature) {
                diagnostics.push(problem);
            }
        };
        let settings = settings.copied().unwrap_or_default();
        let mut trailing_comma_severity = match settings.trailing_commas {
            Some(level) => to_severity(level),
            None => Some(DiagnosticSeverity::ERROR),
        };
        let mut comment_severity = match settings.comments {
            Some(level) => to_severity(level),
            None => self.comment_severity,
        };
        let schema_validation = match settings.schema_validation {
            Some(level) => to_severity(level),
            None => Some(DiagnosticSeverity::WARNING),
        };
        let schema_request = match settings.schema_request {
            Some(level) => to_severity(level),
            None => Some(DiagnosticSeverity::WARNING),
        };

        if let Some(schema) = schema {
            let mut add_schema_problem = |error: &crate::SchemaDiagnostic| {
                let (Some(root), Some(severity)) = (json_document.root(), schema_request) else {
                    return;
                };
                let first = (json_document[root].kind == AstKind::Object)
                    .then(|| json_document.children(root).first().copied())
                    .flatten();
                let range = match first {
                    Some(p) if json_document.key(p) == "$schema" => {
                        let node = &json_document[json_document.value(p).unwrap_or(p)];
                        text_document.range(node.offset(), node.end())
                    }
                    _ => {
                        let offset = json_document[root].offset();
                        text_document.range(offset, offset + 1)
                    }
                };
                add(Diagnostic {
                    range,
                    severity: Some(severity),
                    code: Some(NumberOrString::Number(error.code.0 as i32)),
                    source: Some("json".to_owned()),
                    message: error.message.clone(),
                    related_information: error.related_information.clone(),
                    ..Diagnostic::default()
                });
            };
            if let Some(error) = schema.errors.first() {
                add_schema_problem(error);
            } else if let Some(severity) = schema_validation {
                for warning in &schema.warnings {
                    add_schema_problem(warning);
                }
                if let Some(errors) = json_document.validate(
                    text_document,
                    &schema.schemas,
                    schema.root,
                    severity,
                    settings.schema_draft,
                    schema.active_vocabularies.as_ref(),
                ) {
                    errors.into_iter().for_each(&mut add);
                }
            }
            if allows(&schema.schemas, schema.root, &|s| s.allow_comments) == Some(true) {
                comment_severity = None;
            }
            if allows(&schema.schemas, schema.root, &|s| {
                s.allow_trailing_commas.or(s.allows_trailing_commas)
            }) == Some(true)
            {
                trailing_comma_severity = None;
            }
        }

        for p in &json_document.syntax_errors {
            let mut p = p.clone();
            if p.code == Some(NumberOrString::Number(ErrorCode::TRAILING_COMMA.0 as i32)) {
                let Some(severity) = trailing_comma_severity else {
                    continue;
                };
                p.severity = Some(severity);
            }
            add(p);
        }

        if let Some(severity) = comment_severity {
            for range in &json_document.comments {
                add(Diagnostic {
                    range: *range,
                    severity: Some(severity),
                    code: Some(NumberOrString::Number(
                        ErrorCode::COMMENT_NOT_PERMITTED.0 as i32,
                    )),
                    message: "Comments are not permitted in JSON.".to_owned(),
                    ..Diagnostic::default()
                });
            }
        }
        diagnostics
    }
}

/// `schemaAllowsComments` and `schemaAllowsTrailingCommas`.
fn allows(
    schemas: &Schemas,
    id: SchemaId,
    flag: &dyn Fn(&crate::schema::JsonSchema) -> Option<bool>,
) -> Option<bool> {
    let schema = &schemas[id];
    if let Some(b) = flag(schema) {
        return Some(b);
    }
    schema.all_of.as_ref()?.iter().find_map(|s| match s {
        SchemaRef::Schema(s) => allows(schemas, *s, flag),
        SchemaRef::Bool(_) => None,
    })
}
