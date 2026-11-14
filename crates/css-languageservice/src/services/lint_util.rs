//! Port of `lintUtil.ts`.

use crate::parser::nodes::{Ast, Field, NodeId};
use crate::utils::parse_float;

/// `Element`: a declaration and its lowercase full property name.
#[derive(Clone, Debug)]
pub struct Element {
    pub full_property_name: String,
    pub node: NodeId,
}

impl Element {
    pub fn new(ast: &Ast, decl: NodeId) -> Self {
        let mut full_property_name = ast.full_property_name(decl);
        if full_property_name.is_ascii() {
            full_property_name.make_ascii_lowercase();
        } else {
            full_property_name = full_property_name.to_lowercase();
        }
        Element {
            full_property_name,
            node: decl,
        }
    }
}

/// `SideState`. Properties are indexes into the property table.
#[derive(Clone, Debug, Default)]
pub struct SideState {
    pub value: bool,
    pub properties: Vec<usize>,
}

/// `BoxModel`. Elements are indexes into the property table.
#[derive(Clone, Debug, Default)]
pub struct BoxModel {
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub top: SideState,
    pub right: SideState,
    pub bottom: SideState,
    pub left: SideState,
}

#[derive(Clone, Copy)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl BoxModel {
    fn side(&mut self, side: Side) -> &mut SideState {
        match side {
            Side::Top => &mut self.top,
            Side::Right => &mut self.right,
            Side::Bottom => &mut self.bottom,
            Side::Left => &mut self.left,
        }
    }

    fn set_side(&mut self, side: Side, value: bool, property: usize) {
        let state = self.side(side);
        state.value = value;
        if value && !state.properties.contains(&property) {
            state.properties.push(property);
        }
    }

    /// `updateModelWithValue`: a side name, or all sides for other names.
    fn update_with_value(&mut self, side: Option<&str>, value: bool, property: usize) {
        let side = match side {
            Some("top") => Side::Top,
            Some("right") => Side::Right,
            Some("bottom") => Side::Bottom,
            Some("left") => Side::Left,
            _ => {
                for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
                    self.set_side(side, value, property);
                }
                return;
            }
        };
        self.set_side(side, value, property);
    }

    fn update_with_list(&mut self, values: &[bool], property: usize) {
        let sides: &[(&str, usize)] = match values.len() {
            1 => &[("all", 0)],
            2 => &[("top", 0), ("bottom", 0), ("right", 1), ("left", 1)],
            3 => &[("top", 0), ("right", 1), ("left", 1), ("bottom", 2)],
            4 => &[("top", 0), ("right", 1), ("bottom", 2), ("left", 3)],
            _ => &[],
        };
        for &(side, i) in sides {
            self.update_with_value(Some(side), values[i], property);
        }
    }
}

fn matches_any(ast: &Ast, value: NodeId, candidates: &[&str]) -> bool {
    candidates.iter().any(|c| ast.matches(value, c))
}

/// `checkLineWidth`: `true` for a non-zero width.
fn check_line_width(ast: &Ast, value: NodeId, allows_keywords: bool) -> bool {
    if allows_keywords && matches_any(ast, value, &["initial", "unset"]) {
        return false;
    }
    parse_float(ast.get_text(value)) != 0.0
}

/// `checkLineStyle`: `true` for a style that draws a border.
fn check_line_style(ast: &Ast, value: NodeId, allows_keywords: bool) -> bool {
    if matches_any(ast, value, &["none", "hidden"]) {
        return false;
    }
    !(allows_keywords && matches_any(ast, value, &["initial", "unset"]))
}

fn check_border_shorthand(ast: &Ast, node: NodeId) -> bool {
    if let Some(value) = ast.only_child(node) {
        return check_line_width(ast, value, true) && check_line_style(ast, value, true);
    }
    ast.children(node)
        .all(|c| check_line_width(ast, c, false) && check_line_style(ast, c, false))
}

/// `calculateBoxModel`.
pub fn calculate_box_model(ast: &Ast, property_table: &[Element]) -> BoxModel {
    let mut model = BoxModel::default();
    for (index, property) in property_table.iter().enumerate() {
        let Some(value) = ast.field(property.node, Field::Value) else {
            continue;
        };
        let width_list = |allows_keywords| -> Vec<bool> {
            ast.children(value)
                .map(|c| check_line_width(ast, c, allows_keywords))
                .collect()
        };
        match property.full_property_name.as_str() {
            "box-sizing" => return BoxModel::default(),
            "width" => model.width = Some(index),
            "height" => model.height = Some(index),
            name => {
                let segments: Vec<&str> = name.split('-').collect();
                let segment = |i: usize| segments.get(i).copied();
                match segments[0] {
                    "border" => match segment(1) {
                        None | Some("top" | "right" | "bottom" | "left") => match segment(2) {
                            None => model.update_with_value(
                                segment(1),
                                check_border_shorthand(ast, value),
                                index,
                            ),
                            Some("width") => model.update_with_value(
                                segment(1),
                                check_line_width(ast, value, false),
                                index,
                            ),
                            Some("style") => model.update_with_value(
                                segment(1),
                                check_line_style(ast, value, true),
                                index,
                            ),
                            _ => {}
                        },
                        Some("width") => model.update_with_list(&width_list(false), index),
                        Some("style") => {
                            let styles: Vec<bool> = ast
                                .children(value)
                                .map(|c| check_line_style(ast, c, true))
                                .collect();
                            model.update_with_list(&styles, index);
                        }
                        _ => {}
                    },
                    "padding" => {
                        if segments.len() == 1 {
                            model.update_with_list(&width_list(true), index);
                        } else {
                            model.update_with_value(
                                segment(1),
                                check_line_width(ast, value, true),
                                index,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    model
}
