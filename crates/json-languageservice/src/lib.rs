//! Port of `microsoft/vscode-json-languageservice`.

mod colors;
mod completion;
mod configuration;
mod folding;
mod format;
mod glob;
mod hover;
mod language_service;
mod links;
mod parser;
pub mod schema;
mod schema_service;
mod selection_ranges;
mod sort;
mod strings;
mod symbols;
mod validation;
mod validator;
mod vocabularies;

#[cfg(test)]
mod tests;

pub use colors::{Color, ColorInformation, color_from_hex, get_color_presentations};
pub use configuration::schema_contributions;
pub use folding::get_folding_ranges;
pub use format::format;
pub use language_service::{LanguageService, LanguageServiceParams, LanguageSettings};
pub use links::find_links;
pub use parser::{AstKind, AstNode, JsonDocument, NodeId, parse};
pub use schema_service::{
    FallbackSchemaProvider, JsonSchemaService, ResolveRelativePath, ResolvedSchema,
    SchemaAssociation, SchemaConfiguration, SchemaContributions, SchemaDiagnostic,
    SchemaRequestError, SchemaRequestService, normalize_id,
};
pub use selection_ranges::get_selection_ranges;
pub use sort::sort;
pub use symbols::{find_document_symbols, find_document_symbols2};
pub use text_document::TextDocument;
pub use validation::{DocumentLanguageSettings, JsonValidation, SeverityLevel};
pub use validator::{ApplicableSchema, Problem, ValidationResult, get_schema_draft_from_id};
pub use vocabularies::Vocabularies;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaDraft {
    V3 = 3,
    V4 = 4,
    V6 = 6,
    V7 = 7,
    V2019_09 = 19,
    V2020_12 = 20,
}

/// JS truthiness of a JSON value.
pub(crate) fn is_truthy(value: &serde_json::Value) -> bool {
    use serde_json::Value;
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Diagnostic codes. Codes from [`ErrorCode::SCHEMA_RESOLVE_ERROR`] up are schema load errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErrorCode(pub u32);

impl ErrorCode {
    pub const UNDEFINED: Self = Self(0);
    pub const ENUM_VALUE_MISMATCH: Self = Self(1);
    pub const DEPRECATED: Self = Self(2);
    pub const UNEXPECTED_END_OF_COMMENT: Self = Self(0x101);
    pub const UNEXPECTED_END_OF_STRING: Self = Self(0x102);
    pub const UNEXPECTED_END_OF_NUMBER: Self = Self(0x103);
    pub const INVALID_UNICODE: Self = Self(0x104);
    pub const INVALID_ESCAPE_CHARACTER: Self = Self(0x105);
    pub const INVALID_CHARACTER: Self = Self(0x106);
    pub const PROPERTY_EXPECTED: Self = Self(0x201);
    pub const COMMA_EXPECTED: Self = Self(0x202);
    pub const COLON_EXPECTED: Self = Self(0x203);
    pub const VALUE_EXPECTED: Self = Self(0x204);
    /// The spelling is from upstream.
    pub const COMMA_OR_CLOSE_BACKET_EXPECTED: Self = Self(0x205);
    pub const COMMA_OR_CLOSE_BRACE_EXPECTED: Self = Self(0x206);
    pub const TRAILING_COMMA: Self = Self(0x207);
    pub const DUPLICATE_KEY: Self = Self(0x208);
    pub const COMMENT_NOT_PERMITTED: Self = Self(0x209);
    pub const PROPERTY_KEYS_MUST_BE_DOUBLEQUOTED: Self = Self(0x210);
    pub const SCHEMA_UNSUPPORTED_FEATURE: Self = Self(0x301);
    pub const SCHEMA_RESOLVE_ERROR: Self = Self(0x10000);

    pub fn is_schema_resolve_error(self) -> bool {
        self.0 >= Self::SCHEMA_RESOLVE_ERROR.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormattingOptions {
    /// 0 means 4.
    pub tab_size: u32,
    pub insert_spaces: bool,
    pub insert_final_newline: bool,
    pub keep_lines: bool,
}
