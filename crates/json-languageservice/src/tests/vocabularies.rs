use crate::SchemaDraft;
use crate::vocabularies::{Vocabularies, is_format_assertion_enabled, is_keyword_enabled};

fn vocabs(entries: &[(&str, bool)]) -> Vocabularies {
    entries.iter().map(|&(k, v)| (k.to_owned(), v)).collect()
}

#[test]
fn returns_true_for_all_keywords_when_activevocabularies_is_undefined() {
    assert!(is_keyword_enabled("type", None));
    assert!(is_keyword_enabled("properties", None));
    assert!(is_keyword_enabled("minimum", None));
    assert!(is_keyword_enabled("format", None));
    assert!(is_keyword_enabled("$ref", None));
    assert!(is_keyword_enabled("allOf", None));
    assert!(is_keyword_enabled("unknownKeyword", None));
}

#[test]
fn returns_true_for_core_keywords_regardless_of_vocabulary_settings() {
    assert!(is_keyword_enabled("$id", Some(&vocabs(&[]))));
    assert!(is_keyword_enabled("$schema", Some(&vocabs(&[]))));
    assert!(is_keyword_enabled("$ref", Some(&vocabs(&[]))));
    assert!(is_keyword_enabled("$anchor", Some(&vocabs(&[]))));
    assert!(is_keyword_enabled("$defs", Some(&vocabs(&[]))));
    assert!(is_keyword_enabled("$comment", Some(&vocabs(&[]))));
    assert!(is_keyword_enabled("$vocabulary", Some(&vocabs(&[]))));
}

#[test]
fn returns_true_for_validation_keywords_when_validation_vocabulary_is_active_2019_09() {
    assert!(is_keyword_enabled(
        "type",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "enum",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "const",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "minimum",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "maximum",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "minLength",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "maxLength",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "pattern",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "required",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "multipleOf",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
}

#[test]
fn returns_false_for_validation_keywords_when_validation_vocabulary_is_not_active() {
    assert!(!is_keyword_enabled(
        "type",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
    assert!(!is_keyword_enabled(
        "enum",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
    assert!(!is_keyword_enabled(
        "const",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
    assert!(!is_keyword_enabled(
        "minimum",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
    assert!(!is_keyword_enabled(
        "maximum",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
    assert!(!is_keyword_enabled(
        "required",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
}

#[test]
fn returns_true_for_applicator_keywords_when_applicator_vocabulary_is_active_2019_09() {
    assert!(is_keyword_enabled(
        "properties",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "patternProperties",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "additionalProperties",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "items",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "allOf",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "anyOf",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "oneOf",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "not",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "if",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "then",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "else",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/applicator",
                true
            )
        ]))
    ));
}

#[test]
fn returns_false_for_applicator_keywords_when_applicator_vocabulary_is_not_active() {
    assert!(!is_keyword_enabled(
        "properties",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(!is_keyword_enabled(
        "allOf",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(!is_keyword_enabled(
        "anyOf",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
    assert!(!is_keyword_enabled(
        "if",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
}

#[test]
fn returns_true_for_format_keyword_when_format_vocabulary_is_active_2019_09() {
    assert!(is_keyword_enabled(
        "format",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/format",
            true
        )]))
    ));
}

#[test]
fn returns_false_for_format_keyword_when_format_vocabulary_is_not_active() {
    assert!(!is_keyword_enabled(
        "format",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2019-09/vocab/core", true),
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            )
        ]))
    ));
}

#[test]
fn returns_true_for_validation_keywords_when_validation_vocabulary_is_active_2020_12() {
    assert!(is_keyword_enabled(
        "type",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "enum",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "minimum",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/validation",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "required",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/validation",
                true
            )
        ]))
    ));
}

#[test]
fn returns_true_for_unevaluated_keywords_when_unevaluated_vocabulary_is_active_2020_12() {
    assert!(is_keyword_enabled(
        "unevaluatedItems",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/unevaluated",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "unevaluatedProperties",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/unevaluated",
                true
            )
        ]))
    ));
}

#[test]
fn returns_false_for_unevaluated_keywords_when_unevaluated_vocabulary_is_not_active() {
    assert!(!is_keyword_enabled(
        "unevaluatedItems",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(!is_keyword_enabled(
        "unevaluatedProperties",
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/applicator",
                true
            )
        ]))
    ));
}

#[test]
fn returns_true_for_format_keyword_with_format_annotation_vocabulary_2020_12() {
    assert!(is_keyword_enabled(
        "format",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2020-12/vocab/format-annotation",
            true
        )]))
    ));
}

#[test]
fn returns_true_for_format_keyword_with_format_assertion_vocabulary_2020_12() {
    assert!(is_keyword_enabled(
        "format",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2020-12/vocab/format-assertion",
            true
        )]))
    ));
}

#[test]
fn returns_false_for_unknown_keywords_when_vocabularies_are_restricted() {
    assert!(!is_keyword_enabled(
        "customKeyword",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
    assert!(!is_keyword_enabled(
        "x-extension",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
}

#[test]
fn handles_mixed_2019_09_and_2020_12_vocabularies() {
    assert!(is_keyword_enabled(
        "type",
        Some(&vocabs(&[
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            ),
            (
                "https://json-schema.org/draft/2020-12/vocab/applicator",
                true
            )
        ]))
    ));
    assert!(is_keyword_enabled(
        "properties",
        Some(&vocabs(&[
            (
                "https://json-schema.org/draft/2019-09/vocab/validation",
                true
            ),
            (
                "https://json-schema.org/draft/2020-12/vocab/applicator",
                true
            )
        ]))
    ));
}

#[test]
fn meta_data_vocabulary_keywords() {
    assert!(is_keyword_enabled(
        "title",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/meta-data",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "description",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/meta-data",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "default",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/meta-data",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "deprecated",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/meta-data",
            true
        )]))
    ));
}

#[test]
fn content_vocabulary_keywords() {
    assert!(is_keyword_enabled(
        "contentEncoding",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/content",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "contentMediaType",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/content",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "contentSchema",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/content",
            true
        )]))
    ));
}

#[test]
fn recursiveref_and_recursiveanchor_are_core_keywords_in_2019_09() {
    assert!(is_keyword_enabled(
        "$recursiveRef",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "$recursiveAnchor",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/core",
            true
        )]))
    ));
}

#[test]
fn dynamicref_and_dynamicanchor_are_core_keywords_in_2020_12() {
    assert!(is_keyword_enabled(
        "$dynamicRef",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2020-12/vocab/core",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "$dynamicAnchor",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2020-12/vocab/core",
            true
        )]))
    ));
}

#[test]
fn prefixitems_is_an_applicator_keyword() {
    assert!(is_keyword_enabled(
        "prefixItems",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/applicator",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "prefixItems",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2020-12/vocab/applicator",
            true
        )]))
    ));
}

#[test]
fn dependentschemas_and_dependentrequired_are_in_correct_vocabularies() {
    assert!(is_keyword_enabled(
        "dependentSchemas",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/applicator",
            true
        )]))
    ));
    assert!(is_keyword_enabled(
        "dependentRequired",
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/validation",
            true
        )]))
    ));
}

#[test]
fn returns_true_when_activevocabularies_is_undefined() {
    assert!(is_format_assertion_enabled(None, None));
}

#[test]
fn returns_true_for_required_2019_09_format_vocabulary() {
    assert!(is_format_assertion_enabled(
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/format",
            true
        )])),
        None
    ));
}

#[test]
fn returns_false_for_optional_2019_09_format_vocabulary() {
    assert!(!is_format_assertion_enabled(
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2019-09/vocab/format",
            false
        )])),
        None
    ));
}

#[test]
fn returns_true_for_2020_12_format_assertion_vocabulary() {
    assert!(is_format_assertion_enabled(
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2020-12/vocab/format-assertion",
            true
        )])),
        None
    ));
}

#[test]
fn returns_false_for_2020_12_format_annotation_vocabulary() {
    assert!(!is_format_assertion_enabled(
        Some(&vocabs(&[(
            "https://json-schema.org/draft/2020-12/vocab/format-annotation",
            true
        )])),
        None
    ));
}

#[test]
fn returns_false_when_no_format_vocabulary_is_active() {
    assert!(!is_format_assertion_enabled(
        Some(&vocabs(&[
            ("https://json-schema.org/draft/2020-12/vocab/core", true),
            (
                "https://json-schema.org/draft/2020-12/vocab/validation",
                true
            )
        ])),
        None
    ));
}

#[test]
fn format_assertion_takes_precedence_when_both_annotation_and_assertion_are_active() {
    assert!(is_format_assertion_enabled(
        Some(&vocabs(&[
            (
                "https://json-schema.org/draft/2020-12/vocab/format-annotation",
                true
            ),
            (
                "https://json-schema.org/draft/2020-12/vocab/format-assertion",
                true
            )
        ])),
        None
    ));
}

#[test]
fn returns_false_for_2019_09_draft_without_vocabulary_info() {
    assert!(!is_format_assertion_enabled(
        None,
        Some(SchemaDraft::V2019_09)
    ));
}

#[test]
fn returns_false_for_2020_12_draft_without_vocabulary_info() {
    assert!(!is_format_assertion_enabled(
        None,
        Some(SchemaDraft::V2020_12)
    ));
}

#[test]
fn returns_true_for_draft_07_without_vocabulary_info() {
    assert!(is_format_assertion_enabled(None, Some(SchemaDraft::V7)));
}

#[test]
fn returns_true_when_no_vocabulary_and_no_explicit_draft() {
    assert!(is_format_assertion_enabled(None, None));
}
