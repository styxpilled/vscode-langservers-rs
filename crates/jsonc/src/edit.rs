use std::fmt;

use serde_json::{Value, json};

use crate::format::is_eol;
use crate::{
    Edit, FormattingOptions, Literal, NodeType, ParseOptions, Range, Segment, format, parse_tree,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditError(String);

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

/// Gives the insertion index of a new property from the existing property names.
pub type InsertionIndex<'a> = &'a dyn Fn(&[&str]) -> usize;

#[derive(Default)]
pub struct ModificationOptions<'a> {
    /// If `None`, the new content is not formatted.
    pub formatting_options: Option<FormattingOptions>,
    /// If the path ends with an array index, insert a new item at the index instead of
    /// replacing the item.
    pub is_array_insertion: bool,
    pub get_insertion_index: Option<InsertionIndex<'a>>,
}

/// Computes the edits that set the value at `path`. A value of `None` removes the property or
/// item. Missing parents are created. An array index at or after the end appends an item.
pub fn modify(
    text: &str,
    path: &[Segment],
    value: Option<&Value>,
    options: &ModificationOptions,
) -> Result<Vec<Edit>, EditError> {
    let (tree, _) = parse_tree(text, ParseOptions::default());
    let mut value = value.cloned();
    let mut path = path.to_vec();
    let mut parent = None;
    let mut last_segment = None;
    while let Some(segment) = path.pop() {
        parent = tree
            .as_ref()
            .and_then(|tree| tree.find_node_at_location(tree.root(), &path));
        let wrap = parent.is_none() && value.is_some();
        if wrap {
            value = value.map(|value| match &segment {
                Segment::Property(name) => json!({ name: value }),
                Segment::Index(_) => json!([value]),
            });
        }
        last_segment = Some(segment);
        if !wrap {
            break;
        }
    }

    let (Some(tree), Some(parent)) = (&tree, parent) else {
        let Some(value) = value else {
            return Err(EditError("Can not delete in empty document".to_owned()));
        };
        let (offset, length) = tree.as_ref().map_or((0, 0), |tree| {
            (tree[tree.root()].offset, tree[tree.root()].length)
        });
        let edit = Edit {
            offset,
            length,
            content: value.to_string(),
        };
        return Ok(with_formatting(text, edit, options));
    };

    let node = &tree[parent];
    let children = &node.children;
    let edit = match (node.node_type, last_segment) {
        (NodeType::Object, Some(Segment::Property(name))) => {
            let existing = tree.find_node_at_location(parent, &[Segment::Property(name.clone())]);
            match (existing, value) {
                (Some(existing), None) => {
                    let malformed = || EditError("Malformed AST".to_owned());
                    let property = tree[existing].parent.ok_or_else(malformed)?;
                    let index = children
                        .iter()
                        .position(|&c| c == property)
                        .ok_or_else(malformed)?;
                    let mut remove_end = tree[property].end();
                    let remove_begin = if index > 0 {
                        tree[children[index - 1]].end()
                    } else {
                        if let Some(&next) = children.get(1) {
                            remove_end = tree[next].offset;
                        }
                        node.offset + 1
                    };
                    Edit {
                        offset: remove_begin,
                        length: remove_end - remove_begin,
                        content: String::new(),
                    }
                }
                (Some(existing), Some(value)) => Edit {
                    offset: tree[existing].offset,
                    length: tree[existing].length,
                    content: value.to_string(),
                },
                (None, None) => return Ok(Vec::new()),
                (None, Some(value)) => {
                    let new_property = format!("{}: {}", Value::String(name), value);
                    let index = match options.get_insertion_index {
                        Some(get_insertion_index) => {
                            let names: Vec<&str> = children
                                .iter()
                                .map(|&property| match &tree[tree[property].children[0]].value {
                                    Some(Literal::String(name)) => name.as_str(),
                                    _ => "",
                                })
                                .collect();
                            get_insertion_index(&names).min(children.len())
                        }
                        None => children.len(),
                    };
                    if index > 0 {
                        Edit {
                            offset: tree[children[index - 1]].end(),
                            length: 0,
                            content: format!(",{new_property}"),
                        }
                    } else if children.is_empty() {
                        Edit {
                            offset: node.offset + 1,
                            length: 0,
                            content: new_property,
                        }
                    } else {
                        Edit {
                            offset: node.offset + 1,
                            length: 0,
                            content: format!("{new_property},"),
                        }
                    }
                }
            }
        }
        (NodeType::Array, Some(Segment::Index(index))) => match value {
            None => {
                if index >= children.len() {
                    return Err(EditError(format!(
                        "Can not remove Array index {index} as length is not sufficient"
                    )));
                }
                if children.len() == 1 {
                    Edit {
                        offset: node.offset + 1,
                        length: node.length.saturating_sub(2),
                        content: String::new(),
                    }
                } else if index == children.len() - 1 {
                    let offset = tree[children[index - 1]].end();
                    Edit {
                        offset,
                        length: (node.end() - 1).saturating_sub(offset),
                        content: String::new(),
                    }
                } else {
                    let offset = tree[children[index]].offset;
                    Edit {
                        offset,
                        length: tree[children[index + 1]].offset - offset,
                        content: String::new(),
                    }
                }
            }
            Some(value) => {
                let new_item = value.to_string();
                if !options.is_array_insertion && index < children.len() {
                    let item = &tree[children[index]];
                    Edit {
                        offset: item.offset,
                        length: item.length,
                        content: new_item,
                    }
                } else if children.is_empty() || index == 0 {
                    Edit {
                        offset: node.offset + 1,
                        length: 0,
                        content: if children.is_empty() {
                            new_item
                        } else {
                            format!("{new_item},")
                        },
                    }
                } else {
                    let previous = &tree[children[index.min(children.len()) - 1]];
                    Edit {
                        offset: previous.end(),
                        length: 0,
                        content: format!(",{new_item}"),
                    }
                }
            }
        },
        (node_type, segment) => {
            let target = match segment {
                Some(Segment::Index(_)) => "index",
                _ => "property",
            };
            return Err(EditError(format!(
                "Can not add {target} to parent of type {node_type:?}"
            )));
        }
    };
    Ok(with_formatting(text, edit, options))
}

/// Applies the edit and formats the changed lines. The result is one edit.
fn with_formatting(text: &str, edit: Edit, options: &ModificationOptions) -> Vec<Edit> {
    let Some(formatting_options) = &options.formatting_options else {
        return vec![edit];
    };
    let mut new_text = apply_edit(text, &edit);

    let mut begin = edit.offset;
    let mut end = edit.offset + edit.content.len();
    if edit.length == 0 || edit.content.is_empty() {
        while begin > 0 && !is_eol(&new_text, begin - 1) {
            begin -= 1;
        }
        while end < new_text.len() && !is_eol(&new_text, end) {
            end += 1;
        }
    }

    let range = Range {
        offset: begin,
        length: end - begin,
    };
    let formatting_options = FormattingOptions {
        keep_lines: false,
        ..formatting_options.clone()
    };
    let edits = format(&new_text, Some(range), &formatting_options);

    for edit in edits.iter().rev() {
        new_text = apply_edit(&new_text, edit);
        begin = begin.min(edit.offset);
        end = end.max(edit.offset + edit.length);
        end = end + edit.content.len() - edit.length;
    }
    let length = text.len() - (new_text.len() - end) - begin;
    vec![Edit {
        offset: begin,
        length,
        content: new_text[begin..end].to_owned(),
    }]
}

fn apply_edit(text: &str, edit: &Edit) -> String {
    let mut result = String::with_capacity(text.len() + edit.content.len());
    result.push_str(&text[..edit.offset]);
    result.push_str(&edit.content);
    result.push_str(&text[edit.offset + edit.length..]);
    result
}

/// Applies edits that refer to the original text. Edits must not overlap.
pub fn apply_edits(text: &str, edits: &[Edit]) -> Result<String, EditError> {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|edit| (edit.offset, edit.length));
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    for edit in sorted {
        let end = edit.offset + edit.length;
        if edit.offset < position || end > text.len() {
            return Err(EditError("Overlapping edit".to_owned()));
        }
        result.push_str(&text[position..edit.offset]);
        result.push_str(&edit.content);
        position = end;
    }
    result.push_str(&text[position..]);
    Ok(result)
}
