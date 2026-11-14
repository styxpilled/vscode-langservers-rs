//! Port of `utils/sort.ts` and `utils/propertyTree.ts`.
//!
//! Line columns are UTF-8 byte offsets. Upstream uses UTF-16 units.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::LazyLock;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};
use jsonc::{Scanner, SyntaxKind};
use lsp_types::{Position, Range, TextEdit};

use crate::{FormattingOptions, TextDocument};

/// Upstream sorts with `String.prototype.localeCompare`, which uses ICU collation.
static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    Collator::try_new(Default::default(), CollatorOptions::default())
        .expect("compiled collation data")
});

/// Sorts the properties of all objects. The result is one edit that replaces the document.
pub fn sort(document: &TextDocument, options: &FormattingOptions) -> Vec<TextEdit> {
    let options = jsonc::FormattingOptions {
        tab_size: options.tab_size as usize,
        insert_spaces: options.insert_spaces,
        insert_final_newline: options.insert_final_newline,
        eol: "\n".to_owned(),
        // The properties must be on separate lines for the sort.
        keep_lines: false,
    };
    let format = |text: &str| {
        let edits = jsonc::format(text, None, &options);
        jsonc::apply_edits(text, &edits).ok()
    };
    let Some(formatted) = format(document.text()) else {
        return Vec::new();
    };
    let Some(mut tree) = find_property_tree(&formatted) else {
        return Vec::new();
    };
    let sorted = sort_document(&formatted, &mut tree);
    let Some(result) = format(&sorted) else {
        return Vec::new();
    };
    vec![TextEdit {
        range: Range::new(
            Position::new(0, 0),
            document.position_at(document.text().len()),
        ),
        new_text: result,
    }]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Object,
    Array,
}

#[derive(Default)]
struct Property {
    name: String,
    beginning_line: Option<i64>,
    end_line: Option<i64>,
    parent: Option<usize>,
    comma_index: Option<i64>,
    comma_line: Option<i64>,
    line_where_to_add_comma: Option<i64>,
    index_where_to_add_comma: Option<i64>,
    container: Option<Container>,
    children: Vec<usize>,
    last_property: bool,
    no_key_name: bool,
}

struct PropertyTree {
    nodes: Vec<Property>,
}

const ROOT: usize = 0;

impl PropertyTree {
    fn add_child(&mut self, parent: usize, mut child: Property) -> usize {
        let id = self.nodes.len();
        child.parent = Some(parent);
        let index = if child.no_key_name {
            self.nodes[parent].children.len()
        } else {
            self.insertion_index(parent, &child.name)
        };
        self.nodes.push(child);
        self.nodes[parent].children.insert(index, id);
        id
    }

    /// Binary search on the lowercase names, as upstream does.
    fn insertion_index(&self, parent: usize, name: &str) -> usize {
        let children = &self.nodes[parent].children;
        if children.is_empty() {
            return 0;
        }
        let lower = |id: usize| self.nodes[id].name.to_lowercase();
        let name = name.to_lowercase();
        if compare_utf16(&name, &lower(children[0])) == Ordering::Less {
            return 0;
        }
        if compare_utf16(&name, &lower(children[children.len() - 1])) == Ordering::Greater {
            return children.len();
        }
        let (mut m, mut n) = (0isize, children.len() as isize - 1);
        while m <= n {
            let k = (n + m) >> 1;
            match compare_utf16(&name, &lower(children[k as usize])) {
                Ordering::Greater => m = k + 1,
                Ordering::Less => n = k - 1,
                Ordering::Equal => return k as usize,
            }
        }
        m as usize
    }
}

/// JS string comparison: by UTF-16 code units.
fn compare_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn is_trivia_or_comment(token: SyntaxKind) -> bool {
    matches!(
        token,
        SyntaxKind::LineBreakTrivia
            | SyntaxKind::Trivia
            | SyntaxKind::LineCommentTrivia
            | SyntaxKind::BlockCommentTrivia
    )
}

fn is_close(token: Option<SyntaxKind>) -> bool {
    matches!(
        token,
        Some(SyntaxKind::CloseBraceToken | SyntaxKind::CloseBracketToken)
    )
}

/// Returns `None` where upstream fails on an undefined property.
fn find_property_tree(text: &str) -> Option<PropertyTree> {
    let line_offsets = Lines::new(text);
    let mut scanner = Scanner::new(text, false);
    let mut tree = PropertyTree {
        nodes: vec![Property::default()],
    };
    let mut current_tree = Some(ROOT);
    let mut current_property = Some(ROOT);
    let mut last_property = Some(ROOT);

    let mut last_token_line = 0i64;
    let mut characters_on_previous_lines = 0i64;
    let mut last_token: Option<SyntaxKind> = None;
    let mut second_to_last_token: Option<SyntaxKind> = None;
    let mut line_of_last_token = -1i64;
    let mut end_index_of_last_token = -1i64;
    let mut beginning_line = 0i64;
    let mut containers: Vec<Container> = Vec::new();
    let mut update_last_property_end_line = false;
    let mut update_beginning_line = false;

    loop {
        let token = scanner.scan();
        if token == SyntaxKind::EOF {
            break;
        }
        let token_line = scanner.token_start_line() as i64;
        let token_offset = scanner.token_offset() as i64;
        let in_container = |c| containers.last() == Some(&c);

        // A block comment that starts on the line of a property comma belongs to the property.
        if update_last_property_end_line
            && !is_trivia_or_comment(token)
            && tree.nodes[current_property?].end_line.is_none()
        {
            let target = if is_close(second_to_last_token) {
                last_property?
            } else {
                current_property?
            };
            tree.nodes[target].end_line = Some(token_line - 1);
            beginning_line = token_line;
            update_last_property_end_line = false;
        }

        // A block comment after an open brace or bracket belongs to the brace or bracket.
        if update_beginning_line && !is_trivia_or_comment(token) {
            beginning_line = token_line;
            update_beginning_line = false;
        }

        if token_line != last_token_line {
            for line in last_token_line..token_line {
                characters_on_previous_lines += line_offsets.line_length(line) as i64;
            }
            last_token_line = token_line;
        }

        match token {
            SyntaxKind::StringLiteral => {
                // A string after `{`, or after `,` in an object, is a property name.
                if last_token.is_none()
                    || last_token == Some(SyntaxKind::OpenBraceToken)
                    || (last_token == Some(SyntaxKind::CommaToken)
                        && in_container(Container::Object))
                {
                    let child = Property {
                        name: scanner.token_value().to_owned(),
                        beginning_line: Some(beginning_line),
                        ..Default::default()
                    };
                    last_property = current_property;
                    current_property = Some(tree.add_child(current_tree?, child));
                }
            }
            SyntaxKind::OpenBracketToken => {
                if tree.nodes[ROOT].beginning_line.is_none() {
                    tree.nodes[ROOT].beginning_line = Some(token_line);
                }
                if in_container(Container::Object) {
                    current_tree = current_property;
                } else if in_container(Container::Array) {
                    let child = Property {
                        beginning_line: Some(beginning_line),
                        no_key_name: true,
                        ..Default::default()
                    };
                    last_property = current_property;
                    current_property = Some(tree.add_child(current_tree?, child));
                    current_tree = current_property;
                }
                containers.push(Container::Array);
                tree.nodes[current_property?].container = Some(Container::Array);
                beginning_line = token_line + 1;
            }
            SyntaxKind::OpenBraceToken => {
                if tree.nodes[ROOT].beginning_line.is_none() {
                    tree.nodes[ROOT].beginning_line = Some(token_line);
                } else if in_container(Container::Array) {
                    let child = Property {
                        beginning_line: Some(beginning_line),
                        no_key_name: true,
                        ..Default::default()
                    };
                    last_property = current_property;
                    current_property = Some(tree.add_child(current_tree?, child));
                }
                tree.nodes[current_property?].container = Some(Container::Object);
                containers.push(Container::Object);
                current_tree = current_property;
                beginning_line = token_line + 1;
            }
            SyntaxKind::CloseBracketToken => {
                let end_line = token_line;
                containers.pop();
                let property = current_property?;
                if tree.nodes[property].end_line.is_none() && is_close(last_token) {
                    let node = &mut tree.nodes[property];
                    node.end_line = Some(end_line - 1);
                    node.last_property = true;
                    node.line_where_to_add_comma = Some(line_of_last_token);
                    node.index_where_to_add_comma = Some(end_index_of_last_token);
                    last_property = current_property;
                    current_property = node.parent;
                    current_tree = current_property;
                }
                tree.nodes[ROOT].end_line = Some(end_line);
                beginning_line = end_line + 1;
            }
            SyntaxKind::CloseBraceToken => {
                let end_line = token_line;
                containers.pop();
                if last_token != Some(SyntaxKind::OpenBraceToken) {
                    let node = &mut tree.nodes[current_property?];
                    if node.end_line.is_none() {
                        node.end_line = Some(end_line - 1);
                        node.last_property = true;
                        node.line_where_to_add_comma = Some(line_of_last_token);
                        node.index_where_to_add_comma = Some(end_index_of_last_token);
                    }
                    last_property = current_property;
                    current_property = node.parent;
                    current_tree = current_property;
                }
                tree.nodes[ROOT].end_line = Some(token_line);
                beginning_line = end_line + 1;
            }
            SyntaxKind::CommaToken => {
                let end_line = token_line;
                let node = &mut tree.nodes[current_property?];
                if node.end_line.is_none()
                    && (in_container(Container::Object)
                        || (in_container(Container::Array) && is_close(last_token)))
                {
                    node.end_line = Some(end_line);
                    node.comma_index = Some(token_offset - characters_on_previous_lines);
                    node.comma_line = Some(end_line);
                }
                if is_close(last_token) {
                    last_property = current_property;
                    current_property = node.parent;
                    current_tree = current_property;
                }
                beginning_line = end_line + 1;
            }
            SyntaxKind::BlockCommentTrivia => {
                let owns_comment = (in_container(Container::Array)
                    && is_close(second_to_last_token))
                    || in_container(Container::Object);
                if last_token == Some(SyntaxKind::CommaToken)
                    && line_of_last_token == token_line
                    && owns_comment
                {
                    tree.nodes[current_property?].end_line = None;
                    update_last_property_end_line = true;
                }
                if matches!(
                    last_token,
                    Some(SyntaxKind::OpenBraceToken | SyntaxKind::OpenBracketToken)
                ) && line_of_last_token == token_line
                {
                    update_beginning_line = true;
                }
            }
            _ => {}
        }

        if !is_trivia_or_comment(token) {
            second_to_last_token = last_token;
            last_token = Some(token);
            line_of_last_token = token_line;
            end_index_of_last_token =
                token_offset + scanner.token_length() as i64 - characters_on_previous_lines;
        }
    }
    Some(tree)
}

/// Line offsets with the semantics of `TextDocument.offsetAt`, for byte columns.
struct Lines<'a> {
    text: &'a str,
    offsets: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut offsets = vec![0];
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            if matches!(bytes[i], b'\r' | b'\n') {
                offsets.push(i + 1);
            }
            i += 1;
        }
        Self { text, offsets }
    }

    fn line_offset(&self, line: i64) -> usize {
        if line < 0 {
            0
        } else {
            self.offsets
                .get(line as usize)
                .copied()
                .unwrap_or(self.text.len())
        }
    }

    fn line_length(&self, line: i64) -> usize {
        self.line_offset(line + 1) - self.line_offset(line)
    }

    fn offset_at(&self, line: i64, column: i64) -> usize {
        if line >= self.offsets.len() as i64 {
            return self.text.len();
        }
        let line_offset = self.line_offset(line);
        if column <= 0 {
            return line_offset;
        }
        let next_line_offset = self.line_offset(line + 1);
        let mut offset = (line_offset + column as usize).min(next_line_offset);
        let bytes = self.text.as_bytes();
        while offset > line_offset && matches!(bytes[offset - 1], b'\r' | b'\n') {
            offset -= 1;
        }
        offset
    }
}

/// A reversed range is swapped, as upstream `TextDocument.update` does.
fn replace(text: &str, a: usize, b: usize, new_text: &str) -> String {
    let (start, end) = (a.min(b), a.max(b));
    let mut result = String::with_capacity(text.len() + new_text.len());
    result.push_str(&text[..start]);
    result.push_str(new_text);
    result.push_str(&text[end..]);
    result
}

fn sort_document(text: &str, tree: &mut PropertyTree) -> String {
    if tree.nodes[ROOT].children.is_empty() {
        return text.to_owned();
    }
    let lines = Lines::new(text);
    // Upstream replaces line ranges in the text. A line list gives the same result without a
    // copy of the text for each property.
    let mut sorted = split_lines(text);
    let mut queue: VecDeque<(i64, Vec<usize>)> = VecDeque::new();
    let root_beginning = tree.nodes[ROOT].beginning_line.unwrap_or(0);
    update_sorting_queue(&mut queue, tree, ROOT, root_beginning);

    while let Some((mut beginning_line, properties)) = queue.pop_front() {
        let count = properties.len();
        for (i, &id) in properties.iter().enumerate() {
            let node = &tree.nodes[id];
            let begin = node.beginning_line.unwrap_or(0);
            let end = node.end_line.unwrap_or(0);
            // Malformed input can give a reversed range. JS `substring` swaps it.
            let (a, b) = (lines.line_offset(begin), lines.line_offset(end + 1));
            let mut content = text[a.min(b)..a.max(b)].to_owned();

            if node.last_property && i != count - 1 {
                let content_lines = Lines::new(&content);
                let line = node.line_where_to_add_comma.unwrap_or(0) - begin;
                let at = content_lines.offset_at(line, node.index_where_to_add_comma.unwrap_or(0));
                content = replace(&content, at, at, ",");
            } else if !node.last_property && i == count - 1 {
                let content_lines = Lines::new(&content);
                let line = node.comma_line.unwrap_or(0) - begin;
                let column = node.comma_index.unwrap_or(0);
                let start = content_lines.offset_at(line, column);
                let end = content_lines.offset_at(line, column + 1);
                content = replace(&content, start, end, "");
            }

            let length = end - begin + 1;
            let clamp = |line: i64| line.clamp(0, sorted.len() as i64) as usize;
            let (a, b) = (clamp(beginning_line), clamp(beginning_line + length));
            let new_lines: Vec<String> = split_lines(&content);
            sorted.splice(a.min(b)..a.max(b), new_lines);
            update_sorting_queue(&mut queue, tree, id, beginning_line);
            beginning_line += length;
        }
    }
    sorted.concat()
}

/// Splits after each line break, with the same line breaks as [`Lines`].
fn split_lines(text: &str) -> Vec<String> {
    let lines = Lines::new(text);
    let mut ends: Vec<usize> = lines.offsets[1..].to_vec();
    if ends.last() != Some(&text.len()) {
        ends.push(text.len());
    }
    let mut start = 0;
    ends.into_iter()
        .map(|end| {
            let line = text[start..end].to_owned();
            start = end;
            line
        })
        .collect()
}

fn min_child_beginning(tree: &PropertyTree, id: usize) -> Option<i64> {
    tree.nodes[id]
        .children
        .iter()
        .filter_map(|&c| tree.nodes[c].beginning_line)
        .min()
}

fn update_sorting_queue(
    queue: &mut VecDeque<(i64, Vec<usize>)>,
    tree: &mut PropertyTree,
    id: usize,
    beginning_line: i64,
) {
    if tree.nodes[id].children.is_empty() {
        return;
    }
    match tree.nodes[id].container {
        Some(Container::Object) => {
            let min = min_child_beginning(tree, id).unwrap_or(0);
            let diff = min - tree.nodes[id].beginning_line.unwrap_or(0);
            let mut children = std::mem::take(&mut tree.nodes[id].children);
            children.sort_by(|&a, &b| COLLATOR.compare(&tree.nodes[a].name, &tree.nodes[b].name));
            tree.nodes[id].children = children.clone();
            queue.push_back((beginning_line + diff, children));
        }
        Some(Container::Array) => update_sorting_queue_for_array(queue, tree, id, beginning_line),
        None => {}
    }
}

/// The properties of objects in arrays keep the order of insertion, which is the order of the
/// lowercase names. Upstream does the same.
fn update_sorting_queue_for_array(
    queue: &mut VecDeque<(i64, Vec<usize>)>,
    tree: &mut PropertyTree,
    id: usize,
    beginning_line: i64,
) {
    let array_beginning = tree.nodes[id].beginning_line.unwrap_or(0);
    for sub in tree.nodes[id].children.clone() {
        let sub_beginning = tree.nodes[sub].beginning_line.unwrap_or(0);
        let offset = beginning_line + sub_beginning - array_beginning;
        match tree.nodes[sub].container {
            Some(Container::Object) => {
                if let Some(min) = min_child_beginning(tree, sub) {
                    queue.push_back((
                        offset + min - sub_beginning,
                        tree.nodes[sub].children.clone(),
                    ));
                }
            }
            Some(Container::Array) => update_sorting_queue_for_array(queue, tree, sub, offset),
            None => {}
        }
    }
}
