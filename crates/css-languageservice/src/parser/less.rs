//! The Less grammar of `lessParser.ts`.

use super::Parser;
use super::errors::*;
use super::nodes::{Field, Kind, NodeId, NodeType, ReferenceType};
use super::scanner::TokenType as T;

/// `/^[\w-]/` on a token text.
fn is_word_start(text: &str) -> bool {
    text.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl<'a> Parser<'a> {
    pub(crate) fn less_parse_stylesheet_statement(&mut self, is_nested: bool) -> Option<NodeId> {
        if self.peek(T::AtKeyword) {
            return self
                .parse_variable_declaration(&[])
                .or_else(|| self.parse_plugin())
                .or_else(|| self.parse_stylesheet_at_statement(is_nested));
        }
        self.try_parse_mixin_declaration()
            .or_else(|| self.try_parse_mixin_reference(true))
            .or_else(|| self.parse_function())
            .or_else(|| self.parse_ruleset(true))
    }

    pub(crate) fn less_parse_import(&mut self) -> Option<NodeId> {
        // `@import-once` is deprecated since Less 1.4.1.
        if !self.peek_keyword("@import") && !self.peek_keyword("@import-once") {
            return None;
        }
        let node = self.create(Kind::Import);
        self.consume_token();

        // Less 1.4.1: `@import (css) "lib"`.
        if self.accept(T::ParenthesisL) {
            if !self.accept(T::Ident) {
                return Some(self.finish_resync(node, &IDENTIFIER_EXPECTED, &[T::SemiColon], &[]));
            }
            while self.accept(T::Comma) && self.accept(T::Ident) {}
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(
                    node,
                    &RIGHT_PARENTHESIS_EXPECTED,
                    &[T::SemiColon],
                    &[],
                ));
            }
        }

        let uri = self.parse_uri_literal();
        if !self.ast.add_child(node, uri) {
            let string = self.parse_string_literal();
            if !self.ast.add_child(node, string) {
                return Some(self.finish_resync(
                    node,
                    &URI_OR_STRING_EXPECTED,
                    &[T::SemiColon],
                    &[],
                ));
            }
        }

        if !self.peek(T::SemiColon) && !self.peek(T::Eof) {
            let list = self.parse_media_query_list();
            self.ast.adopt_child(node, list, None);
        }

        self.complete_parse_import(node)
    }

    fn parse_plugin(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@plugin") {
            return None;
        }
        let node = self.create_node(NodeType::Plugin);
        self.consume_token();

        let string = self.parse_string_literal();
        if !self.ast.add_child(node, string) {
            return Some(self.finish_error(node, &STRING_LITERAL_EXPECTED));
        }
        if !self.accept(T::SemiColon) {
            return Some(self.finish_error(node, &SEMI_COLON_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn less_parse_media_query(&mut self) -> Option<NodeId> {
        if let Some(node) = self.css_parse_media_query() {
            return Some(node);
        }
        let node = self.create(Kind::MediaQuery);
        let variable = self.parse_variable();
        if self.ast.add_child(node, variable) {
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn less_parse_media_declaration(&mut self, is_nested: bool) -> Option<NodeId> {
        self.try_parse_ruleset(is_nested)
            .or_else(|| self.try_to_parse_declaration(None))
            .or_else(|| self.try_parse_mixin_declaration())
            .or_else(|| self.try_parse_mixin_reference(true))
            .or_else(|| self.parse_detached_rule_set_mixin())
            .or_else(|| self.parse_stylesheet_statement(is_nested))
    }

    pub(crate) fn less_parse_media_feature_name(&mut self) -> Option<NodeId> {
        self.parse_ident(None).or_else(|| self.parse_variable())
    }

    pub(crate) fn less_parse_variable_declaration(&mut self, panic: &[T]) -> Option<NodeId> {
        let node = self.create(Kind::VariableDeclaration);
        let mark = self.mark();

        // `setVariable` attaches without `addChild`.
        let variable = self.less_parse_variable(true, false);
        if !self.ast.set_node(node, Field::Variable, variable, None) {
            return None;
        }

        if self.accept(T::Colon) {
            if let Some(prev) = &self.prev_token {
                let offset = prev.offset as u32;
                self.ast.extra_mut(node).colon_position = Some(offset);
            }
            let detached = self.parse_detached_rule_set();
            if self.ast.set_node(node, Field::Value, detached, None) {
                self.ast.extra_mut(node).needs_semicolon = false;
            } else {
                let value = self.parse_expr(false);
                if !self.ast.set_node(node, Field::Value, value, None) {
                    return Some(self.finish_resync(node, &VARIABLE_VALUE_EXPECTED, &[], panic));
                }
            }
            let prio = self.parse_prio();
            self.ast.add_child(node, prio);
        } else {
            // An at-keyword without `:` is not a variable declaration.
            self.restore_at_mark(mark);
            return None;
        }

        if self.peek(T::SemiColon) {
            let offset = self.token.offset as u32;
            self.ast.extra_mut(node).semicolon_position = Some(offset);
        }
        Some(self.finish(node))
    }

    fn parse_detached_rule_set(&mut self) -> Option<NodeId> {
        let mark = self.mark();

        // An anonymous mixin, used in `each()`.
        if self.peek_delim("#") || self.peek_delim(".") {
            self.consume_token();
            if !self.has_whitespace() && self.accept(T::ParenthesisL) {
                let node = self.create(Kind::MixinDeclaration);
                self.parse_mixin_parameter_list(node);
                if !self.accept(T::ParenthesisR) {
                    self.restore_at_mark(mark);
                    return None;
                }
            } else {
                self.restore_at_mark(mark);
                return None;
            }
        }

        if !self.peek(T::CurlyL) {
            return None;
        }
        let content = self.create(Kind::BodyDeclaration);
        self.parse_body(content, |p| p.parse_rule_set_declaration());
        Some(self.finish(content))
    }

    /// The parameters of a mixin declaration, separated by `,` or `;`.
    fn parse_mixin_parameter_list(&mut self, node: NodeId) {
        let parameter = self.parse_mixin_parameter();
        let parameters = self.ast.list(node, Field::Parameters);
        if self.ast.add_child(parameters, parameter) {
            while self.accept(T::Comma) || self.accept(T::SemiColon) {
                if self.peek(T::ParenthesisR) {
                    break;
                }
                let parameter = self.parse_mixin_parameter();
                if !self.ast.add_child(parameters, parameter) {
                    self.mark_error_with(
                        node,
                        &IDENTIFIER_EXPECTED,
                        Some((&[], &[T::ParenthesisR])),
                    );
                }
            }
        }
    }

    fn add_lookup_children(&mut self, node: NodeId) -> bool {
        let value = self.parse_lookup_value();
        if !self.ast.add_child(node, value) {
            return false;
        }
        let mut expects_value = false;
        loop {
            if self.peek(T::BracketL) {
                expects_value = true;
            }
            let value = self.parse_lookup_value();
            if !self.ast.add_child(node, value) {
                break;
            }
            expects_value = false;
        }
        !expects_value
    }

    fn parse_lookup_value(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Node);
        let mark = self.mark();

        if !self.accept(T::BracketL) {
            self.restore_at_mark(mark);
            return None;
        }

        let inner = {
            let variable = self.less_parse_variable(false, true);
            self.ast.add_child(node, variable) || {
                let property = self.parse_property_identifier();
                self.ast.add_child(node, property)
            }
        };
        if (inner && self.accept(T::BracketR)) || self.accept(T::BracketR) {
            return Some(node);
        }

        self.restore_at_mark(mark);
        None
    }

    pub(crate) fn less_parse_variable(
        &mut self,
        declaration: bool,
        inside_lookup: bool,
    ) -> Option<NodeId> {
        let is_property_reference = !declaration && self.peek_delim("$");
        if !self.peek_delim("@") && !is_property_reference && !self.peek(T::AtKeyword) {
            return None;
        }

        let node = self.create(Kind::Variable);
        let mark = self.mark();

        while self.accept_delim("@") || (!declaration && self.accept_delim("$")) {
            if self.has_whitespace() {
                self.restore_at_mark(mark);
                return None;
            }
        }
        if !self.accept(T::AtKeyword) && !self.accept(T::Ident) {
            self.restore_at_mark(mark);
            return None;
        }
        if !inside_lookup && self.peek(T::BracketL) && !self.add_lookup_children(node) {
            self.restore_at_mark(mark);
            return None;
        }
        Some(node)
    }

    pub(crate) fn less_parse_term_expression(&mut self) -> Option<NodeId> {
        self.parse_variable()
            .or_else(|| self.parse_escaped())
            // Colors before mixin references.
            .or_else(|| self.css_parse_term_expression())
            .or_else(|| self.try_parse_mixin_reference(false))
    }

    fn parse_escaped(&mut self) -> Option<NodeId> {
        if self.peek(T::EscapedJavaScript) || self.peek(T::BadEscapedJavaScript) {
            let node = self.create_node(NodeType::EscapedValue);
            self.consume_token();
            return Some(self.finish(node));
        }
        if self.peek_delim("~") {
            let node = self.create_node(NodeType::EscapedValue);
            self.consume_token();
            if self.accept(T::String) || self.accept(T::EscapedJavaScript) {
                return Some(self.finish(node));
            }
            return Some(self.finish_error(node, &TERM_EXPECTED));
        }
        None
    }

    pub(crate) fn less_parse_operator(&mut self) -> Option<NodeId> {
        self.parse_guard_operator()
            .or_else(|| self.css_parse_operator())
    }

    fn parse_guard_operator(&mut self) -> Option<NodeId> {
        let follow = if self.peek_delim(">") || self.peek_delim("<") {
            "="
        } else if self.peek_delim("=") {
            "<"
        } else {
            return None;
        };
        let node = self.create_node(NodeType::Operator);
        self.consume_token();
        self.accept_delim(follow);
        Some(node)
    }

    pub(crate) fn less_parse_rule_set_declaration(&mut self) -> Option<NodeId> {
        if self.peek(T::AtKeyword) {
            return self
                .parse_keyframe()
                .or_else(|| self.parse_media(true))
                .or_else(|| self.parse_import())
                .or_else(|| self.parse_supports(true))
                .or_else(|| self.parse_layer(false))
                .or_else(|| self.parse_property_at_rule())
                .or_else(|| self.parse_container(true))
                .or_else(|| self.parse_detached_rule_set_mixin())
                .or_else(|| self.parse_variable_declaration(&[]))
                .or_else(|| self.parse_rule_set_declaration_at_statement());
        }
        self.try_parse_mixin_declaration()
            .or_else(|| self.try_parse_ruleset(true))
            .or_else(|| self.try_parse_mixin_reference(true))
            .or_else(|| self.parse_function())
            .or_else(|| self.parse_extend())
            // A keyframe selector, such as `0%` in content included into `@keyframes`.
            .or_else(|| self.try_parse_keyframe_selector())
            .or_else(|| self.parse_declaration(None, false))
    }

    pub(crate) fn less_parse_keyframe_ident(&mut self) -> Option<NodeId> {
        self.parse_ident(Some(&[ReferenceType::Keyframe]))
            .or_else(|| self.parse_variable())
    }

    pub(crate) fn less_parse_keyframe_selector(&mut self) -> Option<NodeId> {
        self.parse_detached_rule_set_mixin()
            .or_else(|| self.css_parse_keyframe_selector())
    }

    /// With CSS guards.
    pub(crate) fn less_parse_selector(&mut self, is_nested: bool) -> Option<NodeId> {
        let node = self.create(Kind::Selector);

        let mut has_content = false;
        if is_nested {
            let combinator = self.parse_combinator();
            has_content = self.ast.add_child(node, combinator);
        }
        loop {
            let simple = self.parse_simple_selector();
            if !self.ast.add_child(node, simple) {
                break;
            }
            has_content = true;
            let mark = self.mark();
            let guard = self.parse_guard();
            if self.ast.add_child(node, guard) && self.peek(T::CurlyL) {
                break;
            }
            self.restore_at_mark(mark);
            let combinator = self.parse_combinator();
            self.ast.add_child(node, combinator);
        }
        has_content.then(|| self.finish(node))
    }

    pub(crate) fn less_parse_nesting_selector(&mut self) -> Option<NodeId> {
        if !self.peek_delim("&") {
            return None;
        }
        let node = self.create_node(NodeType::SelectorCombinator);
        self.consume_token();
        // `&-foo`
        while !self.has_whitespace()
            && (self.accept_delim("-")
                || self.accept(T::Num)
                || self.accept(T::Dimension)
                || {
                    let ident = self.parse_ident(None);
                    self.ast.add_child(node, ident)
                }
                || self.accept_delim("&"))
        {}
        Some(self.finish(node))
    }

    pub(crate) fn less_parse_selector_ident(&mut self) -> Option<NodeId> {
        if !self.peek_interpolated_ident() {
            return None;
        }
        let node = self.create_node(NodeType::SelectorInterpolation);
        let has_content = self.accept_interpolated_ident(node, false);
        has_content.then(|| self.finish(node))
    }

    /// `_parsePropertyIdentifier()`. Upstream never passes `inLookup`, so only
    /// the `inLookup = false` path is here.
    pub(crate) fn less_parse_property_identifier(&mut self) -> Option<NodeId> {
        if !self.peek_interpolated_ident() && !is_word_start(&self.text()) {
            return None;
        }
        let mark = self.mark();
        let node = self.create(Kind::Identifier);

        let is_custom_property = self.accept_delim("-") && self.accept_delim("-");
        self.ast.node_mut(node).is_custom_property = is_custom_property;

        let child_added = self.accept_interpolated_ident(node, !is_custom_property);
        if !child_added {
            self.restore_at_mark(mark);
            return None;
        }

        if !self.has_whitespace() {
            self.accept_delim("+");
            if !self.has_whitespace() {
                self.accept_ident("_");
            }
        }
        Some(self.finish(node))
    }

    fn peek_interpolated_ident(&self) -> bool {
        self.peek(T::Ident) || self.peek_delim("@") || self.peek_delim("$") || self.peek_delim("-")
    }

    /// `_acceptInterpolatedIdent`. With `word_regex`, a token is accepted
    /// when its text matches `/^[\w-]+/`, otherwise when it is an identifier.
    fn accept_interpolated_ident(&mut self, node: NodeId, word_regex: bool) -> bool {
        let mut has_content = false;
        loop {
            let accepted = if word_regex {
                if is_word_start(&self.text()) {
                    self.consume_token();
                    true
                } else {
                    false
                }
            } else {
                self.accept(T::Ident)
            };
            let matched = accepted || {
                let interpolation = self
                    .parse_interpolation()
                    .or_else(|| self.try_parse(|p| p.less_ident_interpolation()));
                self.ast.add_child(node, interpolation)
            };
            if !matched {
                break;
            }
            has_content = true;
            if self.has_whitespace() {
                break;
            }
        }
        has_content
    }

    fn less_ident_interpolation(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        if self.accept_delim("-") {
            if !self.has_whitespace() {
                self.accept_delim("-");
            }
            if self.has_whitespace() {
                self.restore_at_mark(pos);
                return None;
            }
        }
        self.parse_interpolation()
    }

    /// `@{name}` (a variable) or `${name}` (a property).
    pub(crate) fn less_parse_interpolation(&mut self) -> Option<NodeId> {
        let mark = self.mark();
        if !self.peek_delim("@") && !self.peek_delim("$") {
            return None;
        }
        let node = self.create_node(NodeType::Interpolation);
        self.consume_token();
        if self.has_whitespace() || !self.accept(T::CurlyL) {
            self.restore_at_mark(mark);
            return None;
        }
        let ident = self.parse_ident(None);
        if !self.ast.add_child(node, ident) {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        if !self.accept(T::CurlyR) {
            return Some(self.finish_error(node, &RIGHT_CURLY_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn try_parse_mixin_declaration(&mut self) -> Option<NodeId> {
        let mark = self.mark();
        let node = self.create(Kind::MixinDeclaration);

        let identifier = self.parse_mixin_declaration_identifier();
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
            || !self.accept(T::ParenthesisL)
        {
            self.restore_at_mark(mark);
            return None;
        }

        self.parse_mixin_parameter_list(node);

        if !self.accept(T::ParenthesisR) {
            self.restore_at_mark(mark);
            return None;
        }
        // `setGuard` attaches without `addChild`.
        let guard = self.parse_guard();
        self.ast.set_node(node, Field::Guard, guard, None);

        if !self.peek(T::CurlyL) {
            self.restore_at_mark(mark);
            return None;
        }

        Some(self.parse_body(node, |p| {
            p.parse_font_face()
                .or_else(|| p.parse_rule_set_declaration())
        }))
    }

    fn parse_mixin_declaration_identifier(&mut self) -> Option<NodeId> {
        let identifier;
        if self.peek_delim("#") || self.peek_delim(".") {
            identifier = self.create(Kind::Identifier);
            self.consume_token();
            if self.has_whitespace() {
                return None;
            }
            let ident = self.parse_ident(None);
            if !self.ast.add_child(identifier, ident) {
                return None;
            }
        } else if self.peek(T::Hash) {
            identifier = self.create(Kind::Identifier);
            self.consume_token();
        } else {
            return None;
        }
        self.ast.extra_mut(identifier).reference_types = Some(&[ReferenceType::Mixin]);
        Some(self.finish(identifier))
    }

    pub(crate) fn less_parse_pseudo(&mut self) -> Option<NodeId> {
        if !self.peek(T::Colon) {
            return None;
        }
        let mark = self.mark();
        let node = self.create(Kind::ExtendsReference);
        self.consume_token();
        if self.accept_ident("extend") {
            return Some(self.complete_extends(node));
        }
        self.restore_at_mark(mark);
        self.css_parse_pseudo()
    }

    fn parse_extend(&mut self) -> Option<NodeId> {
        if !self.peek_delim("&") {
            return None;
        }
        let mark = self.mark();
        let node = self.create(Kind::ExtendsReference);
        self.consume_token();

        if self.has_whitespace() || !self.accept(T::Colon) || !self.accept_ident("extend") {
            self.restore_at_mark(mark);
            return None;
        }
        Some(self.complete_extends(node))
    }

    fn complete_extends(&mut self, node: NodeId) -> NodeId {
        if !self.accept(T::ParenthesisL) {
            return self.finish_error(node, &LEFT_PARENTHESIS_EXPECTED);
        }
        let selectors = self.ast.list(node, Field::Selectors);
        let selector = self.parse_selector(true);
        if !self.ast.add_child(selectors, selector) {
            return self.finish_error(node, &SELECTOR_EXPECTED);
        }
        while self.accept(T::Comma) {
            let selector = self.parse_selector(true);
            if !self.ast.add_child(selectors, selector) {
                return self.finish_error(node, &SELECTOR_EXPECTED);
            }
        }
        if !self.accept(T::ParenthesisR) {
            return self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED);
        }
        self.finish(node)
    }

    fn parse_detached_rule_set_mixin(&mut self) -> Option<NodeId> {
        if !self.peek(T::AtKeyword) {
            return None;
        }
        let mark = self.mark();
        let node = self.create(Kind::MixinReference);
        let variable = self.less_parse_variable(true, false);
        if self.ast.add_child(node, variable)
            && (self.has_whitespace() || !self.accept(T::ParenthesisL))
        {
            self.restore_at_mark(mark);
            return None;
        }
        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn try_parse_mixin_reference(&mut self, at_root: bool) -> Option<NodeId> {
        let mark = self.mark();
        let node = self.create(Kind::MixinReference);

        let mut identifier = self.parse_mixin_declaration_identifier();
        while let Some(id) = identifier {
            self.accept_delim(">");
            let Some(next) = self.parse_mixin_declaration_identifier() else {
                break;
            };
            let namespaces = self.ast.list(node, Field::Namespaces);
            self.ast.add_child(namespaces, Some(id));
            identifier = Some(next);
        }
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
        {
            self.restore_at_mark(mark);
            return None;
        }
        let identifier = identifier.unwrap();
        let mut has_arguments = false;

        if self.accept(T::ParenthesisL) {
            has_arguments = true;
            let argument = self.parse_mixin_argument();
            let arguments = self.ast.list(node, Field::Arguments);
            if self.ast.add_child(arguments, argument) {
                while self.accept(T::Comma) || self.accept(T::SemiColon) {
                    if self.peek(T::ParenthesisR) {
                        break;
                    }
                    let argument = self.parse_mixin_argument();
                    if !self.ast.add_child(arguments, argument) {
                        return Some(self.finish_error(node, &EXPRESSION_EXPECTED));
                    }
                }
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
            }
            self.ast.extra_mut(identifier).reference_types = Some(&[ReferenceType::Mixin]);
        } else {
            self.ast.extra_mut(identifier).reference_types =
                Some(&[ReferenceType::Mixin, ReferenceType::Rule]);
        }

        if self.peek(T::BracketL) {
            if !at_root {
                self.add_lookup_children(node);
            }
        } else {
            let prio = self.parse_prio();
            self.ast.add_child(node, prio);
        }

        if !has_arguments && !self.peek(T::SemiColon) && !self.peek(T::CurlyR) && !self.peek(T::Eof)
        {
            self.restore_at_mark(mark);
            return None;
        }
        Some(self.finish(node))
    }

    /// `[variableName ':'] expression | variableName '...'`
    fn parse_mixin_argument(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::FunctionArgument);

        let pos = self.mark();
        if let Some(argument) = self.parse_variable() {
            if !self.accept(T::Colon) {
                self.restore_at_mark(pos);
            } else {
                self.ast
                    .set_node(node, Field::Identifier, Some(argument), Some(0));
            }
        }

        let value = self
            .parse_detached_rule_set()
            .or_else(|| self.parse_expr(true));
        if self.ast.set_node(node, Field::Value, value, Some(0)) {
            return Some(self.finish(node));
        }
        self.restore_at_mark(pos);
        None
    }

    pub(crate) fn parse_mixin_parameter(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::FunctionParameter);

        // The rest variable `@rest...`.
        if self.peek_keyword("@rest") {
            let rest = self.create(Kind::Node);
            self.consume_token();
            if !self.accept(T::Ellipsis) {
                return Some(self.finish_resync(
                    node,
                    &DOT_EXPECTED,
                    &[],
                    &[T::Comma, T::ParenthesisR],
                ));
            }
            let rest = self.finish(rest);
            self.ast
                .set_node(node, Field::Identifier, Some(rest), Some(0));
            return Some(self.finish(node));
        }

        // Varargs `...`.
        if self.peek(T::Ellipsis) {
            let varargs = self.create(Kind::Node);
            self.consume_token();
            let varargs = self.finish(varargs);
            self.ast
                .set_node(node, Field::Identifier, Some(varargs), Some(0));
            return Some(self.finish(node));
        }

        let mut has_content = false;
        // A default value `@param: 12`, or `@name`.
        let variable = self.parse_variable();
        if self
            .ast
            .set_node(node, Field::Identifier, variable, Some(0))
        {
            self.accept(T::Colon);
            has_content = true;
        }
        let value = self
            .parse_detached_rule_set()
            .or_else(|| self.parse_expr(true));
        if !self.ast.set_node(node, Field::DefaultValue, value, Some(0)) && !has_content {
            return None;
        }
        Some(self.finish(node))
    }

    fn parse_guard(&mut self) -> Option<NodeId> {
        if !self.peek_ident("when") {
            return None;
        }
        let node = self.create(Kind::LessGuard);
        self.consume_token();

        let condition = self.parse_guard_condition();
        let conditions = self.ast.list(node, Field::Conditions);
        if !self.ast.add_child(conditions, condition) {
            return Some(self.finish_error(node, &CONDITION_EXPECTED));
        }
        while self.accept_ident("and") || self.accept(T::Comma) {
            let condition = self.parse_guard_condition();
            if !self.ast.add_child(conditions, condition) {
                return Some(self.finish_error(node, &CONDITION_EXPECTED));
            }
        }
        Some(self.finish(node))
    }

    fn parse_guard_condition(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::GuardCondition);
        let is_negated = self.accept_ident("not");
        self.ast.extra_mut(node).is_negated = Some(is_negated);
        if !self.accept(T::ParenthesisL) {
            if is_negated {
                return Some(self.finish_error(node, &LEFT_PARENTHESIS_EXPECTED));
            }
            return None;
        }

        let expr = self.parse_expr(false);
        self.ast.add_child(node, expr);

        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn less_parse_function(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.create(Kind::Function);

        let is_attr = self.peek_ident("attr");

        let identifier = self.parse_function_identifier();
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
        {
            return None;
        }

        if self.has_whitespace() || !self.accept(T::ParenthesisL) {
            self.restore_at_mark(pos);
            return None;
        }

        if is_attr {
            self.attr_depth += 1;
        }
        let argument = self.parse_mixin_argument();
        let arguments = self.ast.list(node, Field::Arguments);
        if self.ast.add_child(arguments, argument) {
            while self.accept(T::Comma) || self.accept(T::SemiColon) {
                if self.peek(T::ParenthesisR) {
                    break;
                }
                let argument = self.parse_mixin_argument();
                if !self.ast.add_child(arguments, argument) {
                    if is_attr {
                        self.attr_depth -= 1;
                    }
                    return Some(self.finish_error(node, &EXPRESSION_EXPECTED));
                }
            }
        }
        if is_attr {
            self.attr_depth -= 1;
        }

        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn less_parse_function_identifier(&mut self) -> Option<NodeId> {
        if self.peek_delim("%") {
            let node = self.create(Kind::Identifier);
            self.ast.extra_mut(node).reference_types = Some(&[ReferenceType::Function]);
            self.consume_token();
            return Some(self.finish(node));
        }
        self.css_parse_function_identifier()
    }

    pub(crate) fn less_parse_url_argument(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.css_parse_url_argument();
        if node.is_none() || !self.peek(T::ParenthesisR) {
            self.restore_at_mark(pos);
            let node = self.create(Kind::Node);
            let expr = self.parse_binary_expr(None, None);
            self.ast.add_child(node, expr);
            return Some(self.finish(node));
        }
        node
    }
}
