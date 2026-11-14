//! Port of `selectorPrinting.ts`.

use lsp_types::{LanguageString, MarkedString};

use crate::language_facts::CssDataManager;
use crate::parser::nodes::{Ast, Field, Kind, NodeId, NodeType};
use crate::parser::scanner::Scanner;
use crate::parser::{Dialect, Parser};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ElementId(usize);

/// The upstream classes `Element`, `RootElement` and `LabelElement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementKind {
    Element,
    Root,
    Label,
}

/// An attribute. `value` is `None` for `[name]` (`undefined` upstream).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Element {
    pub kind: ElementKind,
    pub parent: Option<ElementId>,
    pub children: Option<Vec<ElementId>>,
    pub attributes: Option<Vec<Attribute>>,
}

/// The elements of one selector, in an arena.
#[derive(Clone, Debug, Default)]
pub struct Elements {
    elements: Vec<Element>,
}

/// Concatenation of attribute values. Upstream adds `undefined` as text.
fn concat(a: Option<&str>, sep: &str, b: Option<&str>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a}{sep}{b}")),
        (a, b) => a.or(b).map(str::to_owned),
    }
}

impl Elements {
    pub fn get(&self, id: ElementId) -> &Element {
        &self.elements[id.0]
    }

    fn get_mut(&mut self, id: ElementId) -> &mut Element {
        &mut self.elements[id.0]
    }

    pub fn create(&mut self, kind: ElementKind) -> ElementId {
        self.elements.push(Element {
            kind,
            parent: None,
            children: None,
            attributes: None,
        });
        ElementId(self.elements.len() - 1)
    }

    fn label(&mut self, label: &str) -> ElementId {
        let id = self.create(ElementKind::Label);
        self.add_attr(id, "name", Some(label));
        id
    }

    pub fn children(&self, id: ElementId) -> &[ElementId] {
        self.get(id).children.as_deref().unwrap_or(&[])
    }

    pub fn find_attribute(&self, id: ElementId, name: &str) -> Option<Option<&str>> {
        self.get(id)
            .attributes
            .as_ref()?
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_deref())
    }

    pub fn add_child(&mut self, id: ElementId, child: ElementId) {
        self.get_mut(child).parent = Some(id);
        self.get_mut(id)
            .children
            .get_or_insert_default()
            .push(child);
    }

    fn append(&mut self, id: ElementId, text: &str) {
        if let Some(last) = self
            .get_mut(id)
            .attributes
            .as_mut()
            .and_then(|a| a.last_mut())
        {
            last.value = concat(last.value.as_deref(), "", Some(text));
        }
    }

    fn prepend(&mut self, id: ElementId, text: &str) {
        if let Some(first) = self
            .get_mut(id)
            .attributes
            .as_mut()
            .and_then(|a| a.first_mut())
        {
            first.value = concat(Some(text), "", first.value.as_deref());
        }
    }

    pub fn find_root(&self, id: ElementId) -> ElementId {
        let mut curr = id;
        while let Some(parent) = self.get(curr).parent
            && self.get(parent).kind != ElementKind::Root
        {
            curr = parent;
        }
        curr
    }

    fn remove_child(&mut self, id: ElementId, child: ElementId) {
        if let Some(children) = self.get_mut(id).children.as_mut()
            && let Some(i) = children.iter().position(|&c| c == child)
        {
            children.remove(i);
        }
    }

    fn add_attr(&mut self, id: ElementId, name: &str, value: Option<&str>) {
        let attributes = self.get_mut(id).attributes.get_or_insert_default();
        if let Some(a) = attributes.iter_mut().find(|a| a.name == name) {
            a.value = concat(a.value.as_deref(), " ", value);
            return;
        }
        attributes.push(Attribute {
            name: name.to_owned(),
            value: value.map(str::to_owned),
        });
    }

    /// `clone`: the copy is always a plain `Element`, as upstream.
    fn clone_element(&mut self, id: ElementId, clone_children: bool) -> ElementId {
        let elem = self.create(ElementKind::Element);
        if let Some(attributes) = self.get(id).attributes.clone() {
            self.get_mut(elem).attributes = Some(Vec::new());
            for a in attributes {
                self.add_attr(elem, &a.name, a.value.as_deref());
            }
        }
        if clone_children && let Some(children) = self.get(id).children.clone() {
            self.get_mut(elem).children = Some(Vec::new());
            for child in children {
                let c = self.clone_element(child, true);
                self.add_child(elem, c);
            }
        }
        elem
    }

    fn clone_with_parent(&mut self, id: ElementId) -> ElementId {
        let clone = self.clone_element(id, false);
        if let Some(parent) = self.get(id).parent
            && self.get(parent).kind != ElementKind::Root
        {
            let parent_clone = self.clone_with_parent(parent);
            self.add_child(parent_clone, clone);
        }
        clone
    }
}

/// `quotes.remove`: strips quotes matched by `/^['"](.*)["']$/`.
fn remove_quotes(value: &str) -> &str {
    let b = value.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'\'' | b'"') && matches!(b[b.len() - 1], b'\'' | b'"') {
        let inner = &value[1..value.len() - 1];
        if !inner.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
            return inner;
        }
    }
    value
}

fn unescape(content: &str) -> String {
    let mut scanner = Scanner::new(Dialect::Css);
    scanner.set_source(content);
    match scanner.scan_unquoted_string() {
        Some(token) => scanner.text(&token).into_owned(),
        None => content.to_owned(),
    }
}

/// `MarkedStringPrinter` with `"` quotes.
fn print(elements: &Elements, element: ElementId, contexts: &[String]) -> Vec<MarkedString> {
    let mut result: Vec<String> = contexts.to_vec();
    let roots = if elements.get(element).kind == ElementKind::Root {
        elements.children(element).to_vec()
    } else {
        vec![element]
    };
    print_elements(elements, &roots, 0, &mut result);
    vec![MarkedString::LanguageString(LanguageString {
        language: "html".to_owned(),
        value: result.join("\n"),
    })]
}

fn print_elements(elements: &Elements, list: &[ElementId], indent: usize, out: &mut Vec<String>) {
    for &element in list {
        print_element(elements, element, indent, out);
        print_elements(elements, elements.children(element), indent + 1, out);
    }
}

fn print_element(elements: &Elements, element: ElementId, indent: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(indent);
    let name = elements.find_attribute(element, "name").flatten();
    if elements.get(element).kind == ElementKind::Label || name == Some("\u{2026}") {
        out.push(indent + name.unwrap_or("undefined"));
        return;
    }
    let mut content = String::from("<");
    content += name.filter(|n| !n.is_empty()).unwrap_or("element");
    for attr in elements.get(element).attributes.iter().flatten() {
        if attr.name != "name" {
            content.push(' ');
            content += &attr.name;
            if let Some(value) = attr.value.as_deref().filter(|v| !v.is_empty()) {
                content.push_str("=\"");
                content += remove_quotes(value);
                content.push('"');
            }
        }
    }
    content.push('>');
    out.push(indent + &content);
}

/// `toElement`.
pub fn to_element(
    elements: &mut Elements,
    ast: &Ast,
    node: NodeId,
    parent_element: Option<ElementId>,
) -> ElementId {
    let mut result = elements.create(ElementKind::Element);
    for child in ast.children(node) {
        let text = ast.get_text(child);
        match ast.node_type(child) {
            NodeType::SelectorCombinator => {
                let Some(parent) = parent_element else {
                    continue;
                };
                let segments: Vec<&str> = text.split('&').collect();
                if segments.len() == 1 {
                    elements.add_attr(result, "name", Some(segments[0]));
                    continue;
                }
                result = elements.clone_with_parent(parent);
                if !segments[0].is_empty() {
                    let root = elements.find_root(result);
                    elements.prepend(root, segments[0]);
                }
                for (i, segment) in segments.iter().enumerate().skip(1) {
                    if i > 1 {
                        let clone = elements.clone_with_parent(parent);
                        let root = elements.find_root(clone);
                        elements.add_child(result, root);
                        result = clone;
                    }
                    elements.append(result, segment);
                }
            }
            NodeType::SelectorPlaceholder | NodeType::ElementNameSelector => {
                if ast.node_type(child) == NodeType::SelectorPlaceholder && text == "@at-root" {
                    return result;
                }
                let name = if text == "*" {
                    "element".to_owned()
                } else {
                    unescape(text)
                };
                elements.add_attr(result, "name", Some(&name));
            }
            NodeType::ClassSelector => {
                let class = unescape(text.get(1..).unwrap_or(""));
                elements.add_attr(result, "class", Some(&class));
            }
            NodeType::IdentifierSelector => {
                let id = unescape(text.get(1..).unwrap_or(""));
                elements.add_attr(result, "id", Some(&id));
            }
            NodeType::MixinDeclaration => {
                elements.add_attr(result, "class", Some(ast.identifier_name(child)));
            }
            NodeType::PseudoSelector => {
                elements.add_attr(result, &unescape(text), Some(""));
            }
            NodeType::AttributeSelector => {
                let Some(identifier) = ast.field(child, Field::Identifier) else {
                    continue;
                };
                let value = match (
                    ast.field(child, Field::Value),
                    ast.field(child, Field::Operator),
                ) {
                    (Some(expression), Some(operator)) => {
                        let unescaped = unescape(ast.get_text(expression));
                        let v = remove_quotes(&unescaped);
                        Some(match unescape(ast.get_text(operator)).as_str() {
                            "|=" => format!("{v}-\u{2026}"),
                            "^=" => format!("{v}\u{2026}"),
                            "$=" => format!("\u{2026}{v}"),
                            "~=" => format!(" \u{2026} {v} \u{2026} "),
                            "*=" => format!("\u{2026}{v}\u{2026}"),
                            _ => v.to_owned(),
                        })
                    }
                    _ => None,
                };
                let name = unescape(ast.get_text(identifier));
                elements.add_attr(result, &name, value.as_deref());
            }
            _ => {}
        }
    }
    result
}

/// `SelectorElementBuilder`.
struct Builder<'a> {
    ast: &'a Ast,
    elements: &'a mut Elements,
    prev: Option<NodeId>,
    element: ElementId,
}

impl Builder<'_> {
    fn process_selector(&mut self, selector: NodeId) {
        let ast = self.ast;
        let mut parent_element = None;
        if self.elements.get(self.element).kind != ElementKind::Root
            && ast.children(selector).any(|c| {
                ast.first_child(c)
                    .is_some_and(|first| ast.node_type(first) == NodeType::SelectorCombinator)
            })
        {
            let curr = self.elements.find_root(self.element);
            if let Some(parent) = self.elements.get(curr).parent
                && self.elements.get(parent).kind == ElementKind::Root
            {
                parent_element = Some(self.element);
                self.element = parent;
                self.elements.remove_child(parent, curr);
                self.prev = None;
            }
        }

        for child in ast.children(selector) {
            let is_simple = ast.kind(child) == Kind::SimpleSelector;
            if is_simple {
                let prev_is = |s: &str| self.prev.is_some_and(|p| ast.matches(p, s));
                if self
                    .prev
                    .is_some_and(|p| ast.kind(p) == Kind::SimpleSelector)
                {
                    let label = self.elements.label("\u{2026}");
                    self.elements.add_child(self.element, label);
                    self.element = label;
                } else if (prev_is("+") || prev_is("~"))
                    && let Some(parent) = self.elements.get(self.element).parent
                {
                    self.element = parent;
                }
                if prev_is("~") {
                    let label = self.elements.label("\u{22EE}");
                    self.elements.add_child(self.element, label);
                }
                let this_element = to_element(self.elements, ast, child, parent_element);
                let root = self.elements.find_root(this_element);
                self.elements.add_child(self.element, root);
                self.element = this_element;
            }
            if is_simple
                || matches!(
                    ast.node_type(child),
                    NodeType::SelectorCombinatorParent
                        | NodeType::SelectorCombinatorShadowPiercingDescendant
                        | NodeType::SelectorCombinatorSibling
                        | NodeType::SelectorCombinatorAllSiblings
                )
            {
                self.prev = Some(child);
            }
        }
    }
}

/// `selectorToElement`: the root element, or `None` for `@at-root`.
pub fn selector_to_element(elements: &mut Elements, ast: &Ast, node: NodeId) -> Option<ElementId> {
    if ast.matches(node, "@at-root") {
        return None;
    }
    let root = elements.create(ElementKind::Root);
    let mut parent_rule_sets = Vec::new();
    if let Some(rule_set) = ast.get_parent(node)
        && ast.kind(rule_set) == Kind::RuleSet
    {
        let mut parent = ast.get_parent(rule_set);
        while let Some(p) = parent {
            if matches!(
                ast.node_type(p),
                NodeType::MixinDeclaration | NodeType::Stylesheet
            ) {
                break;
            }
            if ast.kind(p) == Kind::RuleSet {
                if ast
                    .field(p, Field::Selectors)
                    .is_some_and(|s| ast.matches(s, "@at-root"))
                {
                    break;
                }
                parent_rule_sets.push(p);
            }
            parent = ast.get_parent(p);
        }
    }

    let mut builder = Builder {
        ast,
        elements,
        prev: None,
        element: root,
    };
    for &rule_set in parent_rule_sets.iter().rev() {
        if let Some(selector) = ast.list_children(rule_set, Field::Selectors).next() {
            builder.process_selector(selector);
        }
    }
    builder.process_selector(node);
    Some(root)
}

#[derive(Clone, Copy, Default, Debug)]
struct Specificity {
    id: u32,
    attr: u32,
    tag: u32,
}

impl Specificity {
    fn add(&mut self, other: Specificity) {
        self.id += other.id;
        self.attr += other.attr;
        self.tag += other.tag;
    }
}

/// `SelectorPrinting`.
pub struct SelectorPrinting<'a> {
    data: &'a CssDataManager,
}

impl<'a> SelectorPrinting<'a> {
    pub fn new(data: &'a CssDataManager) -> Self {
        SelectorPrinting { data }
    }

    pub fn selector_to_marked_string(
        &self,
        ast: &Ast,
        node: NodeId,
        contexts: &[String],
    ) -> Vec<MarkedString> {
        let mut elements = Elements::default();
        let Some(root) = selector_to_element(&mut elements, ast, node) else {
            return Vec::new();
        };
        let mut marked = print(&elements, root, contexts);
        marked.push(self.specificity_marked_string(ast, node));
        marked
    }

    pub fn simple_selector_to_marked_string(&self, ast: &Ast, node: NodeId) -> Vec<MarkedString> {
        let mut elements = Elements::default();
        let element = to_element(&mut elements, ast, node, None);
        let mut marked = print(&elements, element, &[]);
        marked.push(self.specificity_marked_string(ast, node));
        marked
    }

    /// `/^::?([\w-]+)/` and a known pseudo-element of that name.
    fn is_pseudo_element_identifier(&self, text: &str) -> bool {
        let rest = text
            .strip_prefix("::")
            .or_else(|| text.strip_prefix(':'))
            .unwrap_or_default();
        let n = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
            .count();
        n > 0
            && self
                .data
                .get_pseudo_element(&format!("::{}", &rest[..n]))
                .is_some()
    }

    fn specificity_marked_string(&self, ast: &Ast, node: NodeId) -> MarkedString {
        let s = self.calculate_score(ast, node);
        MarkedString::String(format!(
            "[Selector Specificity](https://developer.mozilla.org/docs/Web/CSS/Specificity): ({}, {}, {})",
            s.id, s.attr, s.tag
        ))
    }

    fn most_specific_list_item(&self, ast: &Ast, containers: &[NodeId]) -> Specificity {
        let mut most = Specificity::default();
        for &container in containers {
            for child in ast.children(container) {
                let item = self.calculate_score(ast, child);
                if (item.id, item.attr, item.tag) > (most.id, most.attr, most.tag) {
                    most = item;
                }
            }
        }
        most
    }

    fn calculate_score(&self, ast: &Ast, node: NodeId) -> Specificity {
        let mut specificity = Specificity::default();
        for element in ast.children(node) {
            match ast.node_type(element) {
                NodeType::IdentifierSelector => specificity.id += 1,
                NodeType::ClassSelector | NodeType::AttributeSelector => specificity.attr += 1,
                NodeType::ElementNameSelector => {
                    if !ast.matches(element, "*") {
                        specificity.tag += 1;
                    }
                }
                NodeType::PseudoSelector => {
                    self.pseudo_score(ast, element, &mut specificity);
                    continue;
                }
                _ => {}
            }
            if ast.has_children(element) {
                specificity.add(self.calculate_score(ast, element));
            }
        }
        specificity
    }

    fn pseudo_score(&self, ast: &Ast, element: NodeId, specificity: &mut Specificity) {
        let text = ast.get_text(element);
        let children: Vec<NodeId> = ast.children(element).collect();
        let children = &children[..];
        let starts = |prefix: &str| {
            text.get(..prefix.len())
                .is_some_and(|t| t.eq_ignore_ascii_case(prefix))
        };
        let starts_any = |prefixes: &[&str]| prefixes.iter().any(|p| starts(p));

        if self.is_pseudo_element_identifier(text) {
            specificity.tag += 1;
            if starts("::slotted") && !children.is_empty() {
                specificity.add(self.most_specific_list_item(ast, children));
            }
            return;
        }
        if starts(":where") {
            return;
        }
        if starts_any(&[":not", ":has", ":is"]) && !children.is_empty() {
            specificity.add(self.most_specific_list_item(ast, children));
            return;
        }
        if starts(":host") && !children.is_empty() {
            specificity.attr += 1;
            specificity.add(self.most_specific_list_item(ast, children));
            return;
        }
        if starts_any(&[":nth-child", ":nth-last-child"]) && !children.is_empty() {
            specificity.attr += 1;
            let last = children[children.len() - 1];
            if children.len() > 2 && ast.node_type(last) == NodeType::SelectorList {
                specificity.add(
                    self.most_specific_list_item(ast, &ast.children(last).collect::<Vec<_>>()),
                );
                return;
            }
            if children.len() < 2 {
                return;
            }
            // `n` without A and B is not a binary expression.
            let pseudo_text = ast.get_text(children[1]);
            let mut scanner = Scanner::new(Dialect::Css);
            scanner.set_source(pseudo_text);
            let first = scanner.scan();
            let second = scanner.scan();
            let first = scanner.text(&first);
            if first == "n" || (first == "-n" && scanner.text(&second) == "of") {
                // The selectors have their own trees: the most specific item
                // of all is the most specific of each tree's best.
                let mut most = Specificity::default();
                for selector in utf16_skip(pseudo_text, second.offset, 2).split(',') {
                    let mut parser = Parser::new(Dialect::Css, selector);
                    if let Some(Some(node)) = parser.internal_parse("_parseSelector") {
                        let item = self.most_specific_list_item(&parser.ast, &[node]);
                        if (item.id, item.attr, item.tag) > (most.id, most.attr, most.tag) {
                            most = item;
                        }
                    }
                }
                specificity.add(most);
            }
            return;
        }
        specificity.attr += 1;
    }
}

/// `s.slice(offset + n)` with `offset` in bytes and `n` in UTF-16 units.
fn utf16_skip(s: &str, offset: usize, n: usize) -> &str {
    let rest = s.get(offset..).unwrap_or("");
    let mut units = 0;
    for (i, c) in rest.char_indices() {
        if units >= n {
            return &rest[i..];
        }
        units += c.len_utf16();
    }
    ""
}
