//! Port of `microsoft/vscode-css-languageservice`.

pub mod beautify;
pub mod data;
pub mod language_facts;
pub mod parser;
pub mod services;
pub mod utils;

use serde::Deserialize;

pub use language_facts::entry::HoverSettings;
pub use services::completion::CompletionSettings;
pub use services::lint_rules::LintSettings;

/// `LanguageSettings`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LanguageSettings {
    pub validate: Option<bool>,
    pub lint: Option<LintSettings>,
    pub hover: Option<HoverSettings>,
    pub completion: Option<CompletionSettings>,
    #[serde(rename = "importAliases")]
    pub import_aliases: Option<services::links::AliasSettings>,
}
