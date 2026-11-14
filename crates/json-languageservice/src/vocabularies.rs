//! Port of `services/vocabularies.ts`.

use indexmap::IndexMap;

use crate::SchemaDraft;

/// Vocabulary URI to the `required` flag.
pub type Vocabularies = IndexMap<String, bool>;

const CORE_201909: &str = "https://json-schema.org/draft/2019-09/vocab/core";
const CORE_202012: &str = "https://json-schema.org/draft/2020-12/vocab/core";

const VALIDATION: &[&str] = &[
    "type",
    "enum",
    "const",
    "multipleOf",
    "maximum",
    "exclusiveMaximum",
    "minimum",
    "exclusiveMinimum",
    "maxLength",
    "minLength",
    "pattern",
    "maxItems",
    "minItems",
    "uniqueItems",
    "maxContains",
    "minContains",
    "maxProperties",
    "minProperties",
    "required",
    "dependentRequired",
];
const APPLICATOR: &[&str] = &[
    "prefixItems",
    "items",
    "contains",
    "additionalProperties",
    "properties",
    "patternProperties",
    "dependentSchemas",
    "propertyNames",
    "if",
    "then",
    "else",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
];
const META_DATA: &[&str] = &[
    "title",
    "description",
    "default",
    "deprecated",
    "readOnly",
    "writeOnly",
    "examples",
];
const CONTENT: &[&str] = &["contentEncoding", "contentMediaType", "contentSchema"];
const CORE_KEYWORDS_201909: &[&str] = &[
    "$id",
    "$schema",
    "$ref",
    "$anchor",
    "$recursiveRef",
    "$recursiveAnchor",
    "$defs",
    "$comment",
    "$vocabulary",
];
const CORE_KEYWORDS_202012: &[&str] = &[
    "$id",
    "$schema",
    "$ref",
    "$anchor",
    "$dynamicRef",
    "$dynamicAnchor",
    "$defs",
    "$comment",
    "$vocabulary",
];

const VOCABULARY_KEYWORDS: &[(&str, &[&str])] = &[
    (CORE_201909, CORE_KEYWORDS_201909),
    (
        "https://json-schema.org/draft/2019-09/vocab/applicator",
        APPLICATOR,
    ),
    (
        "https://json-schema.org/draft/2019-09/vocab/validation",
        VALIDATION,
    ),
    (
        "https://json-schema.org/draft/2019-09/vocab/meta-data",
        META_DATA,
    ),
    (
        "https://json-schema.org/draft/2019-09/vocab/format",
        &["format"],
    ),
    (
        "https://json-schema.org/draft/2019-09/vocab/content",
        CONTENT,
    ),
    (CORE_202012, CORE_KEYWORDS_202012),
    (
        "https://json-schema.org/draft/2020-12/vocab/applicator",
        APPLICATOR,
    ),
    (
        "https://json-schema.org/draft/2020-12/vocab/unevaluated",
        &["unevaluatedItems", "unevaluatedProperties"],
    ),
    (
        "https://json-schema.org/draft/2020-12/vocab/validation",
        VALIDATION,
    ),
    (
        "https://json-schema.org/draft/2020-12/vocab/meta-data",
        META_DATA,
    ),
    (
        "https://json-schema.org/draft/2020-12/vocab/format-annotation",
        &["format"],
    ),
    (
        "https://json-schema.org/draft/2020-12/vocab/format-assertion",
        &["format"],
    ),
    (
        "https://json-schema.org/draft/2020-12/vocab/content",
        CONTENT,
    ),
];

pub fn is_keyword_enabled(keyword: &str, active: Option<&Vocabularies>) -> bool {
    let Some(active) = active else {
        return true;
    };
    VOCABULARY_KEYWORDS
        .iter()
        .any(|(uri, keywords)| keywords.contains(&keyword) && active.contains_key(*uri))
        || CORE_KEYWORDS_201909.contains(&keyword)
        || CORE_KEYWORDS_202012.contains(&keyword)
}

pub fn is_format_assertion_enabled(
    active: Option<&Vocabularies>,
    schema_draft: Option<SchemaDraft>,
) -> bool {
    if let Some(active) = active.filter(|a| !a.is_empty()) {
        if active.contains_key("https://json-schema.org/draft/2020-12/vocab/format-assertion") {
            return true;
        }
        if let Some(&required) = active.get("https://json-schema.org/draft/2019-09/vocab/format") {
            return required;
        }
        return false;
    }
    !schema_draft.is_some_and(|d| d >= SchemaDraft::V2019_09)
}
