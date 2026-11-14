//! Port of `cssErrors.ts`.

use super::nodes::Rule;

macro_rules! rules {
    ($($name:ident = $id:literal, $message:literal;)*) => {
        $(pub static $name: Rule = Rule { id: $id, message: $message };)*
    };
}

rules! {
    NUMBER_EXPECTED = "css-numberexpected", "number expected";
    CONDITION_EXPECTED = "css-conditionexpected", "condition expected";
    RULE_OR_SELECTOR_EXPECTED = "css-ruleorselectorexpected", "at-rule or selector expected";
    DOT_EXPECTED = "css-dotexpected", "dot expected";
    COLON_EXPECTED = "css-colonexpected", "colon expected";
    SEMI_COLON_EXPECTED = "css-semicolonexpected", "semi-colon expected";
    TERM_EXPECTED = "css-termexpected", "term expected";
    EXPRESSION_EXPECTED = "css-expressionexpected", "expression expected";
    OPERATOR_EXPECTED = "css-operatorexpected", "operator expected";
    IDENTIFIER_EXPECTED = "css-identifierexpected", "identifier expected";
    PERCENTAGE_EXPECTED = "css-percentageexpected", "percentage expected";
    URI_OR_STRING_EXPECTED = "css-uriorstringexpected", "uri or string expected";
    URI_EXPECTED = "css-uriexpected", "URI expected";
    VARIABLE_NAME_EXPECTED = "css-varnameexpected", "variable name expected";
    VARIABLE_VALUE_EXPECTED = "css-varvalueexpected", "variable value expected";
    PROPERTY_VALUE_EXPECTED = "css-propertyvalueexpected", "property value expected";
    LEFT_CURLY_EXPECTED = "css-lcurlyexpected", "{ expected";
    RIGHT_CURLY_EXPECTED = "css-rcurlyexpected", "} expected";
    LEFT_SQUARE_BRACKET_EXPECTED = "css-rbracketexpected", "[ expected";
    RIGHT_SQUARE_BRACKET_EXPECTED = "css-lbracketexpected", "] expected";
    LEFT_PARENTHESIS_EXPECTED = "css-lparentexpected", "( expected";
    RIGHT_PARENTHESIS_EXPECTED = "css-rparentexpected", ") expected";
    COMMA_EXPECTED = "css-commaexpected", "comma expected";
    PAGE_DIRECTIVE_OR_DECLARATION_EXPECTED = "css-pagedirordeclexpected", "page directive or declaraton expected";
    UNKNOWN_AT_RULE = "css-unknownatrule", "at-rule unknown";
    UNKNOWN_KEYWORD = "css-unknownkeyword", "unknown keyword";
    SELECTOR_EXPECTED = "css-selectorexpected", "selector expected";
    STRING_LITERAL_EXPECTED = "css-stringliteralexpected", "string literal expected";
    WHITESPACE_EXPECTED = "css-whitespaceexpected", "whitespace expected";
    MEDIA_QUERY_EXPECTED = "css-mediaqueryexpected", "media query expected";
    IDENTIFIER_OR_WILDCARD_EXPECTED = "css-idorwildcardexpected", "identifier or wildcard expected";
    WILDCARD_EXPECTED = "css-wildcardexpected", "wildcard expected";
    IDENTIFIER_OR_VARIABLE_EXPECTED = "css-idorvarexpected", "identifier or variable expected";
    IF_CONDITION_EXPECTED = "css-ifconditionexpected", "if condition expected";
}
