//! The CSS grammar of `cssParser.ts`.

use super::errors::*;
use super::nodes::{Field, Kind, NodeId, NodeType, ReferenceType};
use super::scanner::TokenType as T;
use super::{Dialect, Parser};

const PAGE_BOX_DIRECTIVES: &[&str] = &[
    "@bottom-center",
    "@bottom-left",
    "@bottom-left-corner",
    "@bottom-right",
    "@bottom-right-corner",
    "@left-bottom",
    "@left-middle",
    "@left-top",
    "@right-bottom",
    "@right-middle",
    "@right-top",
    "@top-center",
    "@top-left",
    "@top-left-corner",
    "@top-right",
    "@top-right-corner",
];

fn is_hex_color(text: &str) -> bool {
    let Some(hex) = text.strip_prefix('#') else {
        return false;
    };
    !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `/^@(\-(webkit|ms|moz|o)\-)?keyframes$/i`.
fn is_keyframes_keyword(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('@') else {
        return false;
    };
    let rest = rest.to_ascii_lowercase();
    let rest = ["-webkit-", "-ms-", "-moz-", "-o-"]
        .iter()
        .find_map(|p| rest.strip_prefix(p))
        .unwrap_or(&rest);
    rest == "keyframes"
}

impl<'a> Parser<'a> {
    pub(crate) fn peek_custom_property(&self) -> bool {
        self.peek(T::Ident) && self.text().starts_with("--")
    }

    pub(crate) fn parse_stylesheet_root(&mut self) -> NodeId {
        let node = self.create(Kind::Stylesheet);

        loop {
            let start = self.parse_stylesheet_start();
            if !self.ast.add_child(node, start) {
                break;
            }
        }

        let mut in_recovery = false;
        loop {
            let mut has_match;
            loop {
                has_match = false;
                if let Some(statement) = self.parse_stylesheet_statement(false) {
                    self.ast.add_child(node, Some(statement));
                    has_match = true;
                    in_recovery = false;
                    if !self.peek(T::Eof)
                        && self.needs_semicolon_after(statement)
                        && !self.accept(T::SemiColon)
                    {
                        self.mark_error(node, &SEMI_COLON_EXPECTED);
                    }
                }
                while self.accept(T::SemiColon) || self.accept(T::Cdo) || self.accept(T::Cdc) {
                    has_match = true;
                    in_recovery = false;
                }
                if !has_match {
                    break;
                }
            }

            if self.peek(T::Eof) {
                break;
            }

            if !in_recovery {
                if self.peek(T::AtKeyword) {
                    self.mark_error(node, &UNKNOWN_AT_RULE);
                } else {
                    self.mark_error(node, &RULE_OR_SELECTOR_EXPECTED);
                }
                in_recovery = true;
            }
            self.consume_token();
            if self.peek(T::Eof) {
                break;
            }
        }

        self.finish(node)
    }

    pub(crate) fn parse_stylesheet_start(&mut self) -> Option<NodeId> {
        self.parse_charset()
    }

    pub(crate) fn parse_stylesheet_statement(&mut self, is_nested: bool) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_stylesheet_statement(is_nested),
            Dialect::Less => self.less_parse_stylesheet_statement(is_nested),
            _ => self.css_parse_stylesheet_statement(is_nested),
        }
    }

    pub(crate) fn css_parse_stylesheet_statement(&mut self, is_nested: bool) -> Option<NodeId> {
        if self.peek(T::AtKeyword) {
            return self.parse_stylesheet_at_statement(is_nested);
        }
        self.parse_ruleset(is_nested)
    }

    pub(crate) fn parse_stylesheet_at_statement(&mut self, is_nested: bool) -> Option<NodeId> {
        self.parse_import()
            .or_else(|| self.parse_media(is_nested))
            .or_else(|| self.parse_scope())
            .or_else(|| self.parse_page())
            .or_else(|| self.parse_font_face())
            .or_else(|| self.parse_keyframe())
            .or_else(|| self.parse_supports(is_nested))
            .or_else(|| self.parse_layer(is_nested))
            .or_else(|| self.parse_property_at_rule())
            .or_else(|| self.parse_view_port())
            .or_else(|| self.parse_namespace())
            .or_else(|| self.parse_document())
            .or_else(|| self.parse_container(is_nested))
            .or_else(|| self.parse_starting_style_at_rule(is_nested))
            .or_else(|| self.parse_unknown_at_rule())
    }

    pub(crate) fn try_parse_ruleset(&mut self, is_nested: bool) -> Option<NodeId> {
        if self.dialect == Dialect::Css && !self.may_be_ruleset() {
            return None;
        }
        let mark = self.mark();
        if self.parse_selector(is_nested).is_some() {
            while self.accept(T::Comma) && self.parse_selector(is_nested).is_some() {}
            if self.accept(T::CurlyL) {
                self.restore_and_discard(mark);
                return self.parse_ruleset(is_nested);
            }
        }
        self.restore_and_discard(mark);
        None
    }

    /// Not upstream. Before a `(` or `[`, the CSS selectors and the commas
    /// between them consume only these token types. If the first other token
    /// is not `{`, `(` or `[`, `try_parse_ruleset` fails. Its errors are on
    /// tokens after the first, so the skip changes no output.
    fn may_be_ruleset(&mut self) -> bool {
        let pos = self.scanner.pos();
        let mut kind = self.token.kind;
        while matches!(kind, T::Ident | T::Delim | T::Hash | T::Colon | T::Comma) {
            kind = self.scanner.scan().kind;
        }
        self.scanner.go_back_to(pos);
        matches!(kind, T::CurlyL | T::ParenthesisL | T::BracketL)
    }

    pub(crate) fn parse_ruleset(&mut self, is_nested: bool) -> Option<NodeId> {
        let node = self.create(Kind::RuleSet);
        let selectors = self.ast.list(node, Field::Selectors);

        let selector = self.parse_selector(is_nested);
        if !self.ast.add_child(selectors, selector) {
            return None;
        }

        while self.accept(T::Comma) {
            let selector = self.parse_selector(is_nested);
            if !self.ast.add_child(selectors, selector) {
                return Some(self.finish_error(node, &SELECTOR_EXPECTED));
            }
        }

        Some(self.parse_body(node, |p| p.parse_rule_set_declaration()))
    }

    pub(crate) fn parse_rule_set_declaration_at_statement(&mut self) -> Option<NodeId> {
        self.parse_media(true)
            .or_else(|| self.parse_scope())
            .or_else(|| self.parse_supports(true))
            .or_else(|| self.parse_layer(true))
            .or_else(|| self.parse_container(true))
            .or_else(|| self.parse_starting_style_at_rule(true))
            .or_else(|| self.parse_unknown_at_rule())
    }

    pub(crate) fn parse_rule_set_declaration(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_rule_set_declaration(),
            Dialect::Less => self.less_parse_rule_set_declaration(),
            _ => self.css_parse_rule_set_declaration(),
        }
    }

    pub(crate) fn css_parse_rule_set_declaration(&mut self) -> Option<NodeId> {
        if self.peek(T::AtKeyword) {
            return self.parse_rule_set_declaration_at_statement();
        }
        if !self.peek(T::Ident) {
            return self.parse_ruleset(true);
        }
        self.try_parse_ruleset(true)
            .or_else(|| self.parse_declaration(None, false))
    }

    pub(crate) fn needs_semicolon_after(&self, node: NodeId) -> bool {
        use NodeType::*;
        match self.ast.node_type(node) {
            Keyframe
            | ViewPort
            | Media
            | Ruleset
            | Namespace
            | If
            | For
            | Each
            | While
            | MixinDeclaration
            | FunctionDeclaration
            | MixinContentDeclaration
            | Scope => false,
            ExtendsReference
            | MixinContentReference
            | ReturnStatement
            | MediaQuery
            | Debug
            | Import
            | AtApplyRule
            | CustomPropertyDeclaration => true,
            VariableDeclaration => self.ast.extra(node).is_none_or(|e| e.needs_semicolon),
            MixinReference => self.ast.field(node, Field::Content).is_none(),
            Declaration => self.ast.field(node, Field::NestedProperties).is_none(),
            _ => false,
        }
    }

    pub(crate) fn parse_declarations(
        &mut self,
        mut parse_declaration: impl FnMut(&mut Self) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let node = self.create(Kind::Declarations);
        if !self.accept(T::CurlyL) {
            return None;
        }

        let mut decl = parse_declaration(self);
        while self.ast.add_child(node, decl) {
            if self.peek(T::CurlyR) {
                break;
            }
            let d = decl.unwrap();
            if self.needs_semicolon_after(d) && !self.accept(T::SemiColon) {
                return Some(self.finish_resync(
                    node,
                    &SEMI_COLON_EXPECTED,
                    &[T::SemiColon, T::CurlyR],
                    &[],
                ));
            }
            if let Some(prev) = &self.prev_token
                && prev.kind == T::SemiColon
            {
                let offset = prev.offset as u32;
                self.ast.extra_mut(d).semicolon_position = Some(offset);
            }
            while self.accept(T::SemiColon) {}
            decl = parse_declaration(self);
        }

        if !self.accept(T::CurlyR) {
            return Some(self.finish_resync(
                node,
                &RIGHT_CURLY_EXPECTED,
                &[T::CurlyR, T::SemiColon],
                &[],
            ));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_body(
        &mut self,
        node: NodeId,
        parse_declaration: impl FnMut(&mut Self) -> Option<NodeId>,
    ) -> NodeId {
        let declarations = self.parse_declarations(parse_declaration);
        if !self
            .ast
            .set_node(node, Field::Declarations, declarations, None)
        {
            return self.finish_resync(node, &LEFT_CURLY_EXPECTED, &[T::CurlyR, T::SemiColon], &[]);
        }
        self.finish(node)
    }

    pub(crate) fn parse_selector(&mut self, is_nested: bool) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_selector(is_nested),
            _ => self.css_parse_selector(is_nested),
        }
    }

    pub(crate) fn css_parse_selector(&mut self, is_nested: bool) -> Option<NodeId> {
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
            let combinator = self.parse_combinator();
            self.ast.add_child(node, combinator);
        }
        if !has_content {
            return self.discard(node);
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_declaration(
        &mut self,
        stop_tokens: Option<&[T]>,
        standalone_custom_property_valid: bool,
    ) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_declaration(stop_tokens),
            _ => self.css_parse_declaration(stop_tokens, standalone_custom_property_valid),
        }
    }

    pub(crate) fn css_parse_declaration(
        &mut self,
        stop_tokens: Option<&[T]>,
        standalone_custom_property_valid: bool,
    ) -> Option<NodeId> {
        if let Some(custom) = self
            .try_parse_custom_property_declaration(stop_tokens, standalone_custom_property_valid)
        {
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

        let value = self.parse_expr(false);
        if !self.ast.set_node(node, Field::Value, value, None) {
            return Some(self.finish_error(node, &PROPERTY_VALUE_EXPECTED));
        }

        let prio = self.parse_prio();
        self.ast.add_child(node, prio);
        if self.peek(T::SemiColon) {
            let offset = self.token.offset as u32;
            self.ast.extra_mut(node).semicolon_position = Some(offset);
        }
        Some(self.finish(node))
    }

    pub(crate) fn try_parse_custom_property_declaration(
        &mut self,
        stop_tokens: Option<&[T]>,
        standalone_custom_property_valid: bool,
    ) -> Option<NodeId> {
        if !self.peek_custom_property() {
            return None;
        }
        let node = self.create(Kind::CustomPropertyDeclaration);
        let property = self.parse_property();
        if !self.ast.set_node(node, Field::Property, property, None) {
            return None;
        }

        if !self.accept(T::Colon) {
            if standalone_custom_property_valid {
                return Some(self.finish(node));
            }
            return Some(self.finish_resync(node, &COLON_EXPECTED, &[T::Colon], &[]));
        }
        if let Some(prev) = &self.prev_token {
            let offset = prev.offset as u32;
            self.ast.extra_mut(node).colon_position = Some(offset);
        }

        let mark = self.mark();
        if self.peek(T::CurlyL) {
            let property_set = self.create(Kind::CustomPropertySet);
            let declarations = self.parse_declarations(|p| p.parse_rule_set_declaration());
            if self
                .ast
                .set_node(property_set, Field::Declarations, declarations, None)
                && !self.ast.is_erroneous(declarations.unwrap(), true)
            {
                let prio = self.parse_prio();
                self.ast.add_child(property_set, prio);
                if self.peek(T::SemiColon) {
                    self.finish(property_set);
                    self.ast
                        .set_node(node, Field::PropertySet, Some(property_set), None);
                    let offset = self.token.offset as u32;
                    self.ast.extra_mut(node).semicolon_position = Some(offset);
                    return Some(self.finish(node));
                }
            }
            self.restore_at_mark(mark);
        }

        let expression = self.parse_expr(false);
        if let Some(expression) = expression
            && !self.ast.is_erroneous(expression, true)
        {
            self.parse_prio();
            let stop = stop_tokens.unwrap_or_default();
            if self.peek_one(stop) || self.peek(T::SemiColon) || self.peek(T::Eof) {
                self.ast
                    .set_node(node, Field::Value, Some(expression), None);
                if self.peek(T::SemiColon) {
                    let offset = self.token.offset as u32;
                    self.ast.extra_mut(node).semicolon_position = Some(offset);
                }
                return Some(self.finish(node));
            }
        }
        self.restore_at_mark(mark);
        let value = self.parse_custom_property_value(stop_tokens.unwrap_or(&[T::CurlyR]));
        self.ast.add_child(node, Some(value));
        let prio = self.parse_prio();
        self.ast.add_child(node, prio);
        let colon = self.ast.colon_position(node);
        if colon.is_some_and(|c| self.token.offset as isize == c + 1) {
            return Some(self.finish_error(node, &PROPERTY_VALUE_EXPECTED));
        }
        Some(self.finish(node))
    }

    /// Custom property values: almost any sequence of tokens, up to an
    /// unbalanced delimiter, a bad token, or `;` or `!` at the top level.
    pub(crate) fn parse_custom_property_value(&mut self, stop_tokens: &[T]) -> NodeId {
        let node = self.create(Kind::Node);
        let mut curly_depth = 0;
        let mut parens_depth = 0;
        let mut brackets_depth = 0;
        loop {
            let is_top_level = curly_depth == 0 && parens_depth == 0 && brackets_depth == 0;
            match self.tok() {
                T::SemiColon | T::Exclamation if is_top_level => break,
                T::CurlyL => curly_depth += 1,
                T::CurlyR => {
                    curly_depth -= 1;
                    if curly_depth < 0 {
                        // The last declaration of the rule set, without a semicolon.
                        if stop_tokens.contains(&self.tok())
                            && parens_depth == 0
                            && brackets_depth == 0
                        {
                            break;
                        }
                        return self.finish_error(node, &LEFT_CURLY_EXPECTED);
                    }
                }
                T::ParenthesisL => parens_depth += 1,
                T::ParenthesisR => {
                    parens_depth -= 1;
                    if parens_depth < 0 {
                        if stop_tokens.contains(&self.tok())
                            && brackets_depth == 0
                            && curly_depth == 0
                        {
                            break;
                        }
                        return self.finish_error(node, &LEFT_PARENTHESIS_EXPECTED);
                    }
                }
                T::BracketL => brackets_depth += 1,
                T::BracketR => {
                    brackets_depth -= 1;
                    if brackets_depth < 0 {
                        return self.finish_error(node, &LEFT_SQUARE_BRACKET_EXPECTED);
                    }
                }
                T::BadString => break,
                T::Eof => {
                    let error = if brackets_depth > 0 {
                        &RIGHT_SQUARE_BRACKET_EXPECTED
                    } else if parens_depth > 0 {
                        &RIGHT_PARENTHESIS_EXPECTED
                    } else {
                        &RIGHT_CURLY_EXPECTED
                    };
                    return self.finish_error(node, error);
                }
                _ => {}
            }
            self.consume_token();
        }
        self.finish(node)
    }

    pub(crate) fn try_to_parse_declaration(&mut self, stop_tokens: Option<&[T]>) -> Option<NodeId> {
        let mark = self.mark();
        if self.parse_property().is_some() && self.accept(T::Colon) {
            self.restore_and_discard(mark);
            return self.parse_declaration(stop_tokens, false);
        }
        self.restore_and_discard(mark);
        None
    }

    pub(crate) fn parse_property(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Property);

        let mark = self.mark();
        // IE 5.x, 6 and 7 star hack.
        if (self.accept_delim("*") || self.accept_delim("_")) && self.has_whitespace() {
            self.restore_at_mark(mark);
            return None;
        }
        let identifier = self.parse_property_identifier();
        if self.ast.set_node(node, Field::Identifier, identifier, None) {
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn parse_property_identifier(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_property_identifier(),
            _ => self.parse_ident(None),
        }
    }

    pub(crate) fn parse_charset(&mut self) -> Option<NodeId> {
        if !self.peek(T::Charset) {
            return None;
        }
        let node = self.create(Kind::Node);
        self.consume_token();
        if !self.accept(T::String) {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        if !self.accept(T::SemiColon) {
            return Some(self.finish_error(node, &SEMI_COLON_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_import(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_import(),
            Dialect::Less => self.less_parse_import(),
            _ => self.css_parse_import(),
        }
    }

    pub(crate) fn css_parse_import(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@import") {
            return None;
        }
        let node = self.create(Kind::Import);
        self.consume_token();

        let uri = self.parse_uri_literal();
        if !self.ast.add_child(node, uri) {
            let string = self.parse_string_literal();
            if !self.ast.add_child(node, string) {
                return Some(self.finish_error(node, &URI_OR_STRING_EXPECTED));
            }
        }
        self.complete_parse_import(node)
    }

    pub(crate) fn complete_parse_import(&mut self, node: NodeId) -> Option<NodeId> {
        if self.accept_ident("layer") && self.accept(T::ParenthesisL) {
            let name = self.parse_layer_name();
            if !self.ast.add_child(node, name) {
                return Some(self.finish_resync(node, &IDENTIFIER_EXPECTED, &[T::SemiColon], &[]));
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(
                    node,
                    &RIGHT_PARENTHESIS_EXPECTED,
                    &[T::ParenthesisR],
                    &[],
                ));
            }
        }
        if self.accept_ident("supports") && self.accept(T::ParenthesisL) {
            let condition = self
                .try_to_parse_declaration(None)
                .or_else(|| Some(self.parse_supports_condition()));
            self.ast.add_child(node, condition);
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(
                    node,
                    &RIGHT_PARENTHESIS_EXPECTED,
                    &[T::ParenthesisR],
                    &[],
                ));
            }
        }

        if !self.peek(T::SemiColon) && !self.peek(T::Eof) {
            // `setMedialist` attaches without `addChild`.
            let list = self.parse_media_query_list();
            self.ast.adopt_child(node, list, None);
        }

        Some(self.finish(node))
    }

    pub(crate) fn parse_namespace(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@namespace") {
            return None;
        }
        let node = self.create(Kind::Namespace);
        self.consume_token();

        let uri = self.parse_uri_literal();
        if !self.ast.add_child(node, uri) {
            let prefix = self.parse_ident(None);
            self.ast.add_child(node, prefix);

            let uri = self.parse_uri_literal();
            if !self.ast.add_child(node, uri) {
                let string = self.parse_string_literal();
                if !self.ast.add_child(node, string) {
                    return Some(self.finish_resync(node, &URI_EXPECTED, &[T::SemiColon], &[]));
                }
            }
        }

        if !self.accept(T::SemiColon) {
            return Some(self.finish_error(node, &SEMI_COLON_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_font_face(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@font-face") {
            return None;
        }
        let node = self.create(Kind::FontFace);
        self.consume_token();
        Some(self.parse_body(node, |p| p.parse_rule_set_declaration()))
    }

    pub(crate) fn parse_view_port(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@-ms-viewport")
            && !self.peek_keyword("@-o-viewport")
            && !self.peek_keyword("@viewport")
        {
            return None;
        }
        let node = self.create(Kind::ViewPort);
        self.consume_token();
        Some(self.parse_body(node, |p| p.parse_rule_set_declaration()))
    }

    pub(crate) fn parse_keyframe(&mut self) -> Option<NodeId> {
        if !(self.peek(T::AtKeyword) && is_keyframes_keyword(&self.text())) {
            return None;
        }
        let node = self.create(Kind::Keyframe);

        let at_node = self.create(Kind::Node);
        self.consume_token();
        let at_node = self.finish(at_node);
        self.ast
            .set_node(node, Field::Keyword, Some(at_node), Some(0));
        if self.ast.matches(at_node, "@-ms-keyframes") {
            self.mark_error(at_node, &UNKNOWN_KEYWORD);
        }

        let identifier = self.parse_keyframe_ident();
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
        {
            return Some(self.finish_resync(node, &IDENTIFIER_EXPECTED, &[T::CurlyR], &[]));
        }

        Some(self.parse_body(node, |p| p.parse_keyframe_selector()))
    }

    pub(crate) fn parse_keyframe_ident(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_keyframe_ident(),
            _ => self.parse_ident(Some(&[ReferenceType::Keyframe])),
        }
    }

    pub(crate) fn parse_keyframe_selector(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_keyframe_selector(),
            Dialect::Less => self.less_parse_keyframe_selector(),
            _ => self.css_parse_keyframe_selector(),
        }
    }

    pub(crate) fn css_parse_keyframe_selector(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::KeyframeSelector);

        let ident = self.parse_ident(None);
        let mut has_content = self.ast.add_child(node, ident);
        if self.accept(T::Percentage) {
            has_content = true;
        }
        if !has_content {
            return None;
        }

        while self.accept(T::Comma) {
            let ident = self.parse_ident(None);
            has_content = self.ast.add_child(node, ident);
            if self.accept(T::Percentage) {
                has_content = true;
            }
            if !has_content {
                return Some(self.finish_error(node, &PERCENTAGE_EXPECTED));
            }
        }

        Some(self.parse_body(node, |p| p.parse_rule_set_declaration()))
    }

    pub(crate) fn try_parse_keyframe_selector(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::KeyframeSelector);
        let pos = self.mark();

        let ident = self.parse_ident(None);
        let mut has_content = self.ast.add_child(node, ident);
        if self.accept(T::Percentage) {
            has_content = true;
        }
        if !has_content {
            return None;
        }

        while self.accept(T::Comma) {
            let ident = self.parse_ident(None);
            has_content = self.ast.add_child(node, ident);
            if self.accept(T::Percentage) {
                has_content = true;
            }
            if !has_content {
                self.restore_at_mark(pos);
                return None;
            }
        }

        if !self.peek(T::CurlyL) {
            self.restore_at_mark(pos);
            return None;
        }

        Some(self.parse_body(node, |p| p.parse_rule_set_declaration()))
    }

    pub(crate) fn parse_property_at_rule(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@property") {
            return None;
        }
        let node = self.create(Kind::PropertyAtRule);
        self.consume_token();

        if !self.peek_custom_property() {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        let name = self.parse_ident(Some(&[ReferenceType::Property]));
        // `setName` attaches without `addChild`.
        let Some(name) = name else {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        };
        self.ast.set_node(node, Field::Name, Some(name), None);

        Some(self.parse_body(node, |p| p.parse_declaration(None, false)))
    }

    pub(crate) fn parse_starting_style_at_rule(&mut self, is_nested: bool) -> Option<NodeId> {
        if !self.peek_keyword("@starting-style") {
            return None;
        }
        let node = self.create(Kind::StartingStyleAtRule);
        self.consume_token();
        Some(self.parse_body(node, |p| p.parse_nested_body_declaration(is_nested)))
    }

    /// `_parseStartingStyleDeclaration`, `_parseLayerDeclaration`,
    /// `_parseSupportsDeclaration`, `_parseContainerDeclaration` and the CSS
    /// `_parseMediaDeclaration` are the same.
    pub(crate) fn parse_nested_body_declaration(&mut self, is_nested: bool) -> Option<NodeId> {
        if is_nested {
            // A nested body can have rule sets and declarations.
            return self
                .try_parse_ruleset(true)
                .or_else(|| self.try_to_parse_declaration(None))
                .or_else(|| self.parse_stylesheet_statement(true));
        }
        self.parse_stylesheet_statement(false)
    }

    pub(crate) fn parse_layer(&mut self, is_nested: bool) -> Option<NodeId> {
        if !self.peek_keyword("@layer") {
            return None;
        }
        let node = self.create(Kind::Layer);
        self.consume_token();

        let names = self.parse_layer_name_list();
        if names.is_some() {
            self.ast.set_node(node, Field::Names, names, None);
        }
        if names.is_none_or(|n| self.ast.only_child(n).is_some()) && self.peek(T::CurlyL) {
            return Some(self.parse_body(node, |p| p.parse_nested_body_declaration(is_nested)));
        }
        if !self.accept(T::SemiColon) {
            return Some(self.finish_error(node, &SEMI_COLON_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_layer_name_list(&mut self) -> Option<NodeId> {
        let node = self.create_node(NodeType::LayerNameList);
        let name = self.parse_layer_name();
        if !self.ast.add_child(node, name) {
            return None;
        }
        while self.accept(T::Comma) {
            let name = self.parse_layer_name();
            if !self.ast.add_child(node, name) {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_layer_name(&mut self) -> Option<NodeId> {
        let node = self.create_node(NodeType::LayerName);
        let ident = self.parse_ident(None);
        if !self.ast.add_child(node, ident) {
            return None;
        }
        while !self.has_whitespace() && self.accept_delim(".") {
            if self.has_whitespace() {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
            let ident = self.parse_ident(None);
            if !self.ast.add_child(node, ident) {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_supports(&mut self, is_nested: bool) -> Option<NodeId> {
        if !self.peek_keyword("@supports") {
            return None;
        }
        let node = self.create(Kind::Supports);
        self.consume_token();
        let condition = self.parse_supports_condition();
        self.ast.add_child(node, Some(condition));
        Some(self.parse_body(node, |p| p.parse_nested_body_declaration(is_nested)))
    }

    pub(crate) fn parse_supports_condition(&mut self) -> NodeId {
        let node = self.create(Kind::SupportsCondition);

        if self.accept_ident("not") {
            let c = self.parse_supports_condition_in_parens();
            self.ast.add_child(node, Some(c));
        } else {
            let c = self.parse_supports_condition_in_parens();
            self.ast.add_child(node, Some(c));
            if self.peek(T::Ident)
                && (self.text().eq_ignore_ascii_case("and")
                    || self.text().eq_ignore_ascii_case("or"))
            {
                let text = self.text().to_ascii_lowercase();
                while self.accept_ident(&text) {
                    let c = self.parse_supports_condition_in_parens();
                    self.ast.add_child(node, Some(c));
                }
            }
        }
        self.finish(node)
    }

    pub(crate) fn parse_supports_condition_in_parens(&mut self) -> NodeId {
        match self.dialect {
            Dialect::Scss => self.scss_parse_supports_condition_in_parens(),
            _ => self.css_parse_supports_condition_in_parens(),
        }
    }

    pub(crate) fn css_parse_supports_condition_in_parens(&mut self) -> NodeId {
        let node = self.create(Kind::SupportsCondition);
        if self.accept(T::ParenthesisL) {
            if let Some(prev) = &self.prev_token {
                let offset = prev.offset as u32;
                self.ast.extra_mut(node).l_parent = Some(offset);
            }
            let declaration = self.try_to_parse_declaration(Some(&[T::ParenthesisR]));
            if !self.ast.add_child(node, declaration) {
                // Upstream drops this condition node; it is never `null`.
                self.parse_supports_condition();
            }
            if !self.accept(T::ParenthesisR) {
                return self.finish_resync(
                    node,
                    &RIGHT_PARENTHESIS_EXPECTED,
                    &[T::ParenthesisR],
                    &[],
                );
            }
            if let Some(prev) = &self.prev_token {
                let offset = prev.offset as u32;
                self.ast.extra_mut(node).r_parent = Some(offset);
            }
            return self.finish(node);
        } else if self.peek(T::Ident) {
            let pos = self.mark();
            self.consume_token();
            if !self.has_whitespace() && self.accept(T::ParenthesisL) {
                let mut open_parent_count = 1;
                while self.tok() != T::Eof && open_parent_count != 0 {
                    if self.tok() == T::ParenthesisL {
                        open_parent_count += 1;
                    } else if self.tok() == T::ParenthesisR {
                        open_parent_count -= 1;
                    }
                    self.consume_token();
                }
                return self.finish(node);
            }
            self.restore_at_mark(pos);
        }
        self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], &[T::ParenthesisL])
    }

    pub(crate) fn parse_media_declaration(&mut self, is_nested: bool) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_media_declaration(is_nested),
            _ => self.parse_nested_body_declaration(is_nested),
        }
    }

    pub(crate) fn parse_media(&mut self, is_nested: bool) -> Option<NodeId> {
        if !self.peek_keyword("@media") {
            return None;
        }
        let node = self.create(Kind::Media);
        self.consume_token();

        let list = self.parse_media_query_list();
        if !self.ast.add_child(node, Some(list)) {
            return Some(self.finish_error(node, &MEDIA_QUERY_EXPECTED));
        }
        Some(self.parse_body(node, |p| p.parse_media_declaration(is_nested)))
    }

    pub(crate) fn parse_media_query_list(&mut self) -> NodeId {
        let node = self.create(Kind::Medialist);
        let query = self.parse_media_query();
        if !self.ast.add_child(node, query) {
            return self.finish_error(node, &MEDIA_QUERY_EXPECTED);
        }
        while self.accept(T::Comma) {
            let query = self.parse_media_query();
            if !self.ast.add_child(node, query) {
                return self.finish_error(node, &MEDIA_QUERY_EXPECTED);
            }
        }
        self.finish(node)
    }

    pub(crate) fn parse_media_query(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_media_query(),
            _ => self.css_parse_media_query(),
        }
    }

    pub(crate) fn css_parse_media_query(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::MediaQuery);
        let pos = self.mark();
        self.accept_ident("not");
        if !self.peek(T::ParenthesisL) {
            self.accept_ident("only");
            let ident = self.parse_ident(None);
            if !self.ast.add_child(node, ident) {
                return None;
            }
            if self.accept_ident("and") {
                let condition = self.parse_media_condition();
                self.ast.add_child(node, condition);
            }
        } else {
            // `not` is part of the media condition.
            self.restore_at_mark(pos);
            let condition = self.parse_media_condition();
            self.ast.add_child(node, condition);
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_ratio(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.create(Kind::RatioValue);
        self.parse_numeric()?;
        if !self.accept_delim("/") {
            self.restore_at_mark(pos);
            return None;
        }
        if self.parse_numeric().is_none() {
            return Some(self.finish_error(node, &NUMBER_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_boolean_expression(
        &mut self,
        parse_test: fn(&mut Self) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let node = self.create(Kind::Node);

        if self.accept_ident("not") {
            let group = self.parse_boolean_expression_group(parse_test);
            if !self.ast.add_child(node, group) {
                return None;
            }
        } else {
            let group = self.parse_boolean_expression_group(parse_test);
            if !self.ast.add_child(node, group) {
                return None;
            }
            for op in ["and", "or"] {
                if self.peek_ident(op) {
                    while self.accept_ident(op) {
                        let group = self.parse_boolean_expression_group(parse_test);
                        if !self.ast.add_child(node, group) {
                            return None;
                        }
                    }
                    break;
                }
            }
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_boolean_expression_group(
        &mut self,
        parse_test: fn(&mut Self) -> Option<NodeId>,
    ) -> Option<NodeId> {
        let node = self.create(Kind::Node);
        let pos = self.mark();

        if self.accept(T::ParenthesisL) {
            let expression = self.parse_boolean_expression(parse_test);
            if self.ast.add_child(node, expression) {
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
            self.restore_at_mark(pos);
        }

        let test = parse_test(self);
        if !self.ast.add_child(node, test) {
            return None;
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_media_condition(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_media_condition(),
            _ => Some(self.css_parse_media_condition()),
        }
    }

    pub(crate) fn css_parse_media_condition(&mut self) -> NodeId {
        let node = self.create(Kind::MediaCondition);

        self.accept_ident("not");
        let mut parse_expression = true;

        while parse_expression {
            if !self.accept(T::ParenthesisL) {
                return self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], &[T::CurlyL]);
            }
            if self.peek(T::ParenthesisL) || self.peek_ident("not") {
                let condition = self.parse_media_condition();
                self.ast.add_child(node, condition);
            } else {
                let feature = self.parse_media_feature();
                self.ast.add_child(node, Some(feature));
            }
            // General enclosed is not implemented upstream.
            if !self.accept(T::ParenthesisR) {
                return self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], &[T::CurlyL]);
            }
            parse_expression = self.accept_ident("and") || self.accept_ident("or");
        }
        self.finish(node)
    }

    pub(crate) fn parse_media_feature(&mut self) -> NodeId {
        const STOP: &[T] = &[T::ParenthesisR];

        let node = self.create(Kind::MediaFeature);

        let name = self.parse_media_feature_name();
        if self.ast.add_child(node, name) {
            if self.accept(T::Colon) {
                let value = self.parse_media_feature_value();
                if !self.ast.add_child(node, value) {
                    return self.finish_resync(node, &TERM_EXPECTED, &[], STOP);
                }
            } else if self.parse_media_feature_range_operator() {
                let value = self.parse_media_feature_value();
                if !self.ast.add_child(node, value) {
                    return self.finish_resync(node, &TERM_EXPECTED, &[], STOP);
                }
                if self.parse_media_feature_range_operator() {
                    let value = self.parse_media_feature_value();
                    if !self.ast.add_child(node, value) {
                        return self.finish_resync(node, &TERM_EXPECTED, &[], STOP);
                    }
                }
            }
        } else {
            let value = self.parse_media_feature_value();
            if self.ast.add_child(node, value) {
                if !self.parse_media_feature_range_operator() {
                    return self.finish_resync(node, &OPERATOR_EXPECTED, &[], STOP);
                }
                let name = self.parse_media_feature_name();
                if !self.ast.add_child(node, name) {
                    return self.finish_resync(node, &IDENTIFIER_EXPECTED, &[], STOP);
                }
                if self.parse_media_feature_range_operator() {
                    let value = self.parse_media_feature_value();
                    if !self.ast.add_child(node, value) {
                        return self.finish_resync(node, &TERM_EXPECTED, &[], STOP);
                    }
                }
            } else {
                return self.finish_resync(node, &IDENTIFIER_EXPECTED, &[], STOP);
            }
        }
        self.finish(node)
    }

    pub(crate) fn parse_media_feature_range_operator(&mut self) -> bool {
        match self.dialect {
            Dialect::Scss => self.scss_parse_media_feature_range_operator(),
            _ => self.css_parse_media_feature_range_operator(),
        }
    }

    pub(crate) fn css_parse_media_feature_range_operator(&mut self) -> bool {
        if self.accept_delim("<") || self.accept_delim(">") {
            if !self.has_whitespace() {
                self.accept_delim("=");
            }
            return true;
        }
        self.accept_delim("=")
    }

    pub(crate) fn parse_media_feature_name(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_media_feature_name(),
            Dialect::Less => self.less_parse_media_feature_name(),
            _ => self.parse_ident(None),
        }
    }

    pub(crate) fn parse_media_feature_value(&mut self) -> Option<NodeId> {
        self.parse_ratio().or_else(|| self.parse_term_expression())
    }

    pub(crate) fn parse_scope(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@scope") {
            return None;
        }
        let node = self.create(Kind::Scope);
        self.consume_token();

        let limits = self.parse_scope_limits();
        self.ast.add_child(node, Some(limits));

        Some(self.parse_body(node, |p| p.parse_scope_declaration()))
    }

    pub(crate) fn parse_scope_declaration(&mut self) -> Option<NodeId> {
        // Declarations are implicitly wrapped in `:where(:scope)`, so the body
        // is nested.
        self.try_parse_ruleset(true)
            .or_else(|| self.try_to_parse_declaration(None))
            .or_else(|| self.parse_stylesheet_statement(true))
    }

    pub(crate) fn parse_scope_limits(&mut self) -> NodeId {
        let node = self.create(Kind::ScopeLimits);

        if self.accept(T::ParenthesisL) {
            let start = self.parse_scope_selector_list();
            if !self.ast.set_node(node, Field::ScopeStart, start, None) {
                return self.finish_resync(node, &SELECTOR_EXPECTED, &[], &[T::ParenthesisR]);
            }
            if !self.accept(T::ParenthesisR) {
                return self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], &[T::CurlyL]);
            }
        }

        if self.accept_ident("to") {
            if !self.accept(T::ParenthesisL) {
                return self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], &[T::CurlyL]);
            }
            let end = self.parse_scope_selector_list();
            if !self.ast.set_node(node, Field::ScopeEnd, end, None) {
                return self.finish_resync(node, &SELECTOR_EXPECTED, &[], &[T::ParenthesisR]);
            }
            if !self.accept(T::ParenthesisR) {
                return self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], &[T::CurlyL]);
            }
        }

        self.finish(node)
    }

    fn parse_scope_selector_list(&mut self) -> Option<NodeId> {
        let selectors = self.create_node(NodeType::SelectorList);
        let selector = self.parse_selector(true);
        if !self.ast.add_child(selectors, selector) {
            return None;
        }
        while self.accept(T::Comma) {
            let selector = self.parse_selector(true);
            if !self.ast.add_child(selectors, selector) {
                break;
            }
        }
        Some(self.finish(selectors))
    }

    pub(crate) fn parse_medium(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Node);
        let ident = self.parse_ident(None);
        if self.ast.add_child(node, ident) {
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn parse_page_declaration(&mut self) -> Option<NodeId> {
        self.parse_page_margin_box()
            .or_else(|| self.parse_rule_set_declaration())
    }

    pub(crate) fn parse_page(&mut self) -> Option<NodeId> {
        if !self.peek_keyword("@page") {
            return None;
        }
        let node = self.create(Kind::Page);
        self.consume_token();

        let selector = self.parse_page_selector();
        if self.ast.add_child(node, selector) {
            while self.accept(T::Comma) {
                let selector = self.parse_page_selector();
                if !self.ast.add_child(node, selector) {
                    return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
                }
            }
        }

        Some(self.parse_body(node, |p| p.parse_page_declaration()))
    }

    pub(crate) fn parse_page_margin_box(&mut self) -> Option<NodeId> {
        if !self.peek(T::AtKeyword) {
            return None;
        }
        let node = self.create(Kind::PageBoxMarginBox);
        if !self.accept_one_keyword(PAGE_BOX_DIRECTIVES) {
            self.mark_error_with(node, &UNKNOWN_AT_RULE, Some((&[], &[T::CurlyL])));
        }
        Some(self.parse_body(node, |p| p.parse_rule_set_declaration()))
    }

    pub(crate) fn parse_page_selector(&mut self) -> Option<NodeId> {
        if !self.peek(T::Ident) && !self.peek(T::Colon) {
            return None;
        }
        let node = self.create(Kind::Node);
        let ident = self.parse_ident(None);
        self.ast.add_child(node, ident);

        if self.accept(T::Colon) {
            let ident = self.parse_ident(None);
            if !self.ast.add_child(node, ident) {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_document(&mut self) -> Option<NodeId> {
        // `-moz-document` is experimental.
        if !self.peek_keyword("@-moz-document") {
            return None;
        }
        let node = self.create(Kind::Document);
        self.consume_token();

        self.resync(&[], &[T::CurlyL]);
        Some(self.parse_body(node, |p| p.parse_stylesheet_statement(false)))
    }

    pub(crate) fn parse_container(&mut self, is_nested: bool) -> Option<NodeId> {
        if !self.peek_keyword("@container") {
            return None;
        }
        let node = self.create(Kind::Container);
        self.consume_token();

        let name = self.parse_container_name();
        self.ast.add_child(node, name);
        let query = self.parse_container_query();
        if self.ast.add_child(node, Some(query)) {
            while self.accept(T::Comma) {
                if self.peek(T::CurlyL) {
                    break;
                }
                let name = self.parse_container_name();
                self.ast.add_child(node, name);
                let query = self.parse_container_query();
                self.ast.add_child(node, Some(query));
            }
        }

        Some(self.parse_body(node, |p| p.parse_nested_body_declaration(is_nested)))
    }

    pub(crate) fn parse_container_name(&mut self) -> Option<NodeId> {
        if self.peek_ident("style") || self.peek_ident("scroll-state") {
            let mark = self.mark();
            self.consume_token();
            if !self.has_whitespace() && self.peek(T::ParenthesisL) {
                self.restore_at_mark(mark);
                return None;
            }
            self.restore_at_mark(mark);
        }
        self.parse_ident(None)
    }

    pub(crate) fn parse_container_query(&mut self) -> NodeId {
        let node = self.create(Kind::Node);
        if self.accept_ident("not") {
            let q = self.parse_container_query_in_parens(false);
            self.ast.add_child(node, q);
        } else {
            let q = self.parse_container_query_in_parens(true);
            self.ast.add_child(node, q);
            for op in ["and", "or"] {
                if self.peek_ident(op) {
                    while self.accept_ident(op) {
                        let q = self.parse_container_query_in_parens(false);
                        self.ast.add_child(node, q);
                    }
                    break;
                }
            }
        }
        self.finish(node)
    }

    pub(crate) fn parse_container_query_in_parens(&mut self, optional: bool) -> Option<NodeId> {
        const STOP: &[T] = &[T::CurlyL];
        let node = self.create(Kind::Node);
        if self.accept(T::ParenthesisL) {
            if self.peek_ident("not") || self.peek(T::ParenthesisL) {
                let q = self.parse_container_query();
                self.ast.add_child(node, Some(q));
            } else {
                let f = self.parse_media_feature();
                self.ast.add_child(node, Some(f));
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], STOP));
            }
        } else if self.accept_ident("style") {
            if self.has_whitespace() || !self.accept(T::ParenthesisL) {
                return Some(self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            let q = self.parse_style_query();
            self.ast.add_child(node, Some(q));
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], STOP));
            }
        } else if self.accept_ident("scroll-state") {
            if self.has_whitespace() || !self.accept(T::ParenthesisL) {
                return Some(self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            let d = self.parse_declaration(Some(&[T::ParenthesisR]), true);
            self.ast.add_child(node, d);
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], STOP));
            }
        } else {
            if optional {
                return None;
            }
            return Some(self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], STOP));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_style_query(&mut self) -> NodeId {
        let node = self.create(Kind::Node);

        if self.accept_ident("not") {
            let s = self.parse_style_in_parens();
            self.ast.add_child(node, Some(s));
        } else if self.peek(T::ParenthesisL) {
            let s = self.parse_style_in_parens();
            self.ast.add_child(node, Some(s));
            for op in ["and", "or"] {
                if self.peek_ident(op) {
                    while self.accept_ident(op) {
                        let s = self.parse_style_in_parens();
                        self.ast.add_child(node, Some(s));
                    }
                    break;
                }
            }
        } else {
            let d = self.parse_declaration(Some(&[T::ParenthesisR]), true);
            self.ast.add_child(node, d);
        }
        self.finish(node)
    }

    pub(crate) fn parse_style_in_parens(&mut self) -> NodeId {
        let node = self.create(Kind::Node);
        if self.accept(T::ParenthesisL) {
            let q = self.parse_style_query();
            self.ast.add_child(node, Some(q));
            if !self.accept(T::ParenthesisR) {
                return self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], &[T::CurlyL]);
            }
        } else {
            return self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], &[T::CurlyL]);
        }
        self.finish(node)
    }

    pub(crate) fn parse_unknown_at_rule(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.css_parse_unknown_at_rule(Some(T::InterpolationFunction)),
            _ => self.css_parse_unknown_at_rule(None),
        }
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-an-at-rule>
    pub(crate) fn css_parse_unknown_at_rule(
        &mut self,
        interpolation_token: Option<T>,
    ) -> Option<NodeId> {
        if !self.peek(T::AtKeyword) {
            return None;
        }
        let node = self.create(Kind::UnknownAtRule);
        let name = self.parse_unknown_at_rule_name();
        self.ast.add_child(node, Some(name));

        let mut curly_l_count = 0;
        let mut curly_depth = 0;
        let mut parens_depth = 0;
        let mut brackets_depth = 0;
        loop {
            if Some(self.tok()) == interpolation_token {
                // An interpolation balances a brace but does not start the block.
                curly_depth += 1;
                self.consume_token();
                continue;
            }
            let is_top_level = curly_depth == 0 && parens_depth == 0 && brackets_depth == 0;
            match self.tok() {
                T::SemiColon if is_top_level => break,
                T::Eof => {
                    return Some(if curly_depth > 0 {
                        self.finish_error(node, &RIGHT_CURLY_EXPECTED)
                    } else if brackets_depth > 0 {
                        self.finish_error(node, &RIGHT_SQUARE_BRACKET_EXPECTED)
                    } else if parens_depth > 0 {
                        self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED)
                    } else {
                        self.finish(node)
                    });
                }
                T::CurlyL => {
                    curly_l_count += 1;
                    curly_depth += 1;
                }
                T::CurlyR => {
                    curly_depth -= 1;
                    // The end of the at-rule.
                    if curly_l_count > 0 && curly_depth == 0 {
                        self.consume_token();
                        if brackets_depth > 0 {
                            return Some(self.finish_error(node, &RIGHT_SQUARE_BRACKET_EXPECTED));
                        } else if parens_depth > 0 {
                            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
                        }
                        break;
                    }
                    if curly_depth < 0 {
                        // The last declaration of the rule set, without a semicolon.
                        if parens_depth == 0 && brackets_depth == 0 {
                            break;
                        }
                        return Some(self.finish_error(node, &LEFT_CURLY_EXPECTED));
                    }
                }
                T::ParenthesisL => parens_depth += 1,
                T::ParenthesisR => {
                    parens_depth -= 1;
                    if parens_depth < 0 {
                        return Some(self.finish_error(node, &LEFT_PARENTHESIS_EXPECTED));
                    }
                }
                T::BracketL => brackets_depth += 1,
                T::BracketR => {
                    brackets_depth -= 1;
                    if brackets_depth < 0 {
                        return Some(self.finish_error(node, &LEFT_SQUARE_BRACKET_EXPECTED));
                    }
                }
                _ => {}
            }
            self.consume_token();
        }
        // Upstream returns without `finish` here.
        Some(node)
    }

    pub(crate) fn parse_unknown_at_rule_name(&mut self) -> NodeId {
        let node = self.create(Kind::Node);
        if self.accept(T::AtKeyword) {
            return self.finish(node);
        }
        node
    }

    pub(crate) fn parse_operator(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_operator(),
            Dialect::Less => self.less_parse_operator(),
            _ => self.css_parse_operator(),
        }
    }

    pub(crate) fn css_parse_operator(&mut self) -> Option<NodeId> {
        // Operators of binary expressions. `=` is not standard.
        if self.peek_delim("/")
            || self.peek_delim("*")
            || self.peek_delim("+")
            || self.peek_delim("-")
            || self.peek_one(&[
                T::Dashmatch,
                T::Includes,
                T::SubstringOperator,
                T::PrefixOperator,
                T::SuffixOperator,
            ])
            || self.peek_delim("=")
        {
            let node = self.create_node(NodeType::Operator);
            self.consume_token();
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn parse_unary_operator(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_unary_operator(),
            _ => self.css_parse_unary_operator(),
        }
    }

    pub(crate) fn css_parse_unary_operator(&mut self) -> Option<NodeId> {
        if !self.peek_delim("+") && !self.peek_delim("-") {
            return None;
        }
        let node = self.create(Kind::Node);
        self.consume_token();
        Some(self.finish(node))
    }

    pub(crate) fn parse_combinator(&mut self) -> Option<NodeId> {
        if self.peek_delim(">") {
            let node = self.create(Kind::Node);
            self.consume_token();
            let mark = self.mark();
            if !self.has_whitespace() && self.accept_delim(">") {
                if !self.has_whitespace() && self.accept_delim(">") {
                    self.ast
                        .set_type(node, NodeType::SelectorCombinatorShadowPiercingDescendant);
                    return Some(self.finish(node));
                }
                self.restore_at_mark(mark);
            }
            self.ast.set_type(node, NodeType::SelectorCombinatorParent);
            return Some(self.finish(node));
        } else if self.peek_delim("+") {
            let node = self.create(Kind::Node);
            self.consume_token();
            self.ast.set_type(node, NodeType::SelectorCombinatorSibling);
            return Some(self.finish(node));
        } else if self.peek_delim("~") {
            let node = self.create(Kind::Node);
            self.consume_token();
            self.ast
                .set_type(node, NodeType::SelectorCombinatorAllSiblings);
            return Some(self.finish(node));
        } else if self.peek_delim("/") {
            let node = self.create(Kind::Node);
            self.consume_token();
            let mark = self.mark();
            if !self.has_whitespace()
                && self.accept_ident("deep")
                && !self.has_whitespace()
                && self.accept_delim("/")
            {
                self.ast
                    .set_type(node, NodeType::SelectorCombinatorShadowPiercingDescendant);
                return Some(self.finish(node));
            }
            self.restore_at_mark(mark);
        }
        None
    }

    pub(crate) fn parse_simple_selector(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::SimpleSelector);
        let mut c = 0;
        let first = self
            .parse_element_name()
            .or_else(|| self.parse_nesting_selector());
        if self.ast.add_child(node, first) {
            c += 1;
        }
        while c == 0 || !self.has_whitespace() {
            let body = self.parse_simple_selector_body();
            if !self.ast.add_child(node, body) {
                break;
            }
            c += 1;
        }
        if c == 0 {
            return self.discard(node);
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_nesting_selector(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_nesting_selector(),
            Dialect::Less => self.less_parse_nesting_selector(),
            _ => self.css_parse_nesting_selector(),
        }
    }

    pub(crate) fn css_parse_nesting_selector(&mut self) -> Option<NodeId> {
        if self.peek_delim("&") {
            let node = self.create_node(NodeType::SelectorCombinator);
            self.consume_token();
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn parse_simple_selector_body(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_simple_selector_body(),
            _ => self.css_parse_simple_selector_body(),
        }
    }

    pub(crate) fn css_parse_simple_selector_body(&mut self) -> Option<NodeId> {
        self.parse_pseudo()
            .or_else(|| self.parse_hash())
            .or_else(|| self.parse_class())
            .or_else(|| self.parse_attrib())
    }

    pub(crate) fn parse_selector_ident(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_selector_ident(),
            _ => self.parse_ident(None),
        }
    }

    pub(crate) fn parse_hash(&mut self) -> Option<NodeId> {
        if !self.peek(T::Hash) && !self.peek_delim("#") {
            return None;
        }
        let node = self.create_node(NodeType::IdentifierSelector);
        if self.accept_delim("#") {
            if self.has_whitespace() {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
            let ident = self.parse_selector_ident();
            if !self.ast.add_child(node, ident) {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
        } else {
            self.consume_token();
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_class(&mut self) -> Option<NodeId> {
        if !self.peek_delim(".") {
            return None;
        }
        let node = self.create_node(NodeType::ClassSelector);
        self.consume_token();

        if self.has_whitespace() {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        let ident = self.parse_selector_ident();
        if !self.ast.add_child(node, ident) {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_element_name(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_element_name(),
            _ => self.css_parse_element_name(),
        }
    }

    pub(crate) fn css_parse_element_name(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.create_node(NodeType::ElementNameSelector);
        let prefix = self.parse_namespace_prefix();
        self.ast.add_child(node, prefix);
        let ident = self.parse_selector_ident();
        if !self.ast.add_child(node, ident) && !self.accept_delim("*") {
            self.restore_and_discard(pos);
            return None;
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_namespace_prefix(&mut self) -> Option<NodeId> {
        if !self.may_be_namespace_prefix() {
            return None;
        }
        let pos = self.mark();
        let node = self.create_node(NodeType::NamespacePrefix);
        let ident = self.parse_ident(None);
        if !self.ast.add_child(node, ident) {
            self.accept_delim("*");
        }
        if !self.accept_delim("|") {
            self.restore_and_discard(pos);
            return None;
        }
        Some(self.finish(node))
    }

    /// Not upstream. A prefix is `|`, or `*` or an identifier before a `|`.
    /// Outside SCSS an identifier is one token.
    fn may_be_namespace_prefix(&mut self) -> bool {
        if self.dialect == Dialect::Scss {
            return true;
        }
        if self.peek_delim("|") {
            return true;
        }
        if !self.peek(T::Ident) && !self.peek_delim("*") {
            return false;
        }
        let pos = self.scanner.pos();
        let next = self.scanner.scan();
        self.scanner.go_back_to(pos);
        next.kind == T::Delim && self.scanner.substring(next.offset, next.len) == "|"
    }

    pub(crate) fn parse_attrib(&mut self) -> Option<NodeId> {
        if !self.peek(T::BracketL) {
            return None;
        }
        let node = self.create(Kind::AttributeSelector);
        self.consume_token();

        let prefix = self.parse_namespace_prefix();
        self.ast
            .set_node(node, Field::NamespacePrefix, prefix, None);

        let ident = self.parse_ident(None);
        if !self.ast.set_node(node, Field::Identifier, ident, None) {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        let operator = self.parse_operator();
        if self.ast.set_node(node, Field::Operator, operator, None) {
            let value = self.parse_binary_expr(None, None);
            self.ast.set_node(node, Field::Value, value, None);
            self.accept_ident("i");
            self.accept_ident("s");
        }

        if !self.accept(T::BracketR) {
            return Some(self.finish_error(node, &RIGHT_SQUARE_BRACKET_EXPECTED));
        }
        Some(self.finish(node))
    }

    fn try_as_selector(&mut self) -> Option<NodeId> {
        let selectors = self.create_node(NodeType::SelectorList);
        let selector = self.parse_selector(true);
        if !self.ast.add_child(selectors, selector) {
            return None;
        }
        while self.accept(T::Comma) {
            let selector = self.parse_selector(true);
            if !self.ast.add_child(selectors, selector) {
                break;
            }
        }
        if self.peek(T::ParenthesisR) {
            return Some(self.finish(selectors));
        }
        None
    }

    pub(crate) fn parse_pseudo(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_pseudo(),
            _ => self.css_parse_pseudo(),
        }
    }

    pub(crate) fn css_parse_pseudo(&mut self) -> Option<NodeId> {
        let node = self.try_parse_pseudo_identifier()?;
        if !self.has_whitespace() && self.accept(T::ParenthesisL) {
            let selector = self.try_parse(|p| p.try_as_selector());
            let has_selector = self.ast.add_child(node, selector);
            if !has_selector {
                // The <an+b> syntax, which is not a proper expression.
                while !self.peek_ident("of") {
                    let term = self.parse_term();
                    if self.ast.add_child(node, term) {
                        continue;
                    }
                    let operator = self.parse_operator();
                    if !self.ast.add_child(node, operator) {
                        break;
                    }
                }
                if self.accept_ident("of") {
                    let selector = self.try_parse(|p| p.try_as_selector());
                    if !self.ast.add_child(node, selector) {
                        return Some(self.finish_error(node, &SELECTOR_EXPECTED));
                    }
                }
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
            }
        }
        Some(self.finish(node))
    }

    pub(crate) fn try_parse_pseudo_identifier(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_try_parse_pseudo_identifier(),
            _ => self.css_try_parse_pseudo_identifier(),
        }
    }

    pub(crate) fn css_try_parse_pseudo_identifier(&mut self) -> Option<NodeId> {
        if !self.peek(T::Colon) {
            return None;
        }
        let pos = self.mark();
        let node = self.create_node(NodeType::PseudoSelector);
        self.consume_token();
        if self.has_whitespace() {
            self.restore_and_discard(pos);
            return None;
        }
        // `::`
        self.accept(T::Colon);
        if self.has_whitespace() {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        let ident = self.parse_ident(None);
        if !self.ast.add_child(node, ident) {
            return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn try_parse_prio(&mut self) -> Option<NodeId> {
        let mark = self.mark();
        if let Some(prio) = self.parse_prio() {
            return Some(prio);
        }
        self.restore_at_mark(mark);
        None
    }

    pub(crate) fn parse_prio(&mut self) -> Option<NodeId> {
        if !self.peek(T::Exclamation) {
            return None;
        }
        let node = self.create_node(NodeType::Prio);
        if self.accept(T::Exclamation) && self.accept_ident("important") {
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn parse_expr(&mut self, stop_on_comma: bool) -> Option<NodeId> {
        let node = self.create(Kind::Expression);
        let expr = self.parse_binary_expr(None, None);
        if !self.ast.add_child(node, expr) {
            return self.discard(node);
        }

        loop {
            if self.peek(T::Comma) {
                if stop_on_comma {
                    return Some(self.finish(node));
                }
                self.consume_token();
            }
            let expr = self.parse_binary_expr(None, None);
            if !self.ast.add_child(node, expr) {
                break;
            }
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_unicode_range(&mut self) -> Option<NodeId> {
        if !self.peek_ident("u") {
            return None;
        }
        let node = self.create(Kind::UnicodeRange);
        if !self.accept_unicode_range() {
            return None;
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_named_line(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_named_line(),
            _ => self.css_parse_named_line(),
        }
    }

    /// <https://www.w3.org/TR/css-grid-1/#named-lines>
    pub(crate) fn css_parse_named_line(&mut self) -> Option<NodeId> {
        if !self.peek(T::BracketL) {
            return None;
        }
        let node = self.create_node(NodeType::GridLine);
        self.consume_token();
        loop {
            let ident = self.parse_ident(None);
            if !self.ast.add_child(node, ident) {
                break;
            }
        }
        if !self.accept(T::BracketR) {
            return Some(self.finish_error(node, &RIGHT_SQUARE_BRACKET_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_binary_expr(
        &mut self,
        preparsed_left: Option<NodeId>,
        preparsed_oper: Option<NodeId>,
    ) -> Option<NodeId> {
        let mut node = self.create(Kind::BinaryExpression);

        // The operand token starts at the current position; the expression
        // starts at the left operand.
        if let Some(left) = preparsed_left {
            let offset = self.ast.offset(left);
            self.ast.node_mut(node).set_offset(offset);
        }

        let left = preparsed_left.or_else(|| self.parse_term());
        if !self.ast.set_node(node, Field::Left, left, None) {
            return self.discard(node);
        }

        let operator = preparsed_oper.or_else(|| self.parse_operator());
        if !self.ast.set_node(node, Field::Operator, operator, None) {
            return Some(self.finish(node));
        }

        let right = self.parse_term();
        if !self.ast.set_node(node, Field::Right, right, None) {
            return Some(self.finish_error(node, &TERM_EXPECTED));
        }

        // A chain of binary expressions.
        node = self.finish(node);
        if let Some(operator) = self.parse_operator() {
            node = self.parse_binary_expr(Some(node), Some(operator)).unwrap();
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_term(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Term);
        let operator = self.parse_unary_operator();
        self.ast.set_node(node, Field::Operator, operator, None);

        let expression = self.parse_term_expression();
        if self.ast.set_node(node, Field::Expression, expression, None) {
            return Some(self.finish(node));
        }
        self.discard(node)
    }

    pub(crate) fn parse_term_expression(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_term_expression(),
            Dialect::Less => self.less_parse_term_expression(),
            _ => self.css_parse_term_expression(),
        }
    }

    pub(crate) fn css_parse_term_expression(&mut self) -> Option<NodeId> {
        self.parse_uri_literal()
            .or_else(|| self.parse_unicode_range())
            .or_else(|| self.parse_attr_type())
            .or_else(|| self.parse_function())
            .or_else(|| self.parse_ident(None))
            .or_else(|| self.parse_string_literal())
            .or_else(|| self.parse_numeric())
            .or_else(|| self.parse_hex_color())
            .or_else(|| self.parse_operation())
            .or_else(|| self.parse_named_line())
    }

    pub(crate) fn parse_operation(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_operation(),
            _ => self.css_parse_operation(),
        }
    }

    pub(crate) fn css_parse_operation(&mut self) -> Option<NodeId> {
        if !self.peek(T::ParenthesisL) {
            return None;
        }
        let node = self.create(Kind::Node);
        self.consume_token();
        let expr = self.parse_expr(false);
        self.ast.add_child(node, expr);
        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_numeric(&mut self) -> Option<NodeId> {
        if self.peek_one(&[
            T::Num,
            T::Percentage,
            T::Resolution,
            T::Length,
            T::Ems,
            T::Exs,
            T::Angle,
            T::Time,
            T::Dimension,
            T::ContainerQueryLength,
            T::Freq,
        ]) {
            let node = self.create(Kind::NumericValue);
            self.consume_token();
            return Some(self.finish(node));
        }
        None
    }

    pub(crate) fn parse_string_literal(&mut self) -> Option<NodeId> {
        if !self.peek(T::String) && !self.peek(T::BadString) {
            return None;
        }
        let node = self.create_node(NodeType::StringLiteral);
        self.consume_token();
        Some(self.finish(node))
    }

    pub(crate) fn parse_uri_literal(&mut self) -> Option<NodeId> {
        if !(self.peek(T::Ident)
            && (self.text().eq_ignore_ascii_case("url")
                || self.text().eq_ignore_ascii_case("url-prefix")))
        {
            return None;
        }
        let pos = self.mark();
        let node = self.create_node(NodeType::URILiteral);
        self.accept(T::Ident);
        if self.has_whitespace() || !self.peek(T::ParenthesisL) {
            self.restore_and_discard(pos);
            return None;
        }
        self.scanner.in_url = true;
        self.consume_token();
        let argument = self.parse_url_argument();
        self.ast.add_child(node, argument);
        self.scanner.in_url = false;

        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_url_argument(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_url_argument(),
            Dialect::Less => self.less_parse_url_argument(),
            _ => self.css_parse_url_argument(),
        }
    }

    pub(crate) fn css_parse_url_argument(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Node);
        if !self.accept(T::String) && !self.accept(T::BadString) && !self.accept_unquoted_string() {
            return None;
        }
        Some(self.finish(node))
    }

    pub(crate) fn parse_ident(
        &mut self,
        reference_types: Option<&'static [ReferenceType]>,
    ) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_ident(reference_types),
            _ => self.css_parse_ident(reference_types),
        }
    }

    pub(crate) fn css_parse_ident(
        &mut self,
        reference_types: Option<&'static [ReferenceType]>,
    ) -> Option<NodeId> {
        if !self.peek(T::Ident) {
            return None;
        }
        let node = self.create(Kind::Identifier);
        self.ast.node_mut(node).is_custom_property = self.peek_custom_property();
        if reference_types.is_some() {
            self.ast.extra_mut(node).reference_types = reference_types;
        }
        self.consume_token();
        Some(self.finish(node))
    }

    pub(crate) fn parse_function(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_function(),
            Dialect::Less => self.less_parse_function(),
            _ => self.css_parse_function(),
        }
    }

    pub(crate) fn css_parse_function(&mut self) -> Option<NodeId> {
        let pos = self.mark();
        let node = self.create(Kind::Function);

        let is_if = self.peek_ident("if");
        let separator = if is_if { T::SemiColon } else { T::Comma };
        let is_attr = self.peek_ident("attr");

        let identifier = self.parse_function_identifier();
        if !self
            .ast
            .set_node(node, Field::Identifier, identifier, Some(0))
        {
            return self.discard(node);
        }

        if self.has_whitespace() || !self.accept(T::ParenthesisL) {
            self.restore_and_discard(pos);
            return None;
        }

        if is_attr {
            self.attr_depth += 1;
        }
        let parse_argument = |p: &mut Self| {
            if is_if {
                Some(p.parse_if_branch())
            } else {
                p.parse_function_argument()
            }
        };
        let argument = parse_argument(self);
        let arguments = self.ast.list(node, Field::Arguments);
        if self.ast.add_child(arguments, argument) {
            while self.accept(separator) {
                if self.peek(T::ParenthesisR) {
                    break;
                }
                let argument = parse_argument(self);
                let arguments = self.ast.list(node, Field::Arguments);
                if !self.ast.add_child(arguments, argument) {
                    self.mark_error(node, &EXPRESSION_EXPECTED);
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

    pub(crate) fn parse_function_identifier(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Less => self.less_parse_function_identifier(),
            _ => self.css_parse_function_identifier(),
        }
    }

    pub(crate) fn css_parse_function_identifier(&mut self) -> Option<NodeId> {
        if !self.peek(T::Ident) {
            return None;
        }
        let node = self.create(Kind::Identifier);
        self.ast.extra_mut(node).reference_types = Some(&[ReferenceType::Function]);

        if self.accept_ident("progid") {
            // IE7 filters: `progid:DXImageTransform.Microsoft.MotionBlur(strength=13)`.
            if self.accept(T::Colon) {
                while self.accept(T::Ident) && self.accept_delim(".") {}
            }
            return Some(self.finish(node));
        }
        self.consume_token();
        Some(self.finish(node))
    }

    pub(crate) fn parse_function_argument(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_function_argument(),
            _ => self.css_parse_function_argument(),
        }
    }

    pub(crate) fn css_parse_function_argument(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::FunctionArgument);
        let value = self.parse_expr(true);
        if self.ast.set_node(node, Field::Value, value, Some(0)) {
            return Some(self.finish(node));
        }
        None
    }

    /// `type(<syntax>)` inside `attr()`.
    /// <https://drafts.csswg.org/css-values-5/#attr-notation>
    pub(crate) fn parse_attr_type(&mut self) -> Option<NodeId> {
        if self.attr_depth == 0 || !self.peek_ident("type") {
            return None;
        }

        let pos = self.mark();
        let node = self.create(Kind::Function);
        let identifier = self.parse_function_identifier();
        self.ast
            .set_node(node, Field::Identifier, identifier, Some(0));

        if self.has_whitespace() || !self.accept(T::ParenthesisL) {
            self.restore_at_mark(pos);
            return None;
        }

        let syntax = self.parse_attr_type_syntax();
        let arguments = self.ast.list(node, Field::Arguments);
        if !self.ast.add_child(arguments, syntax) {
            return Some(self.finish_resync(node, &EXPRESSION_EXPECTED, &[], &[T::ParenthesisR]));
        }

        if !self.accept(T::ParenthesisR) {
            return Some(self.finish_error(node, &RIGHT_PARENTHESIS_EXPECTED));
        }
        Some(self.finish(node))
    }

    /// `<syntax> = '*' | <syntax-component> [ '|' <syntax-component> ]*`
    pub(crate) fn parse_attr_type_syntax(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Node);
        if self.accept_delim("*") {
            return Some(self.finish(node));
        }
        if !self.accept_attr_syntax_component() {
            return None;
        }
        while self.accept_delim("|") {
            if !self.accept_attr_syntax_component() {
                return Some(self.finish_error(node, &IDENTIFIER_EXPECTED));
            }
        }
        Some(self.finish(node))
    }

    /// `<syntax-component> = ( '<' <syntax-type-name> '>' | <ident> ) <syntax-multiplier>?`
    fn accept_attr_syntax_component(&mut self) -> bool {
        if self.accept_delim("<") {
            if !self.accept(T::Ident) || !self.accept_delim(">") {
                return false;
            }
        } else if !self.accept(T::Ident) {
            return false;
        }
        if !self.accept_delim("+") {
            self.accept_delim("#");
        }
        true
    }

    /// `<if-branch> = <if-condition> : <declaration-value>?`
    pub(crate) fn parse_if_branch(&mut self) -> NodeId {
        let node = self.create(Kind::Node);
        let condition = self.parse_if_condition();
        if !self.ast.add_child(node, condition) {
            return self.finish_resync(node, &IF_CONDITION_EXPECTED, &[], &[T::SemiColon]);
        }
        if !self.accept(T::Colon) {
            return self.finish_resync(node, &COLON_EXPECTED, &[], &[T::SemiColon]);
        }
        let expr = self.parse_expr(false);
        self.ast.add_child(node, expr);
        self.finish(node)
    }

    /// `<if-condition> = <boolean-expr[ <if-test> ]> | else`
    pub(crate) fn parse_if_condition(&mut self) -> Option<NodeId> {
        let node = self.create(Kind::Node);
        if self.peek_ident("else") {
            let ident = self.parse_ident(None);
            self.ast.add_child(node, ident);
            return Some(self.finish(node));
        }
        self.parse_boolean_expression(Self::parse_if_test)
    }

    pub(crate) fn parse_if_test(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_if_test(),
            _ => self.css_parse_if_test(),
        }
    }

    pub(crate) fn css_parse_if_test(&mut self) -> Option<NodeId> {
        const STOP: &[T] = &[T::Colon];
        let node = self.create(Kind::Node);

        if self.accept_ident("supports") {
            if self.has_whitespace() || !self.accept(T::ParenthesisL) {
                return Some(self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            let condition = self
                .try_to_parse_declaration(None)
                .or_else(|| Some(self.parse_supports_condition()));
            self.ast.add_child(node, condition);
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            return Some(self.finish(node));
        }

        if self.accept_ident("media") {
            if self.has_whitespace() || !self.accept(T::ParenthesisL) {
                return Some(self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            let pos = self.mark();
            let condition = self.parse_media_condition();
            if let Some(condition) = condition
                && !self.ast.is_erroneous(condition, false)
            {
                self.ast.add_child(node, Some(condition));
            } else {
                self.restore_at_mark(pos);
                let feature = self.parse_media_feature();
                self.ast.add_child(node, Some(feature));
            }
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            return Some(self.finish(node));
        }

        if self.accept_ident("style") {
            if self.has_whitespace() || !self.accept(T::ParenthesisL) {
                return Some(self.finish_resync(node, &LEFT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            let query = self.parse_style_query();
            self.ast.add_child(node, Some(query));
            if !self.accept(T::ParenthesisR) {
                return Some(self.finish_resync(node, &RIGHT_PARENTHESIS_EXPECTED, &[], STOP));
            }
            return Some(self.finish(node));
        }

        None
    }

    pub(crate) fn parse_hex_color(&mut self) -> Option<NodeId> {
        if self.peek(T::Hash) && is_hex_color(&self.text()) {
            let node = self.create(Kind::HexColorValue);
            self.consume_token();
            return Some(self.finish(node));
        }
        None
    }

    /// SCSS and Less only.
    pub(crate) fn parse_variable(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_variable(),
            Dialect::Less => self.less_parse_variable(false, false),
            Dialect::Css => None,
        }
    }

    /// SCSS and Less only.
    pub(crate) fn parse_variable_declaration(&mut self, panic: &[T]) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_variable_declaration(panic),
            Dialect::Less => self.less_parse_variable_declaration(panic),
            Dialect::Css => None,
        }
    }

    /// SCSS and Less only.
    pub(crate) fn parse_interpolation(&mut self) -> Option<NodeId> {
        match self.dialect {
            Dialect::Scss => self.scss_parse_interpolation(),
            Dialect::Less => self.less_parse_interpolation(),
            Dialect::Css => None,
        }
    }

    /// The parse functions by their upstream names, for `internal_parse`.
    pub(crate) fn parse_by_name(&mut self, name: &str) -> Option<Option<NodeId>> {
        Some(match name {
            "_parseVariableDeclaration" => self.parse_variable_declaration(&[]),
            "_parseVariable" => self.parse_variable(),
            "_parseInterpolation" => self.parse_interpolation(),
            "_parseMixinParameter" => self.parse_mixin_parameter(),
            "_tryParseMixinDeclaration" => self.try_parse_mixin_declaration(),
            "_tryParseMixinReference" => self.try_parse_mixin_reference(true),
            "_parseModuleMember" => self.parse_module_member(),
            "_parseForward" | "_parseUse" | "_parseMixinContent" => self.parse_scss_by_name(name),
            "_parseStylesheet" => Some(self.parse_stylesheet_root()),
            "_parseRuleset" => self.parse_ruleset(false),
            "_parseExpr" => self.parse_expr(false),
            "_parseDeclaration" => self.parse_declaration(None, false),
            "_parseMedia" => self.parse_media(false),
            "_parseURILiteral" => self.parse_uri_literal(),
            "_parseFunction" => self.parse_function(),
            "_parseRuleSetDeclaration" => self.parse_rule_set_declaration(),
            "_parseTerm" => self.parse_term(),
            "_parseScope" => self.parse_scope(),
            "_parsePseudo" => self.parse_pseudo(),
            "_parseSimpleSelector" => self.parse_simple_selector(),
            "_parseKeyframe" => self.parse_keyframe(),
            "_parseOperator" => self.parse_operator(),
            "_parseKeyframeSelector" => self.parse_keyframe_selector(),
            "_parseAttrib" => self.parse_attrib(),
            "_parsePage" => self.parse_page(),
            "_parseSupports" => self.parse_supports(false),
            "_parseSelector" => self.parse_selector(false),
            "_parseLayer" => self.parse_layer(false),
            "_parseFontFace" => self.parse_font_face(),
            "_parseImport" => self.parse_import(),
            "_parseProperty" => self.parse_property(),
            "_parseElementName" => self.parse_element_name(),
            "_parseIdent" => self.parse_ident(None),
            "_parseCombinator" => self.parse_combinator(),
            "_parsePrio" => self.parse_prio(),
            "_parseHexColor" => self.parse_hex_color(),
            "_parseUnaryOperator" => self.parse_unary_operator(),
            "_parseMedium" => self.parse_medium(),
            "_parseMediaQueryList" => Some(self.parse_media_query_list()),
            "_parseClass" => self.parse_class(),
            "_parseStartingStyleAtRule" => self.parse_starting_style_at_rule(false),
            _ => return None,
        })
    }
}
