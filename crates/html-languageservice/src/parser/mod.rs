//! Port of `htmlParser.ts`.

pub mod entities;
pub mod scanner;

use scanner::{Scanner, ScannerState, TokenType};

pub type NodeId = usize;

/// The pseudo node that holds the roots.
const DOCUMENT: NodeId = 0;

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub tag: Option<String>,
    pub closed: bool,
    pub start: usize,
    pub end: usize,
    pub start_tag_end: Option<usize>,
    pub end_tag_start: Option<usize>,
    /// The attributes in source order, with the last value of each name.
    /// Upstream uses a JS object: integer names come first there, and
    /// `__proto__` is not kept.
    pub attributes: Option<Vec<(String, Option<String>)>>,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
}

impl Node {
    fn new(start: usize, end: usize, parent: Option<NodeId>) -> Self {
        Node {
            tag: None,
            closed: false,
            start,
            end,
            start_tag_end: None,
            end_tag_start: None,
            attributes: None,
            children: Vec::new(),
            parent,
        }
    }

    /// `attributeNames`.
    pub fn attribute_names(&self) -> impl Iterator<Item = &str> {
        self.attributes.iter().flatten().map(|(n, _)| n.as_str())
    }

    /// `attributes[name]`: `None` for a missing attribute, `Some(None)` for an
    /// attribute without value.
    pub fn attribute(&self, name: &str) -> Option<Option<&str>> {
        self.attributes
            .as_ref()?
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_deref())
    }

    /// Tag names are ASCII (the scanner's name pattern).
    fn is_same_tag(&self, tag: Option<&str>) -> bool {
        match (&self.tag, tag) {
            (None, t) => t.is_none(),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (Some(_), None) => false,
        }
    }

    /// `startTagEnd` as a JS condition (a number that is not 0).
    pub fn start_tag_end_truthy(&self) -> Option<usize> {
        self.start_tag_end.filter(|&n| n != 0)
    }

    /// `endTagStart` as a JS condition (a number that is not 0).
    pub fn end_tag_start_truthy(&self) -> Option<usize> {
        self.end_tag_start.filter(|&n| n != 0)
    }
}

/// `HTMLDocument`.
#[derive(Clone, Debug)]
pub struct HtmlDocument {
    nodes: Vec<Node>,
}

impl std::ops::Index<NodeId> for HtmlDocument {
    type Output = Node;

    fn index(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }
}

impl HtmlDocument {
    pub fn roots(&self) -> &[NodeId] {
        &self.nodes[DOCUMENT].children
    }

    /// The count of nodes, without the pseudo node of the document.
    pub fn node_count(&self) -> usize {
        self.nodes.len() - 1
    }

    /// The node of `id`, or `None` for the pseudo node of the document.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id].parent.filter(|&p| p != DOCUMENT)
    }

    /// Whether `id` is the pseudo node that holds the roots. Upstream returns
    /// it from `findNodeAt` and `findNodeBefore` (a node without tag).
    pub fn is_document(&self, id: NodeId) -> bool {
        id == DOCUMENT
    }

    /// `findNodeBefore`.
    pub fn find_node_before(&self, offset: usize) -> NodeId {
        let mut id = DOCUMENT;
        loop {
            let node = &self.nodes[id];
            let idx = node
                .children
                .partition_point(|&c| offset > self.nodes[c].start);
            if idx == 0 {
                return id;
            }
            let child_id = node.children[idx - 1];
            let child = &self.nodes[child_id];
            if offset <= child.start {
                return id;
            }
            if offset < child.end {
                id = child_id;
                continue;
            }
            match child.children.last() {
                Some(&last) if self.nodes[last].end == child.end => id = child_id,
                _ => return child_id,
            }
        }
    }

    /// `findNodeAt`.
    pub fn find_node_at(&self, offset: usize) -> NodeId {
        let mut id = DOCUMENT;
        loop {
            let node = &self.nodes[id];
            let idx = node
                .children
                .partition_point(|&c| offset > self.nodes[c].start);
            if idx == 0 {
                return id;
            }
            let child_id = node.children[idx - 1];
            let child = &self.nodes[child_id];
            if offset > child.start && offset <= child.end {
                id = child_id;
            } else {
                return id;
            }
        }
    }

    /// `HTMLParser.parse`.
    pub fn parse(
        text: &str,
        is_void_element: impl Fn(&str) -> bool,
        create_nodes_for_orphan_end_tags: bool,
    ) -> HtmlDocument {
        let mut scanner = Scanner::new(text, 0, ScannerState::WithinContent, true);
        // Each node starts at a `<`. The pages that stay unused are not touched.
        let mut nodes = Vec::with_capacity(1 + text.bytes().filter(|&b| b == b'<').count());
        nodes.push(Node::new(0, text.len(), None));
        let mut curr = DOCUMENT;
        let mut end_tag_start = 0;
        let mut end_tag_name: Option<&str> = None;
        let mut pending_attribute: Option<usize> = None;
        let mut token = scanner.scan();
        while token != TokenType::Eos {
            match token {
                TokenType::StartTagOpen => {
                    let child = nodes.len();
                    nodes.push(Node::new(scanner.token_offset(), text.len(), Some(curr)));
                    nodes[curr].children.push(child);
                    curr = child;
                }
                TokenType::StartTag => {
                    nodes[curr].tag = Some(scanner.token_text().to_owned());
                }
                TokenType::StartTagClose => {
                    if let Some(parent) = nodes[curr].parent {
                        nodes[curr].end = scanner.token_end();
                        if scanner.token_length() > 0 {
                            nodes[curr].start_tag_end = Some(scanner.token_end());
                            if nodes[curr].tag.as_deref().is_some_and(&is_void_element) {
                                nodes[curr].closed = true;
                                curr = parent;
                            }
                        } else {
                            // A pseudo close token of an incomplete start tag.
                            curr = parent;
                        }
                    }
                }
                TokenType::StartTagSelfClose => {
                    if let Some(parent) = nodes[curr].parent {
                        let node = &mut nodes[curr];
                        node.closed = true;
                        node.end = scanner.token_end();
                        node.start_tag_end = Some(scanner.token_end());
                        curr = parent;
                    }
                }
                TokenType::EndTagOpen => {
                    end_tag_start = scanner.token_offset();
                    end_tag_name = None;
                }
                TokenType::EndTag => {
                    end_tag_name = Some(scanner.token_text());
                }
                TokenType::EndTagClose => {
                    let mut node = curr;
                    while !nodes[node].is_same_tag(end_tag_name)
                        && let Some(parent) = nodes[node].parent
                    {
                        node = parent;
                    }
                    if nodes[node].parent.is_some() {
                        while curr != node {
                            nodes[curr].end = end_tag_start;
                            nodes[curr].closed = false;
                            curr = nodes[curr].parent.unwrap();
                        }
                        let n = &mut nodes[curr];
                        n.closed = true;
                        n.end_tag_start = Some(end_tag_start);
                        n.end = scanner.token_end();
                        curr = n.parent.unwrap();
                    } else if create_nodes_for_orphan_end_tags {
                        let orphan = nodes.len();
                        let mut n = Node::new(end_tag_start, scanner.token_end(), Some(curr));
                        n.tag = end_tag_name.map(str::to_ascii_lowercase);
                        n.closed = true;
                        n.end_tag_start = Some(end_tag_start);
                        nodes.push(n);
                        nodes[curr].children.push(orphan);
                    }
                }
                TokenType::AttributeName => {
                    let name = scanner.token_text();
                    let attributes = nodes[curr].attributes.get_or_insert_default();
                    let index = match attributes.iter().position(|(n, _)| n == name) {
                        Some(i) => {
                            attributes[i].1 = None;
                            i
                        }
                        None => {
                            attributes.push((name.to_owned(), None));
                            attributes.len() - 1
                        }
                    };
                    pending_attribute = Some(index);
                }
                TokenType::AttributeValue => {
                    if let (Some(attributes), Some(i)) =
                        (nodes[curr].attributes.as_mut(), pending_attribute.take())
                    {
                        attributes[i].1 = Some(scanner.token_text().to_owned());
                    }
                }
                _ => {}
            }
            token = scanner.scan();
        }
        while let Some(parent) = nodes[curr].parent {
            nodes[curr].end = text.len();
            nodes[curr].closed = false;
            curr = parent;
        }
        HtmlDocument { nodes }
    }
}
