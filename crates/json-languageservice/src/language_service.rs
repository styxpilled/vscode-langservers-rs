//! Port of `jsonLanguageService.ts`: the features that use the schema service. The other
//! features are free functions.

use std::sync::Arc;

use lsp_types::{ClientCapabilities, CompletionList, Diagnostic, Hover, Position};
use serde_json::Value;

use crate::completion::JsonCompletion;
use crate::{
    ApplicableSchema, ColorInformation, DocumentLanguageSettings, FallbackSchemaProvider,
    JsonDocument, JsonSchemaService, JsonValidation, ResolveRelativePath, ResolvedSchema,
    SchemaConfiguration, SchemaContributions, SchemaRequestService, TextDocument,
    schema_contributions,
};

#[derive(Default)]
pub struct LanguageServiceParams {
    pub schema_request_service: Option<SchemaRequestService>,
    pub workspace_context: Option<ResolveRelativePath>,
    pub fallback_schema_provider: Option<FallbackSchemaProvider>,
    pub client_capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, Default)]
pub struct LanguageSettings {
    pub validate: Option<bool>,
    pub allow_comments: Option<bool>,
    pub schemas: Vec<SchemaConfiguration>,
}

pub struct LanguageService {
    schema_service: JsonSchemaService,
    contributions: SchemaContributions,
    /// The schemas of the last `configure`.
    schemas: Vec<SchemaConfiguration>,
    validation: JsonValidation,
    completion: JsonCompletion,
}

impl LanguageService {
    pub fn new(params: LanguageServiceParams) -> Self {
        let mut schema_service = JsonSchemaService::new(
            params.schema_request_service,
            params.workspace_context,
            params.fallback_schema_provider,
        );
        let contributions = schema_contributions();
        schema_service.set_schema_contributions(&contributions);
        LanguageService {
            schema_service,
            contributions,
            schemas: Vec::new(),
            validation: JsonValidation::default(),
            completion: JsonCompletion::new(&params.client_capabilities),
        }
    }

    pub fn schema_service(&mut self) -> &mut JsonSchemaService {
        &mut self.schema_service
    }

    /// Clears all loaded schemas and registers `settings.schemas`.
    pub fn configure(&mut self, settings: &LanguageSettings) {
        self.schemas = settings.schemas.clone();
        self.schema_service.reset(&self.contributions);
        for schema in &self.schemas {
            self.schema_service.register_external_schema(schema);
        }
        self.validation.configure(
            settings.validate != Some(false),
            settings.allow_comments == Some(true),
        );
    }

    pub fn reset_schema(&mut self, uri: &str) -> bool {
        self.schema_service.on_resource_change(uri)
    }

    pub fn do_validation(
        &mut self,
        document: &TextDocument,
        json: &JsonDocument,
        settings: Option<&DocumentLanguageSettings>,
        schema: Option<&Value>,
    ) -> Vec<Diagnostic> {
        self.validation
            .do_validation(&mut self.schema_service, document, json, settings, schema)
    }

    pub fn do_syntax_validation(
        &self,
        document: &TextDocument,
        json: &JsonDocument,
        settings: Option<&DocumentLanguageSettings>,
    ) -> Vec<Diagnostic> {
        self.validation
            .do_syntax_validation(document, json, settings)
    }

    /// `getLanguageStatus`: the URIs of the schemas for the document.
    pub fn get_language_status(&self, document: &TextDocument, json: &JsonDocument) -> Vec<String> {
        self.schema_service
            .get_schema_uris_for_resource(document.uri().as_str(), Some(json))
    }

    pub fn get_matching_schemas(
        &mut self,
        document: &TextDocument,
        json: &JsonDocument,
        schema: Option<&Value>,
    ) -> Option<(Arc<ResolvedSchema>, Vec<ApplicableSchema>)> {
        self.schema_service
            .get_matching_schemas(document.uri().as_str(), json, schema)
    }

    pub fn do_complete(
        &mut self,
        document: &TextDocument,
        position: Position,
        json: &JsonDocument,
    ) -> CompletionList {
        let completion = &self.completion;
        self.schema_service
            .run(|s| completion.do_complete(s, document, position, json))
    }

    pub fn do_hover(
        &mut self,
        document: &TextDocument,
        position: Position,
        json: &JsonDocument,
    ) -> Option<Hover> {
        self.schema_service
            .run(|s| crate::hover::do_hover(s, document, position, json))
    }

    pub fn find_document_colors(
        &mut self,
        document: &TextDocument,
        json: &JsonDocument,
        limit: Option<usize>,
    ) -> Vec<ColorInformation> {
        self.schema_service
            .run(|s| crate::colors::find_document_colors(s, document, json, limit))
    }
}
