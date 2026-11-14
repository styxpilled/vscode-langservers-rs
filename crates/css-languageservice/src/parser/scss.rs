//! The SCSS grammar of `scssParser.ts`.

use super::Parser;
use super::errors::*;
use super::nodes::{Field, Kind, NodeId, NodeType, ReferenceType, Rule};
use super::scanner::TokenType as T;

pub static FROM_EXPECTED: Rule = Rule {
    id: "scss-fromexpected",
    message: "'from' expected",
};
pub static THROUGH_OR_TO_EXPECTED: Rule = Rule {
    id: "scss-throughexpected",
    message: "'through' or 'to' expected",
};
pub static IN_EXPECTED: Rule = Rule {
    id: "scss-fromexpected",
    message: "'in' expected",
};

type ParseFn<'a> = fn(&mut Parser<'a>) -> Option<NodeId>;

impl<'a> Parser<'a> {
    pub(crate) fn scss_parse_stylesheet_statement(&mut self, is_nested: bool) -> Option<NodeId> {
        if self.peek(T::AtKeyword) {
            return self
                .parse_warn_and_debug()
                .or_else(|| self.parse_control_statement(Self::parse_rule_set_declaration))
                .or_else(|| self.scss_parse_mixin_declaration())
                .or_else(|| self.parse_mixin_content())
                .or_else(|| self.scss_parse_mixin_reference())
                .or_else(|| self.parse_function_declaration())
                .or_else(|| self.parse_forward())
                .or_else(|| self.parse_use())
                .or_else(|| self.parse_ruleset(is_nested))
                .or_else(|| self.parse_stylesheet_at_statement(is_nested));
        }
        self.parse_variable_declaration(&[])
            .or_else(|| self.parse_ruleset(true))
    }

    pub(crate) fn scss_parse_import(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@import") {
            return None;
        }
        let node = self.create(Kind::Import);
        self.consume_token();

        loop {
            let uri = self.parse_uri_literal();
            if !self.ast.add_child(node, uri) {
                let string = self.parse_string_literal();
                if !self.ast.add_child(node, string) {
                    return Some(self.finish_error(node, &URI_OR_STRING_EXPECTED));
                }
            }
            if !self.accept(T::Comma) {
                break;
            }
        }
        self.complete_parse_import(node)
    }

    /// `$font-size: 12px;` or a variable of another module, `lib.$font-size: 12px;`.
    pub(crate) fn scss_parse_variable_declaration(&mut self, panic: &[T]) -> Option<NodeId> {
        if !self.peek(T::VariableName) && !self.peek_module_variable() {
            return None;
        }
        let node = self.create(Kind::VariableDeclaration);

        // `setVariable` attaches without `addChild`.
        let variable = self.parse_variable();
        if !self.ast.set_node(node, Field::Variable, variable, None) {
            let module = self.try_parse_module_variable();
            if !self.ast.add_child(node, module) {
                return None;
            }
        }

        if !self.accept(T::Colon) {
            return Some(self.finish_error(node, &COLON_EXPECTED));
        }
        if let Some(prev) = &self.prev_token {
            let offset = prev.offset as u32;
            self.ast.extra_mut(node).colon_position = Some(offset);
        }

        let value = self.parse_expr(false);
        if !self.ast.set_node(node, Field::Value, value, None) {
            return Some(self.finish_resync(node, &VARIABLE_VALUE_EXPECTED, &[], panic));
        }

        while self.peek(T::Exclamation) {
            let prio = self.try_parse_prio();
            if !self.ast.add_child(node, prio) {
                self.consume_token();
                if !(self.peek(T::Ident) && (self.text() == "default" || self.text() == "global")) {
                    return Some(self.finish_error(node, &UNKNOWN_KEYWORD));
                }
                self.consume_token();
            }
        }

        if self.peek(T::SemiColon) {
            let offset = self.token.offset as u32;
            self.ast.extra_mut(node).semicolon_position = Some(offset);
        }
        Some(self.finish(node))
    }

    pub(crate) fn scss_parse_media_condition(&mut self) -> Option<NodeId> {
        self.parse_interpolation()
            .or_else(|| Some(self.css_parse_media_condition()))
    }

    pub(crate) fn scss_parse_media_feature_range_operator(&mut self) -> bool {
        self.accept(T::SmallerEqualsOperator)
            || self.accept(T::GreaterEqualsOperator)
            || self.css_parse_media_feature_range_operator()
    }

    pub(crate) fn scss_parse_media_feature_name(&mut self) -> Option<NodeId> {
        self.parse_module_member()
            .or_else(|| self.parse_function())
            .or_else(|| self.parse_ident(None))
            .or_else(|| self.parse_variable())
    }

    pub(crate) fn scss_parse_keyframe_selector(&mut self) -> Option<NodeId> {
        self.try_parse_keyframe_selector()
            .or_else(|| self.parse_control_statement(Self::parse_keyframe_selector))
            .or_else(|| self.parse_warn_and_debug())
            .or_else(|| self.scss_parse_mixin_reference())
            .or_else(|| self.parse_function_declaration())
            .or_else(|| self.parse_variable_declaration(&[]))
            .or_else(|| self.parse_mixin_content())
    }

    pub(crate) fn scss_parse_variable(&mut self) -> Option<NodeId> {
        if !self.peek(T::VariableName) {
            return None;
        }
        let node = self.create(Kind::Variable);
        self.consume_token();
        Some(node)
    }

    /// An identifier followed by `.$`. Checks the text, which is cheap.
    fn peek_module_variable(&self) -> bool {
        self.peek(T::Ident)
            && self
                .scanner
                .substring(self.token.offset + self.token.len, 2)
                == ".$"
    }

    fn try_parse_module_variable(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let module = self.parse_module_member();
        if module.is_none() || !self.peek(T::Colon) {
            self.restore_at_mark(pos);
            return None;
        }
        module
    }

    pub(crate) fn parse_module_member(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.create(Kind::Module);

        let identifier = self.parse_ident(Some(&[ReferenceType::Module]));
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
        {
            return self.discard(node);
        }

        if self.has_whitespace() || !self.accept_delim(".") || self.has_whitespace() {
            self.restore_and_discard(pos);
            return None;
        }

        let member = self.parse_variable().or_else(|| self.parse_function());
        if !self.ast.add_child(node, member) {
            return Some(self.finish_error(node, &IDENTIFIER_OR_VARIABLE_EXPECTED));
        }
        Some(node)
    }

    pub(crate) fn scss_parse_ident(
        &mut self,
        reference_types: Option<&'static [ReferenceType]>,
    ) -> Option<NodeId> {
        if !self.peek(T::Ident) && !self.peek(T::InterpolationFunction) && !self.peek_delim("-") {
            return None;
        }
        let node = self.create(Kind::Identifier);
        self.ast.node_mut(node).is_custom_property = self.peek_custom_property();
        if reference_types.is_some() {
            self.ast.extra_mut(node).reference_types = reference_types;
        }

        let mut has_content = false;
        loop {
            let matched = self.accept(T::Ident)
                || {
                    let interpolation = self.ident_interpolation();
                    self.ast.add_child(node, interpolation)
                }
                || has_content && self.accept_word_start();
            if !matched {
                break;
            }
            has_content = true;
            if self.has_whitespace() {
                break;
            }
        }
        has_content.then(|| self.finish(node))
    }

    fn ident_interpolation(&mut self) -> Option<NodeId> {
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

    /// `acceptRegexp(/^[\w-]/)`: any token whose text starts with a word
    /// character or `-`.
    fn accept_word_start(&mut self) -> bool {
        let first = self.text().bytes().next();
        if first.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            self.consume_token();
            return true;
        }
        false
    }

    pub(crate) fn scss_parse_term_expression(&mut self) -> Option<NodeId> {
        self.parse_module_member()
            .or_else(|| self.parse_variable())
            .or_else(|| self.parse_nesting_selector())
            .or_else(|| self.css_parse_term_expression())
    }

    /// Sass bracketed lists: any list of values, not only identifiers.
    /// <https://sass-lang.com/documentation/values/lists/#bracketed-lists>
    pub(crate) fn scss_parse_named_line(&mut self) -> Option<NodeId> {
        if !self.peek(T::BracketL) {
            return None;
        }
        let node = self.create_node(NodeType::GridLine);
        self.consume_token();
        let expr = self.parse_expr(false);
        self.ast.add_child(node, expr);
        if !self.accept(T::BracketR) {
            return Some(self.finish_error(node, &RIGHT_SQUARE_BRACKET_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn scss_parse_interpolation(&mut self) -> Option<NodeId> {
        if !self.peek(T::InterpolationFunction) {
            return None;
        }
        let node = self.create(Kind::Interpolation);
        self.consume_token();
        let expr = self.parse_expr(false);
        if !self.ast.add_child(node, expr) && self.parse_nesting_selector().is_none() {
            if self.accept(T::CurlyR) {
                return Some(self.finish(node));
            }
            return Some(self.finish_error(node, &EXPRESSION_EXPECTED));
        }
        if !self.accept(T::CurlyR) {
            return Some(self.finish_error(node, &RIGHT_CURLY_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn scss_parse_operator(&mut self) -> Option<NodeId> {
        if self.peek_one(&[
            T::EqualsOperator,
            T::NotEqualsOperator,
            T::GreaterEqualsOperator,
            T::SmallerEqualsOperator,
        ]) || self.peek_delim(">")
            || self.peek_delim("<")
            || self.peek_ident("and")
            || self.peek_ident("or")
            || self.peek_delim("%")
        {
            let node = self.create_node(NodeType::Operator);
            self.consume_token();
            return Some(self.finish(node));
        }
        self.css_parse_operator()
    }

    pub(crate) fn scss_parse_unary_operator(&mut self) -> Option<NodeId> {
        if self.peek_ident("not") {
            let node = self.create(Kind::Node);
            self.consume_token();
            return Some(self.finish(node));
        }
        self.css_parse_unary_operator()
    }

    pub(crate) fn scss_parse_rule_set_declaration(&mut self) -> Option<NodeId> {
        if self.peek(T::AtKeyword) {
            return self
                .parse_keyframe()
                .or_else(|| self.parse_import())
                .or_else(|| self.parse_media(true))
                .or_else(|| self.parse_font_face())
                .or_else(|| self.parse_warn_and_debug())
                .or_else(|| self.parse_control_statement(Self::parse_rule_set_declaration))
                .or_else(|| self.parse_function_declaration())
                .or_else(|| self.parse_extends())
                .or_else(|| self.scss_parse_mixin_reference())
                .or_else(|| self.parse_mixin_content())
                .or_else(|| self.scss_parse_mixin_declaration())
                .or_else(|| self.parse_ruleset(true))
                .or_else(|| self.parse_supports(true))
                .or_else(|| self.parse_layer(false))
                .or_else(|| self.parse_property_at_rule())
                .or_else(|| self.parse_container(true))
                .or_else(|| self.parse_rule_set_declaration_at_statement());
        }
        self.parse_variable_declaration(&[])
            .or_else(|| self.try_parse_ruleset(true))
            // A keyframe selector, such as `0%` in content included into `@keyframes`.
            .or_else(|| self.try_parse_keyframe_selector())
            // Last, so that in the error case the AST has a declaration.
            .or_else(|| self.parse_declaration(None, false))
    }

    pub(crate) fn scss_parse_declaration(&mut self, stop_tokens: Option<&[T]>) -> Option<NodeId> {
        if let Some(custom) = self.try_parse_custom_property_declaration(stop_tokens, false) {
            return Some(custom);
        }

        let node = self.create(Kind::Declaration);
        let property = self.parse_property();
        if !self.ast.set_node(node, Field::Property, property, None) {
            return None;
        }

        if !self.accept(T::Colon) {
            return Some(self.finish_resync(
                node,
                &COLON_EXPECTED,
                &[T::Colon],
                stop_tokens.unwrap_or(&[T::SemiColon]),
            ));
        }
        if let Some(prev) = &self.prev_token {
            let offset = prev.offset as u32;
            self.ast.extra_mut(node).colon_position = Some(offset);
        }

        let mut has_content = false;
        let value = self.parse_expr(false);
        if self.ast.set_node(node, Field::Value, value, None) {
            has_content = true;
            let prio = self.parse_prio();
            self.ast.add_child(node, prio);
        }
        if self.peek(T::CurlyL) {
            let nested = self.parse_nested_properties();
            self.ast
                .set_node(node, Field::NestedProperties, Some(nested), None);
        } else if !has_content {
            return Some(self.finish_error(node, &PROPERTY_VALUE_EXPECTED));
        }
        if self.peek(T::SemiColon) {
            let offset = self.token.offset as u32;
            self.ast.extra_mut(node).semicolon_position = Some(offset);
        }
        Some(self.finish(node))
    }

    fn parse_nested_properties(&mut self) -> NodeId {
        let node = self.create(Kind::NestedProperties);
        self.parse_body(node, |p| p.parse_declaration(None, false))
    }

    fn parse_extends(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@extend") {
            return None;
        }
        let node = self.create(Kind::ExtendsReference);
        self.consume_token();
        let selector = self.parse_simple_selector();
        let selectors = self.ast.list(node, Field::Selectors);
        if !self.ast.add_child(selectors, selector) {
            return Some(self.finish_error(node, &SELECTOR_EXPECTED));
        }
        while self.accept(T::Comma) {
            let selector = self.parse_simple_selector();
            self.ast.add_child(selectors, selector);
        }
        if self.accept(T::Exclamation) && !self.accept_ident("optional") {
            return Some(self.finish_error(node, &UNKNOWN_KEYWORD));
        }
        Some(self.finish(node))
    }

    pub(crate) fn scss_parse_simple_selector_body(&mut self) -> Option<NodeId> {
        self.parse_selector_placeholder()
            .or_else(|| self.css_parse_simple_selector_body())
    }

    pub(crate) fn scss_parse_nesting_selector(&mut self) -> Option<NodeId> {
        if !self.peek_delim("&") {
            return None;
        }
        let node = self.create_node(NodeType::SelectorCombinator);
        self.consume_token();
        // `&-foo-1`
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

    fn parse_selector_placeholder(&mut self) -> Option<NodeId> {
        if self.peek_delim("%") {
            let node = self.create_node(NodeType::SelectorPlaceholder);
            self.consume_token();
            self.parse_ident(None);
            return Some(self.finish(node));
        } else if self.peek_keyword("@at-root") {
            let node = self.create_node(NodeType::SelectorPlaceholder);
            self.consume_token();
            if self.accept(T::ParenthesisL) {
                if !self.accept_ident("with") && !self.accept_ident("without") {
                    return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
                }
                if !self.accept(T::Colon) {
                    return Some(self.finish_error(node, &COLON_EXPECTED));
                }
                let ident = self.parse_ident(None);
                if !self.ast.add_child(node, ident) {
                    return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
                }
                if !self.accept(T::ParenthesisR) {
                    return Some(self.finish_resync(
                        node,
                        &RIGHT_PARENTHESIS_EXPECTED,
                        &[T::CurlyR],
                        &[],
                    ));
                }
            }
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn scss_parse_element_name(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.css_parse_element_name();
        // microsoft/vscode#49589
        if node.is_some() && !self.has_whitespace() && self.peek(T::ParenthesisL) {
            self.restore_and_discard(pos);
            return None;
        }
        node
    }

    pub(crate) fn scss_try_parse_pseudo_identifier(&mut self) -> Option<NodeId> {
        self.parse_interpolation()
            .or_else(|| self.css_try_parse_pseudo_identifier())
    }

    /// `@warn`, `@debug` and `@error`.
    fn parse_warn_and_debug(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@debug")
            && !self.peek_keyword("@warn")
            && !self.peek_keyword("@error")
        {
            return None;
        }
        let node = self.create_node(NodeType::Debug);
        self.consume_token();
        let expr = self.parse_expr(false);
        self.ast.add_child(node, expr);
        Some(self.finish(node))
    }

    /// `@if`, `@for`, `@each` and `@while`.
    fn parse_control_statement(&mut self, parse_statement: ParseFn<'a>) -> Option<NodeId> {
        if !self.peek(T::AtKeyword) {
            return None;
        }
        self.parse_if_statement(parse_statement)
            .or_else(|| self.parse_for_statement(parse_statement))
            .or_else(|| self.parse_each_statement(parse_statement))
            .or_else(|| self.parse_while_statement(parse_statement))
    }

    fn parse_if_statement(&mut self, parse_statement: ParseFn<'a>) -> Option<NodeId> {
        if !self.peek_keyword("@if") {
            return None;
        }
        Some(self.internal_parse_if_statement(parse_statement))
    }

    fn internal_parse_if_statement(&mut self, parse_statement: ParseFn<'a>) -> NodeId {
        let node = self.create(Kind::IfStatement);
        self.consume_token();
        let expr = self.parse_expr(true);
        if !self.ast.set_node(node, Field::Expression, expr, Some(0)) {
            return self.finish_error(node, &EXPRESSION_EXPECTED);
        }
        self.parse_body(node, parse_statement);
        if self.accept_keyword("@else") {
            if self.peek_ident("if") {
                let else_clause = self.internal_parse_if_statement(parse_statement);
                self.ast
                    .set_node(node, Field::ElseClause, Some(else_clause), None);
            } else if self.peek(T::CurlyL) {
                let else_node = self.create(Kind::ElseStatement);
                self.parse_body(else_node, parse_statement);
                self.ast
                    .set_node(node, Field::ElseClause, Some(else_node), None);
            }
        }
        self.finish(node)
    }

    fn parse_for_statement(&mut self, parse_statement: ParseFn<'a>) -> Option<NodeId> {
        if !self.peek_keyword("@for") {
            return None;
        }
        const STOP: &[T] = &[T::CurlyR];
        let node = self.create(Kind::ForStatement);
        self.consume_token();
        let variable = self.parse_variable();
        if !self.ast.set_node(node, Field::Variable, variable, Some(0)) {
            return Some(self.finish_resync(node, &VARIABLE_NAME_EXPECTED, STOP, &[]));
        }
        if !self.accept_ident("from") {
            return Some(self.finish_resync(node, &FROM_EXPECTED, STOP, &[]));
        }
        let from = self.parse_binary_expr(None, None);
        if !self.ast.add_child(node, from) {
            return Some(self.finish_resync(node, &EXPRESSION_EXPECTED, STOP, &[]));
        }
        if !self.accept_ident("to") && !self.accept_ident("through") {
            return Some(self.finish_resync(node, &THROUGH_OR_TO_EXPECTED, STOP, &[]));
        }
        let to = self.parse_binary_expr(None, None);
        if !self.ast.add_child(node, to) {
            return Some(self.finish_resync(node, &EXPRESSION_EXPECTED, STOP, &[]));
        }
        Some(self.parse_body(node, parse_statement))
    }

    fn parse_each_statement(&mut self, parse_statement: ParseFn<'a>) -> Option<NodeId> {
        if !self.peek_keyword("@each") {
            return None;
        }
        const STOP: &[T] = &[T::CurlyR];
        let node = self.create(Kind::EachStatement);
        self.consume_token();
        let variables = self.ast.list(node, Field::Variables);
        let variable = self.parse_variable();
        if !self.ast.add_child(variables, variable) {
            return Some(self.finish_resync(node, &VARIABLE_NAME_EXPECTED, STOP, &[]));
        }
        while self.accept(T::Comma) {
            let variable = self.parse_variable();
            if !self.ast.add_child(variables, variable) {
                return Some(self.finish_resync(node, &VARIABLE_NAME_EXPECTED, STOP, &[]));
            }
        }
        self.finish(variables);
        if !self.accept_ident("in") {
            return Some(self.finish_resync(node, &IN_EXPECTED, STOP, &[]));
        }
        let expr = self.parse_expr(false);
        if !self.ast.add_child(node, expr) {
            return Some(self.finish_resync(node, &EXPRESSION_EXPECTED, STOP, &[]));
        }
        Some(self.parse_body(node, parse_statement))
    }

    fn parse_while_statement(&mut self, parse_statement: ParseFn<'a>) -> Option<NodeId> {
        if !self.peek_keyword("@while") {
            return None;
        }
        let node = self.create(Kind::WhileStatement);
        self.consume_token();
        let expr = self.parse_binary_expr(None, None);
        if !self.ast.add_child(node, expr) {
            return Some(self.finish_resync(node, &EXPRESSION_EXPECTED, &[T::CurlyR], &[]));
        }
        Some(self.parse_body(node, parse_statement))
    }

    fn parse_function_body_declaration(&mut self) -> Option<NodeId> {
        self.parse_variable_declaration(&[])
            .or_else(|| self.parse_return_statement())
            .or_else(|| self.parse_warn_and_debug())
            .or_else(|| self.parse_control_statement(Self::parse_function_body_declaration))
    }

    fn parse_function_declaration(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@function") {
            return None;
        }
        const STOP: &[T] = &[T::CurlyR];
        let node = self.create(Kind::FunctionDeclaration);
        self.consume_token();

        let identifier = self.parse_ident(Some(&[ReferenceType::Function]));
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
        {
            return Some(self.finish_resync(node, &IDENTIFIER_EXPECTED, STOP, &[]));
        }
        if !self.accept(T::ParenthesisL) {
            return Some(self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, STOP, &[]));
        }
        if let Some(error) = self.parse_parameter_list(node) {
            return Some(error);
        }
        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, STOP, &[]));
        }
        Some(self.parse_body(node, Self::parse_function_body_declaration))
    }

    /// The parameter list of a function, mixin or `using` clause. Returns the
    /// finished node on a missing parameter.
    fn parse_parameter_list(&mut self, node: NodeId) -> Option<NodeId> {
        let parameter = self.parse_parameter_declaration();
        let parameters = self.ast.list(node, Field::Parameters);
        if self.ast.add_child(parameters, parameter) {
            while self.accept(T::Comma) {
                if self.peek(T::ParenthesisR) {
                    break;
                }
                let parameter = self.parse_parameter_declaration();
                if !self.ast.add_child(parameters, parameter) {
                    return Some(self.finish_error(node, &VARIABLE_NAME_EXPECTED));
                }
            }
        }
        None
    }

    fn parse_return_statement(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@return") {
            return None;
        }
        let node = self.create_node(NodeType::ReturnStatement);
        self.consume_token();
        let expr = self.parse_expr(false);
        if !self.ast.add_child(node, expr) {
            return Some(self.finish_error(node, &EXPRESSION_EXPECTED));
        }
        Some(self.finish(node))
    }

    fn scss_parse_mixin_declaration(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@mixin") {
            return None;
        }
        const STOP: &[T] = &[T::CurlyR];
        let node = self.create(Kind::MixinDeclaration);
        self.consume_token();

        let identifier = self.parse_ident(Some(&[ReferenceType::Mixin]));
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
        {
            return Some(self.finish_resync(node, &IDENTIFIER_EXPECTED, STOP, &[]));
        }

        if self.accept(T::ParenthesisL) {
            if let Some(error) = self.parse_parameter_list(node) {
                return Some(error);
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, STOP, &[]));
            }
        }

        Some(self.parse_body(node, Self::parse_rule_set_declaration))
    }

    fn parse_parameter_declaration(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::FunctionParameter);

        let variable = self.parse_variable();
        if !self
            .ast
            .set_node(node, Field::Identifier, variable, Some(0))
        {
            return None;
        }

        self.accept(T::Ellipsis);

        if self.accept(T::Colon) {
            let value = self.parse_expr(true);
            if !self.ast.set_node(node, Field::DefaultValue, value, Some(0)) {
                return Some(self.finish_resync(
                    node,
                    &VARIABLE_VALUE_EXPECTED,
                    &[],
                    &[T::Comma, T::ParenthesisR],
                ));
            }
        }
        Some(self.finish(node))
    }

    fn parse_mixin_content(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@content") {
            return None;
        }
        let node = self.create(Kind::MixinContentReference);
        self.consume_token();
        if self.accept(T::ParenthesisL) {
            if let Some(error) = self.parse_argument_list(node) {
                return Some(error);
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
            }
        }
        Some(self.finish(node))
    }

    /// The arguments of `@content` and `@include`. Returns the finished node on
    /// a missing argument.
    fn parse_argument_list(&mut self, node: NodeId) -> Option<NodeId> {
        let argument = self.parse_function_argument();
        let arguments = self.ast.list(node, Field::Arguments);
        if self.ast.add_child(arguments, argument) {
            while self.accept(T::Comma) {
                if self.peek(T::ParenthesisR) {
                    break;
                }
                let argument = self.parse_function_argument();
                if !self.ast.add_child(arguments, argument) {
                    return Some(self.finish_error(node, &EXPRESSION_EXPECTED));
                }
            }
        }
        None
    }

    fn scss_parse_mixin_reference(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@include") {
            return None;
        }
        const STOP: &[T] = &[T::CurlyR];
        let node = self.create(Kind::MixinReference);
        self.consume_token();

        // A module or a mixin identifier; mixin by default.
        let first_ident = self.parse_ident(Some(&[ReferenceType::Mixin]));
        if !self
            .ast
            .set_node(node, Field::Identifier, first_ident, Some(0))
        {
            return Some(self.finish_resync(node, &IDENTIFIER_EXPECTED, STOP, &[]));
        }
        let first_ident = first_ident.unwrap();

        // A module accessor.
        if !self.has_whitespace() && self.accept_delim(".") && !self.has_whitespace() {
            let Some(second_ident) = self.parse_ident(Some(&[ReferenceType::Mixin])) else {
                return Some(self.finish_resync(node, &IDENTIFIER_EXPECTED, STOP, &[]));
            };

            let module = self.create(Kind::Module);
            // The first identifier is the identifier of the module.
            self.ast.extra_mut(first_ident).reference_types = Some(&[ReferenceType::Module]);
            self.ast
                .set_node(module, Field::Identifier, Some(first_ident), Some(0));

            self.ast
                .set_node(node, Field::Identifier, Some(second_ident), Some(0));
            self.ast.add_child(node, Some(module));
        }

        if self.accept(T::ParenthesisL) {
            if let Some(error) = self.parse_argument_list(node) {
                return Some(error);
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
            }
        }

        if self.peek_ident("using") || self.peek(T::CurlyL) {
            let content = self.parse_mixin_content_declaration();
            self.ast.set_node(node, Field::Content, Some(content), None);
        }
        Some(self.finish(node))
    }

    fn parse_mixin_content_declaration(&mut self) -> NodeId {
        let node = self.create(Kind::MixinContentDeclaration);
        if self.accept_ident("using") {
            if !self.accept(T::ParenthesisL) {
                return self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[T::CurlyL], &[]);
            }
            if let Some(error) = self.parse_parameter_list(node) {
                return error;
            }
            if !self.accept(T::ParenthesisR) {
                return self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[T::CurlyL], &[]);
            }
        }
        if self.peek(T::CurlyL) {
            self.parse_body(node, Self::parse_rule_set_declaration);
        }
        self.finish(node)
    }

    pub(crate) fn scss_parse_if_test(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Node);
        if self.accept_ident("sass") {
            if self.has_whitespace() || !self.accept(T::ParenthesisL) {
                return Some(self.finish_resync(
                    node,
                    &LEFT_PARENTHESIS_EXPECTED,
                    &[],
                    &[T::CurlyL],
                ));
            }
            let expr = self.parse_expr(false);
            self.ast.add_child(node, expr);
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(
                    node,
                    &RIGHT_PARENTHESIS_EXPECTED,
                    &[],
                    &[T::CurlyL],
                ));
            }
            return Some(self.finish(node));
        }
        self.css_parse_if_test()
    }

    pub(crate) fn scss_parse_function(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.create(Kind::Function);

        let is_if = self.peek_ident("if");

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

        let mut if_branches = false;
        let first_argument = if !is_if {
            self.parse_function_argument()
        } else {
            let pos = self.mark();
            let branch = self.parse_if_branch();
            if !self.ast.is_erroneous(branch, false) {
                if_branches = true;
                Some(branch)
            } else {
                self.restore_at_mark(pos);
                self.parse_function_argument()
            }
        };
        let separator = if if_branches { T::SemiColon } else { T::Comma };

        let arguments = self.ast.list(node, Field::Arguments);
        if self.ast.add_child(arguments, first_argument) {
            while self.accept(separator) {
                if self.peek(T::ParenthesisR) {
                    break;
                }
                let argument = if if_branches {
                    Some(self.parse_if_branch())
                } else {
                    self.parse_function_argument()
                };
                if !self.ast.add_child(arguments, argument) {
                    self.mark_error(node, &EXPRESSION_EXPECTED);
                }
            }
        }

        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    /// `[variableName ':'] expression | variableName '...'`
    pub(crate) fn scss_parse_function_argument(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::FunctionArgument);

        let pos = self.mark();
        if let Some(argument) = self.parse_variable() {
            if !self.accept(T::Colon) {
                if self.accept(T::Ellipsis) {
                    self.ast
                        .set_node(node, Field::Value, Some(argument), Some(0));
                    return Some(self.finish(node));
                }
                self.restore_at_mark(pos);
            } else {
                self.ast
                    .set_node(node, Field::Identifier, Some(argument), Some(0));
            }
        }

        let value = self.parse_expr(true);
        if self.ast.set_node(node, Field::Value, value, Some(0)) {
            // microsoft/vscode#43746
            self.accept(T::Ellipsis);
            // microsoft/vscode#9859
            let prio = self.parse_prio();
            self.ast.add_child(node, prio);
            return Some(self.finish(node));
        }
        let prio = self.try_parse_prio();
        if self.ast.set_node(node, Field::Value, prio, Some(0)) {
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn scss_parse_url_argument(&mut self) -> Option<NodeId> {
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

    pub(crate) fn scss_parse_operation(&mut self) -> Option<NodeId> {
        if !self.peek(T::ParenthesisL) {
            return None;
        }
        let node = self.create(Kind::Node);
        self.consume_token();
        loop {
            let element = self.parse_list_element();
            if !self.ast.add_child(node, element) {
                break;
            }
            self.accept(T::Comma);
        }
        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    fn parse_list_element(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::ListEntry);
        let child = self.parse_binary_expr(None, None)?;
        if self.accept(T::Colon) {
            self.ast.set_node(node, Field::Key, Some(child), Some(0));
            let value = self.parse_binary_expr(None, None);
            if !self.ast.set_node(node, Field::Value, value, Some(1)) {
                return Some(self.finish_error(node, &EXPRESSION_EXPECTED));
            }
        } else {
            self.ast.set_node(node, Field::Value, Some(child), Some(1));
        }
        Some(self.finish(node))
    }

    fn parse_use(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@use") {
            return None;
        }
        let node = self.create(Kind::Use);
        self.consume_token();

        let string = self.parse_string_literal();
        if !self.ast.add_child(node, string) {
            return Some(self.finish_error(node, &STRING_LITERAL_EXPECTED));
        }

        if !self.peek(T::SemiColon) && !self.peek(T::Eof) {
            // Upstream tests `/as|with/`, which has no anchors.
            if !(self.peek(T::Ident)
                && (self.text().contains("as") || self.text().contains("with")))
            {
                return Some(self.finish_error(node, &UNKNOWN_KEYWORD));
            }

            if self.accept_ident("as") {
                let ident = self.parse_ident(Some(&[ReferenceType::Module]));
                if !self.ast.set_node(node, Field::Identifier, ident, Some(0))
                    && !self.accept_delim("*")
                {
                    return Some(self.finish_error(node, &IDENTIFIER_OR_WILDCARD_EXPECTED));
                }
            }

            if self.accept_ident("with") {
                let config = self.parse_module_config();
                if !self.ast.set_node(node, Field::Parameters, config, None) {
                    return Some(self.finish_resync(
                        node,
                        &LEFT_PARENTHESIS_EXPECTED,
                        &[T::ParenthesisR],
                        &[],
                    ));
                }
            }
        }

        if !self.accept(T::SemiColon) && !self.accept(T::Eof) {
            return Some(self.finish_error(node, &SEMI_COLON_EXPECTED));
        }
        Some(self.finish(node))
    }

    fn parse_module_config(&mut self) -> Option<NodeId> {
        let node = self.create_node(NodeType::ModuleConfig);
        if !self.accept(T::ParenthesisL) {
            return None;
        }

        let declaration = self.parse_module_config_declaration();
        if !self.ast.add_child(node, declaration) {
            return Some(self.finish_error(node, &VARIABLE_NAME_EXPECTED));
        }
        while self.accept(T::Comma) {
            if self.peek(T::ParenthesisR) {
                break;
            }
            let declaration = self.parse_module_config_declaration();
            if !self.ast.add_child(node, declaration) {
                return Some(self.finish_error(node, &VARIABLE_NAME_EXPECTED));
            }
        }

        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    fn parse_module_config_declaration(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::ModuleConfiguration);

        let variable = self.parse_variable();
        if !self
            .ast
            .set_node(node, Field::Identifier, variable, Some(0))
        {
            return None;
        }

        let has_value = self.accept(T::Colon) && {
            let value = self.parse_expr(true);
            self.ast.set_node(node, Field::Value, value, Some(0))
        };
        if !has_value {
            return Some(self.finish_resync(
                node,
                &VARIABLE_VALUE_EXPECTED,
                &[],
                &[T::Comma, T::ParenthesisR],
            ));
        }

        if self.accept(T::Exclamation) && (self.has_whitespace() || !self.accept_ident("default")) {
            return Some(self.finish_error(node, &UNKNOWN_KEYWORD));
        }
        Some(self.finish(node))
    }

    fn parse_forward(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@forward") {
            return None;
        }
        let node = self.create(Kind::Forward);
        self.consume_token();

        let string = self.parse_string_literal();
        if !self.ast.add_child(node, string) {
            return Some(self.finish_error(node, &STRING_LITERAL_EXPECTED));
        }

        if self.accept_ident("as") {
            let identifier = self.parse_ident(Some(&[ReferenceType::Forward]));
            if !self
                .ast
                .set_node(node, Field::Identifier, identifier, Some(0))
            {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
            // The wildcard must follow the identifier directly.
            if self.has_whitespace() || !self.accept_delim("*") {
                return Some(self.finish_error(node, &WILDCARD_EXPECTED));
            }
        }

        if self.accept_ident("with") {
            let config = self.parse_module_config();
            if !self.ast.set_node(node, Field::Parameters, config, None) {
                return Some(self.finish_resync(
                    node,
                    &LEFT_PARENTHESIS_EXPECTED,
                    &[T::ParenthesisR],
                    &[],
                ));
            }
        } else if self.peek_ident("hide") || self.peek_ident("show") {
            let visibility = self.parse_forward_visibility();
            if !self.ast.add_child(node, visibility) {
                return Some(self.finish_error(node, &IDENTIFIER_OR_VARIABLE_EXPECTED));
            }
        }

        if !self.accept(T::SemiColon) && !self.accept(T::Eof) {
            return Some(self.finish_error(node, &SEMI_COLON_EXPECTED));
        }
        Some(self.finish(node))
    }

    fn parse_forward_visibility(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::ForwardVisibility);

        // `hide` or `show`.
        let ident = self.parse_ident(None);
        self.ast.set_node(node, Field::Identifier, ident, Some(0));

        loop {
            let item = self.parse_variable().or_else(|| self.parse_ident(None));
            if !self.ast.add_child(node, item) {
                break;
            }
            self.accept(T::Comma);
        }

        (self.ast.children(node).nth(1).is_some()).then_some(node)
    }

    pub(crate) fn scss_parse_supports_condition_in_parens(&mut self) -> NodeId {
        match self.parse_interpolation() {
            Some(node) => node,
            None => self.css_parse_supports_condition_in_parens(),
        }
    }
}

impl Parser<'_> {
    pub(crate) fn parse_scss_by_name(&mut self, name: &str) -> Option<NodeId> {
        match name {
            "_parseForward" => self.parse_forward(),
            "_parseUse" => self.parse_use(),
            "_parseMixinContent" => self.parse_mixin_content(),
            _ => unreachable!(),
        }
    }
}
