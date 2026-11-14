use std::collections::HashSet;

use lsp_types::{ColorPresentation, Range, TextEdit};
use serde::{Deserialize, Serialize};

use crate::schema_service::JsonSchemaService;
use crate::{JsonDocument, TextDocument};

/// `Color` of the LSP with `f64` components, as JS (`lsp_types::Color` has `f32`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    #[serde(serialize_with = "js_number")]
    pub red: f64,
    #[serde(serialize_with = "js_number")]
    pub green: f64,
    #[serde(serialize_with = "js_number")]
    pub blue: f64,
    #[serde(serialize_with = "js_number")]
    pub alpha: f64,
}

/// Writes an integer value without a fraction, as `JSON.stringify` does.
fn js_number<S: serde::Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.fract() == 0.0 && v.abs() < 9007199254740992.0 {
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

/// `ColorInformation` with the `f64` `Color`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorInformation {
    pub range: Range,
    pub color: Color,
}

/// `findDocumentColors`. Stops after `limit` string nodes with a color format.
pub fn find_document_colors(
    service: &mut JsonSchemaService,
    document: &TextDocument,
    doc: &JsonDocument,
    limit: Option<usize>,
) -> Vec<ColorInformation> {
    let mut result = Vec::new();
    let Some(schema) = service.get_schema_for_resource(document.uri().as_str(), Some(doc)) else {
        return result;
    };
    // Upstream also stops after the first node if the limit is 0.
    let mut remaining = limit.unwrap_or(usize::MAX).max(1);
    let mut visited = HashSet::new();
    let matching = doc.get_matching_schemas(
        &schema.schemas,
        schema.root,
        None,
        None,
        schema.active_vocabularies.as_ref(),
    );
    for s in matching {
        let node = &doc[s.node];
        let Some(value) = doc.string(s.node) else {
            continue;
        };
        if s.inverted
            || !matches!(
                schema.schemas[s.schema].format.as_deref(),
                Some("color" | "color-hex")
            )
            || !visited.insert(node.offset())
        {
            continue;
        }
        if let Some(color) = color_from_hex(value) {
            result.push(ColorInformation {
                range: document.range(node.offset(), node.end()),
                color,
            });
        }
        remaining -= 1;
        if remaining == 0 {
            break;
        }
    }
    result
}

fn hex_digit(c: u8) -> u32 {
    (c as char).to_digit(16).unwrap_or(0)
}

/// Parses `#RGB`, `#RGBA`, `#RRGGBB` and `#RRGGBBAA`. Invalid digits count as 0, as upstream.
pub fn color_from_hex(text: &str) -> Option<Color> {
    let b = text.as_bytes();
    // Upstream also accepts non-ASCII characters as the digit 0.
    if b.first() != Some(&b'#') || !text.is_ascii() {
        return None;
    }
    let short = |i: usize| f64::from(hex_digit(b[i]) * 0x11) / 255.0;
    let long = |i: usize| f64::from(hex_digit(b[i]) * 0x10 + hex_digit(b[i + 1])) / 255.0;
    match b.len() {
        4 => Some(Color {
            red: short(1),
            green: short(2),
            blue: short(3),
            alpha: 1.0,
        }),
        5 => Some(Color {
            red: short(1),
            green: short(2),
            blue: short(3),
            alpha: short(4),
        }),
        7 => Some(Color {
            red: long(1),
            green: long(3),
            blue: long(5),
            alpha: 1.0,
        }),
        9 => Some(Color {
            red: long(1),
            green: long(3),
            blue: long(5),
            alpha: long(7),
        }),
        _ => None,
    }
}

pub fn get_color_presentations(color: Color, range: Range) -> Vec<ColorPresentation> {
    // `Math.round`: halves round up. `floor(x + 0.5)` is wrong for 0.49999999999999994.
    let byte = |component: f64| {
        let x = component * 255.0;
        let floor = x.floor();
        (if x - floor >= 0.5 { floor + 1.0 } else { floor }) as i64
    };
    let hex = |n: i64| {
        let s = if n < 0 {
            format!("-{:x}", -n)
        } else {
            format!("{n:x}")
        };
        if s.len() == 2 { s } else { format!("0{s}") }
    };
    let mut label = format!(
        "#{}{}{}",
        hex(byte(color.red)),
        hex(byte(color.green)),
        hex(byte(color.blue))
    );
    if color.alpha != 1.0 {
        label.push_str(&hex(byte(color.alpha)));
    }
    vec![ColorPresentation {
        text_edit: Some(TextEdit {
            range,
            new_text: format!("\"{label}\""),
        }),
        label,
        additional_text_edits: None,
    }]
}
