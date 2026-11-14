//! Port of `microsoft/vscode-html-languageservice`.

pub mod beautify;
pub mod language_facts;
pub mod parser;
pub mod services;
mod utils;

use std::sync::Arc;

use lsp_types::{ClientCapabilities, MarkupKind};
use text_document::TextDocument;

pub use language_facts::{HoverSettings, HtmlDataManager, HtmlDataProvider};
pub use parser::HtmlDocument;
pub use services::completion::CompletionConfiguration;

/// The parts of `LanguageServiceOptions` that are not data providers or a
/// file system: the services take those as arguments.
pub struct LanguageService {
    pub data_manager: HtmlDataManager,
    completion_markdown: bool,
    hover_markdown: bool,
}

impl Default for LanguageService {
    fn default() -> Self {
        Self::new(true, Vec::new(), None)
    }
}

impl LanguageService {
    /// `getLanguageService`. Without client capabilities, the services use
    /// markdown.
    pub fn new(
        use_default_data_provider: bool,
        custom_data_providers: Vec<Arc<HtmlDataProvider>>,
        client_capabilities: Option<&ClientCapabilities>,
    ) -> Self {
        let td = client_capabilities.map(|c| c.text_document.as_ref());
        let has_markdown = |formats: Option<&Vec<MarkupKind>>| {
            formats.is_some_and(|f| f.contains(&MarkupKind::Markdown))
        };
        let completion_markdown = td.is_none_or(|td| {
            has_markdown(
                td.and_then(|t| t.completion.as_ref())
                    .and_then(|c| c.completion_item.as_ref())
                    .and_then(|i| i.documentation_format.as_ref()),
            )
        });
        let hover_markdown = td.is_none_or(|td| {
            has_markdown(
                td.and_then(|t| t.hover.as_ref())
                    .and_then(|h| h.content_format.as_ref()),
            )
        });
        LanguageService {
            data_manager: HtmlDataManager::new(use_default_data_provider, custom_data_providers),
            completion_markdown,
            hover_markdown,
        }
    }

    /// `parseHTMLDocument`.
    pub fn parse_html_document(&self, document: &TextDocument) -> HtmlDocument {
        let void_elements = self.data_manager.void_elements();
        HtmlDocument::parse(
            document.text(),
            |tag| HtmlDataManager::is_void_element(tag, void_elements),
            false,
        )
    }
}
