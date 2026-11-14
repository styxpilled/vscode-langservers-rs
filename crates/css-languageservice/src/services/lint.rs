//! Port of `lint.ts`.

use foldhash::{HashMap, HashSet};
use indexmap::IndexMap;

use serde_json::Value;

use super::lint_rules::{self as rules, LintConfigurationSettings, VALID_PROPERTIES};
use super::lint_util::{Element, calculate_box_model};
use crate::language_facts::CssDataManager;
use crate::language_facts::builtin_data::UNITS;
use crate::parser::nodes::{Ast, Field, Marker, NodeId, NodeType, Rule};
use crate::utils::{is_js_whitespace, parse_float, utf16_len};

const PREFIXES: [&str; 4] = ["-ms-", "-moz-", "-o-", "-webkit-"];

/// `NodesByRootMap`, in insertion order.
#[derive(Default)]
struct NodesByRootMap(IndexMap<String, RootEntry, foldhash::fast::RandomState>);

#[derive(Default)]
struct RootEntry {
    nodes: Vec<NodeId>,
    names: Vec<String>,
}

impl NodesByRootMap {
    fn add(&mut self, root: &str, name: &str, node: Option<NodeId>) {
        let entry = match self.0.get_mut(root) {
            Some(entry) => entry,
            None => self.0.entry(root.to_owned()).or_default(),
        };
        entry.names.push(name.to_owned());
        entry.nodes.extend(node);
    }
}

/// `LintVisitor.entries`. `filter` is a mask of `Level` values; upstream
/// defaults to warnings and errors.
pub fn lint_entries(
    ast: &Ast,
    node: NodeId,
    settings: LintConfigurationSettings,
    data: &CssDataManager,
    filter: u8,
) -> Vec<Marker> {
    let mut visitor = LintVisitor::new(ast, settings, data);
    ast.accept(node, &mut |n| visitor.visit_node(n));
    visitor.validate_keyframes();
    visitor
        .warnings
        .into_iter()
        .filter(|e| e.level as u8 & filter != 0)
        .collect()
}

struct LintVisitor<'a> {
    ast: &'a Ast,
    settings: LintConfigurationSettings<'a>,
    data: &'a CssDataManager,
    warnings: Vec<Marker>,
    keyframes: NodesByRootMap,
    valid_properties: HashSet<String>,
}

impl<'a> LintVisitor<'a> {
    fn new(
        ast: &'a Ast,
        settings: LintConfigurationSettings<'a>,
        data: &'a CssDataManager,
    ) -> Self {
        let mut valid_properties = HashSet::default();
        if let Some(Value::Array(properties)) = settings.get_setting(VALID_PROPERTIES) {
            for p in properties {
                if let Value::String(p) = p {
                    let name = p.trim_matches(is_js_whitespace).to_lowercase();
                    if !name.is_empty() {
                        valid_properties.insert(name);
                    }
                }
            }
        }
        LintVisitor {
            ast,
            settings,
            data,
            warnings: Vec::new(),
            keyframes: NodesByRootMap::default(),
            valid_properties,
        }
    }

    fn fetch<'e>(&self, input: &'e [Element], s: &str) -> Vec<&'e Element> {
        input.iter().filter(|e| e.full_property_name == s).collect()
    }

    fn fetch_with_value<'e>(&self, input: &'e [Element], s: &str, v: &str) -> Vec<&'e Element> {
        input
            .iter()
            .filter(|e| {
                e.full_property_name == s
                    && self
                        .ast
                        .field(e.node, Field::Value)
                        .is_some_and(|expr| self.find_value_in_expression(expr, v))
            })
            .collect()
    }

    fn find_value_in_expression(&self, expression: NodeId, v: &str) -> bool {
        let mut found = false;
        self.ast.accept(expression, &mut |n| {
            if self.ast.node_type(n) == NodeType::Identifier && self.ast.matches(n, v) {
                found = true;
            }
            !found
        });
        found
    }

    fn add_entry(&mut self, node: NodeId, rule: &'static Rule, details: Option<String>) {
        let n = self.ast.node(node);
        self.warnings.push(Marker {
            node,
            rule,
            level: self.settings.get_rule(rule),
            message: details.unwrap_or_else(|| rule.message.to_owned()),
            offset: n.offset(),
            length: n.length(),
        });
    }

    fn get_missing_names(expected: &[String], actual: &[String]) -> Option<String> {
        let mut expected: Vec<Option<&String>> = expected.iter().map(Some).collect();
        for a in actual {
            if let Some(k) = expected.iter().position(|e| *e == Some(a)) {
                expected[k] = None;
            }
        }
        let mut result: Option<String> = None;
        for curr in expected.into_iter().flatten() {
            if curr.is_empty() {
                continue;
            }
            result = Some(match result {
                None => format!("'{curr}'"),
                Some(r) => format!("{r}, '{curr}'"),
            });
        }
        result
    }

    fn visit_node(&mut self, node: NodeId) -> bool {
        match self.ast.node_type(node) {
            NodeType::UnknownAtRule => self.visit_unknown_at_rule(node),
            NodeType::Keyframe => self.visit_keyframe(node),
            NodeType::FontFace => self.visit_font_face(node),
            NodeType::Ruleset => self.visit_rule_set(node),
            NodeType::SimpleSelector => self.visit_simple_selector(node),
            NodeType::NumericValue => self.visit_numeric_value(node),
            NodeType::Import => {
                self.add_entry(node, &rules::IMPORT_STATEMENT, None);
                true
            }
            NodeType::HexColorValue => self.visit_hex_color_value(node),
            NodeType::Prio => {
                self.add_entry(node, &rules::AVOID_IMPORTANT, None);
                true
            }
            NodeType::IdentifierSelector => {
                self.add_entry(node, &rules::AVOID_ID_SELECTOR, None);
                true
            }
            // `visitFunction` is not ported: it compares the name with `rgb(`,
            // so it never reports an entry. See `docs/differences/css-lint.md`.
            _ => true,
        }
    }

    fn visit_unknown_at_rule(&mut self, node: NodeId) -> bool {
        let Some(at_rule_name) = self.ast.child(node, 0) else {
            return false;
        };
        let text = self.ast.get_text(at_rule_name);
        if self.data.get_at_directive(text).is_some() {
            return false;
        }
        let message = format!("Unknown at rule {text}");
        self.add_entry(at_rule_name, &rules::UNKNOWN_AT_RULES, Some(message));
        true
    }

    fn visit_keyframe(&mut self, node: NodeId) -> bool {
        let Some(keyword) = self.ast.field(node, Field::Keyword) else {
            return false;
        };
        let text = self.ast.get_text(keyword);
        let name = self.ast.identifier_name(node);
        let vendor = (text != "@keyframes").then_some(keyword);
        self.keyframes.add(name, text, vendor);
        true
    }

    fn validate_keyframes(&mut self) {
        let expected = ["@-webkit-keyframes", "@-moz-keyframes", "@-o-keyframes"].map(String::from);
        for (_, entry) in std::mem::take(&mut self.keyframes.0) {
            let actual = &entry.names;
            let needs_standard = !actual.iter().any(|a| a == "@keyframes");
            if !needs_standard && actual.len() == 1 {
                continue;
            }
            let missing_vendor_specific = Self::get_missing_names(&expected, actual);
            if missing_vendor_specific.is_some() || needs_standard {
                for &node in &entry.nodes {
                    if needs_standard {
                        let message =
                            "Always define standard rule '@keyframes' when defining keyframes."
                                .to_owned();
                        self.add_entry(
                            node,
                            &rules::INCLUDE_STANDARD_PROPERTY_WHEN_USING_VENDOR_PREFIX,
                            Some(message),
                        );
                    }
                    if let Some(missing) = &missing_vendor_specific {
                        let message =
                            format!("Always include all vendor specific rules: Missing: {missing}");
                        self.add_entry(node, &rules::ALL_VENDOR_PREFIXES, Some(message));
                    }
                }
            }
        }
    }

    fn visit_simple_selector(&mut self, node: NodeId) -> bool {
        let n = self.ast.node(node);
        if n.length() == 1 && byte_at(self.ast, n.offset()) == Some(b'*') {
            self.add_entry(node, &rules::UNIVERSAL_SELECTOR, None);
        }
        true
    }

    fn visit_rule_set(&mut self, node: NodeId) -> bool {
        let ast = self.ast;
        let Some(declarations) = ast.field(node, Field::Declarations) else {
            return false;
        };
        let selectors = ast.field(node, Field::Selectors);
        if !ast.has_children(declarations)
            && let Some(selectors) = selectors
        {
            self.add_entry(selectors, &rules::EMPTY_RULE_SET, None);
        }

        let property_table: Vec<Element> = ast
            .children(declarations)
            .filter(|&c| ast.kind(c).is_declaration())
            .map(|c| Element::new(ast, c))
            .collect();

        let box_model = calculate_box_model(ast, &property_table);
        let sides = [
            (box_model.width, &box_model.right, &box_model.left),
            (box_model.height, &box_model.top, &box_model.bottom),
        ];
        for (dimension, a, b) in sides {
            let Some(dimension) = dimension else {
                continue;
            };
            let mut properties: Vec<usize> = Vec::new();
            for side in [a, b] {
                if side.value {
                    for &p in &side.properties {
                        if !properties.contains(&p) {
                            properties.push(p);
                        }
                    }
                }
            }
            if !properties.is_empty() {
                for p in properties {
                    self.add_entry(
                        property_table[p].node,
                        &rules::BEWARE_OF_BOX_MODEL_SIZE,
                        None,
                    );
                }
                self.add_entry(
                    property_table[dimension].node,
                    &rules::BEWARE_OF_BOX_MODEL_SIZE,
                    None,
                );
            }
        }

        if !self
            .fetch_with_value(&property_table, "display", "inline-block")
            .is_empty()
        {
            for elem in self.fetch(&property_table, "float") {
                if let Some(value) = ast.field(elem.node, Field::Value)
                    && !ast.matches(value, "none")
                {
                    let message = "inline-block is ignored due to the float. If 'float' has a value other than 'none', the box is floated and 'display' is treated as 'block'";
                    self.add_entry(
                        elem.node,
                        &rules::PROPERTY_IGNORED_DUE_TO_DISPLAY,
                        Some(message.to_owned()),
                    );
                }
            }
        }

        if !self
            .fetch_with_value(&property_table, "display", "block")
            .is_empty()
        {
            for elem in self.fetch(&property_table, "vertical-align") {
                let message = "Property is ignored due to the display. With 'display: block', vertical-align should not be used.";
                self.add_entry(
                    elem.node,
                    &rules::PROPERTY_IGNORED_DUE_TO_DISPLAY,
                    Some(message.to_owned()),
                );
            }
        }

        for elem in self.fetch(&property_table, "float") {
            if !self.valid_properties.contains(&elem.full_property_name) {
                self.add_entry(elem.node, &rules::AVOID_FLOAT, None);
            }
        }

        let not_negative = |decl: NodeId| {
            ast.field(decl, Field::Value)
                .is_some_and(|v| byte_at(ast, ast.offset(v)) != Some(b'-'))
        };
        let candidates: Vec<&Element> = property_table
            .iter()
            .filter(|e| {
                e.full_property_name != "background"
                    && !self.valid_properties.contains(&e.full_property_name)
                    && not_negative(e.node)
            })
            .collect();
        let mut counts: HashMap<&str, usize> = HashMap::default();
        for e in &candidates {
            *counts.entry(&e.full_property_name).or_default() += 1;
        }
        for e in candidates {
            if counts[e.full_property_name.as_str()] > 1 {
                self.add_entry(e.node, &rules::DUPLICATE_DECLARATIONS, None);
            }
        }

        let Some(selectors) = selectors else {
            return true;
        };
        let is_icss_block = ast.matches(selectors, ":export")
            || (ast.starts_with(selectors, ":import(") && ast.ends_with(selectors, ")"));
        if !is_icss_block {
            self.check_properties(node, &property_table);
        }
        true
    }

    /// Unknown properties, and vendor prefixes without the other prefixes or
    /// the standard property.
    fn check_properties(&mut self, node: NodeId, property_table: &[Element]) {
        let ast = self.ast;
        let mut properties_by_suffix = NodesByRootMap::default();
        let mut contains_unknowns = false;

        for element in property_table {
            let decl = element.node;
            if !self.is_css_declaration(decl) {
                contains_unknowns = true;
                continue;
            }
            let property = ast.field(decl, Field::Property).expect("checked");
            let name = element.full_property_name.as_str();
            if let Some(rest) = name.strip_prefix('-') {
                if !rest.starts_with('-') {
                    if !self.data.is_known_property(name) && !self.valid_properties.contains(name) {
                        self.add_entry(property, &rules::UNKNOWN_VENDOR_SPECIFIC_PROPERTY, None);
                    }
                    let non_prefixed = rest.find('-').map_or(name, |end| &rest[end + 1..]);
                    properties_by_suffix.add(non_prefixed, name, Some(property));
                }
            } else {
                let mut short = name;
                if name.starts_with(['*', '_']) {
                    self.add_entry(property, &rules::IE_STAR_HACK, None);
                    short = &name[1..];
                }
                if !self.data.is_known_property(name)
                    && !self.data.is_known_property(short)
                    && !self.valid_properties.contains(short)
                {
                    let message = format!("Unknown property: '{}'", ast.full_property_name(decl));
                    self.add_entry(property, &rules::UNKNOWN_PROPERTY, Some(message));
                }
                properties_by_suffix.add(short, short, None);
            }
        }

        if contains_unknowns {
            return;
        }
        for (suffix, entry) in &properties_by_suffix.0 {
            let actual = &entry.names;
            let needs_standard =
                self.data.is_standard_property(suffix) && !actual.iter().any(|a| a == suffix);
            if !needs_standard && actual.len() == 1 {
                continue;
            }

            let mut need_standard: Vec<NodeId> = Vec::new();
            if needs_standard {
                let pseudo_elements = self.contextual_vendor_specific_pseudo_elements(node);
                for &n in &entry.nodes {
                    let property_name = ast.property_name(n);
                    let prefix = utf16_prefix(
                        property_name,
                        utf16_len(property_name).saturating_sub(utf16_len(suffix)),
                    );
                    if !pseudo_elements.iter().any(|x| x.starts_with(prefix)) {
                        need_standard.push(n);
                    }
                }
            }

            let expected: Vec<String> = PREFIXES
                .iter()
                .map(|p| format!("{p}{suffix}"))
                .filter(|name| self.data.is_standard_property(name))
                .collect();

            let missing_vendor_specific = Self::get_missing_names(&expected, actual);
            if missing_vendor_specific.is_some() || needs_standard {
                for &n in &entry.nodes {
                    if needs_standard && need_standard.contains(&n) {
                        let message = format!(
                            "Also define the standard property '{suffix}' for compatibility"
                        );
                        self.add_entry(
                            n,
                            &rules::INCLUDE_STANDARD_PROPERTY_WHEN_USING_VENDOR_PREFIX,
                            Some(message),
                        );
                    }
                    if let Some(missing) = &missing_vendor_specific {
                        let message = format!(
                            "Always include all vendor specific properties: Missing: {missing}"
                        );
                        self.add_entry(n, &rules::ALL_VENDOR_PREFIXES, Some(message));
                    }
                }
            }
        }
    }

    /// The pseudo-elements in the selectors of `node` and its parent rule sets.
    fn contextual_vendor_specific_pseudo_elements(&self, node: NodeId) -> Vec<&'a str> {
        let ast = self.ast;
        let mut result: Vec<&str> = Vec::new();
        let mut walk_down = |n: NodeId| {
            ast.accept(n, &mut |c| {
                if c != n
                    && ast.node_type(c) == NodeType::PseudoSelector
                    && let Some(first) = ast.child(c, 0)
                {
                    let text = ast.get_text(first);
                    if !text.is_empty() && !result.contains(&text) {
                        result.push(text);
                    }
                }
                true
            });
        };
        let mut current = Some(node);
        while let Some(n) = current {
            if ast.node_type(n) == NodeType::Ruleset {
                for selector in ast.list_children(n, Field::Selectors) {
                    walk_down(selector);
                }
            }
            current = ast.parent(n);
        }
        result
    }

    fn visit_numeric_value(&mut self, node: NodeId) -> bool {
        let ast = self.ast;
        if let Some(function) = ast.find_parent(node, NodeType::Function)
            && ast.identifier_name(function) == "calc"
        {
            return true;
        }
        let Some(decl) = ast.find_parent(node, NodeType::Declaration) else {
            return true;
        };
        if ast.field(decl, Field::Value).is_none() {
            return true;
        }
        let (value, unit) = ast.numeric_value(node);
        let Some(unit) = unit else {
            return true;
        };
        let lengths = UNITS
            .iter()
            .find(|(k, _)| *k == "length")
            .map_or(&[][..], |u| u.1);
        if !lengths.contains(&unit.to_lowercase().as_str()) {
            return true;
        }
        if parse_float(value) == 0.0
            && !self
                .valid_properties
                .contains(&ast.full_property_name(decl).to_lowercase())
        {
            self.add_entry(node, &rules::ZERO_WITH_UNIT, None);
        }
        true
    }

    fn visit_font_face(&mut self, node: NodeId) -> bool {
        let ast = self.ast;
        let Some(declarations) = ast.field(node, Field::Declarations) else {
            return false;
        };
        let mut defines_src = false;
        let mut defines_font_family = false;
        let mut contains_unknowns = false;
        for child in ast.children(declarations) {
            if self.is_css_declaration(child) {
                let property = ast.field(child, Field::Property).expect("checked");
                let name = ast.property_name(property).to_lowercase();
                defines_src |= name == "src";
                defines_font_family |= name == "font-family";
            } else {
                contains_unknowns = true;
            }
        }
        if !contains_unknowns && (!defines_src || !defines_font_family) {
            self.add_entry(node, &rules::REQUIRED_PROPERTIES_FOR_FONT_FACE, None);
        }
        true
    }

    fn is_css_declaration(&self, node: NodeId) -> bool {
        let ast = self.ast;
        ast.kind(node).is_declaration()
            && ast.field(node, Field::Value).is_some()
            && ast
                .field(node, Field::Property)
                .and_then(|p| ast.field(p, Field::Identifier))
                .is_some_and(|id| !ast.has_children(id))
    }

    fn visit_hex_color_value(&mut self, node: NodeId) -> bool {
        if ![9, 7, 5, 4].contains(&self.ast.get_text(node).len()) {
            self.add_entry(node, &rules::HEX_COLOR_LENGTH, None);
        }
        false
    }
}

/// `s.substring(0, n)` with `n` in UTF-16 units, at a character boundary.
fn utf16_prefix(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

/// `documentText.charAt(offset)` for an ASCII character.
fn byte_at(ast: &Ast, offset: isize) -> Option<u8> {
    usize::try_from(offset)
        .ok()
        .and_then(|o| ast.text().as_bytes().get(o).copied())
}
