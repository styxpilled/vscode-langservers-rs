//! Port of `colors.ts`.

use foldhash::HashMap;
use std::sync::LazyLock;

use crate::parser::nodes::{Ast, Children, Field, Kind, NodeId, NodeType};

pub use super::color_data::{COLOR_FUNCTIONS, COLOR_KEYWORDS, COLORS};

/// A color with components in `0..=1` (`Color` of the LSP, with `f64`).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsla {
    pub h: f64,
    pub s: f64,
    pub l: f64,
    pub a: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hwba {
    pub h: f64,
    pub w: f64,
    pub b: f64,
    pub a: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xyz {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub alpha: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
    pub alpha: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
    pub alpha: f64,
}

/// An entry of `colorFunctions`.
#[derive(Clone, Copy, Debug)]
pub struct ColorFunction {
    pub label: &'static str,
    pub func: &'static str,
    pub insert_text: &'static str,
    pub desc: &'static str,
}

static COLOR_MAP: LazyLock<HashMap<&str, &str>> =
    LazyLock::new(|| COLORS.iter().copied().collect());

/// `v` rounded to `digits` decimal places.
fn round_to(v: f64, digits: i32) -> f64 {
    let p = 10f64.powi(digits);
    (v * p).round() / p
}

/// The length of the match of `[-+]?[0-9]*\.?[0-9]+` at the start of `s`.
fn number_prefix(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut i = usize::from(matches!(b.first(), Some(b'-' | b'+')));
    let int = digits(i);
    i += int;
    if b.get(i) == Some(&b'.') {
        let frac = digits(i + 1);
        if frac > 0 {
            return Some(i + 1 + frac);
        }
    }
    (int > 0).then_some(i)
}

/// `getNumericValue`. `None` where upstream throws.
fn get_numeric_value(ast: &Ast, node: NodeId, factor: f64, lower: f64, upper: f64) -> Option<f64> {
    let val = ast.get_text(node);
    let n = number_prefix(val)?;
    let factor = match &val[n..] {
        "" => factor,
        "%" => 100.0,
        _ => return None,
    };
    let result = val[..n].parse::<f64>().ok()? / factor;
    (result >= lower && result <= upper).then_some(result)
}

const DEGREES_PER_CIRCLE: f64 = 360.0;
const GRAD_TO_DEGREE_FACTOR: f64 = 0.9;
const RADIANS_TO_DEGREES_FACTOR: f64 = DEGREES_PER_CIRCLE / 2.0 / std::f64::consts::PI;
const DEGREES_TO_RADIANS_FACTOR: f64 = std::f64::consts::PI / 180.0;

/// `getAngle`. `None` where upstream throws.
fn get_angle(ast: &Ast, node: NodeId) -> Option<f64> {
    let text = ast.get_text(node);
    if text == "none" {
        return Some(0.0);
    }
    let n = number_prefix(text)?;
    let value: f64 = text[..n].parse().ok()?;
    Some(match text[n..].to_ascii_lowercase().as_str() {
        "" | "deg" => value % DEGREES_PER_CIRCLE,
        "grad" => (value * GRAD_TO_DEGREE_FACTOR) % DEGREES_PER_CIRCLE,
        "rad" => (value * RADIANS_TO_DEGREES_FACTOR) % DEGREES_PER_CIRCLE,
        "turn" => (value * DEGREES_PER_CIRCLE) % DEGREES_PER_CIRCLE,
        _ => return None,
    })
}

/// `Function.getName()`.
fn function_name(ast: &Ast, node: NodeId) -> &str {
    ast.field(node, Field::Identifier)
        .map_or("", |id| ast.get_text(id))
}

fn function_arguments(ast: &Ast, node: NodeId) -> Children<'_> {
    ast.list_children(node, Field::Arguments)
}

/// `isColorConstructor`.
pub fn is_color_constructor(ast: &Ast, node: NodeId) -> bool {
    matches!(
        function_name(ast, node).to_ascii_lowercase().as_str(),
        "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch"
    )
}

/// `isColorString`: a hex color, a color name or a color keyword.
pub fn is_color_string(s: &str) -> bool {
    let is_hex = s.strip_prefix('#').is_some_and(|h| {
        matches!(h.len(), 3 | 4 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit())
    });
    is_hex
        || COLORS.iter().any(|(n, _)| s.eq_ignore_ascii_case(n))
        || COLOR_KEYWORDS
            .iter()
            .any(|(n, _)| s.eq_ignore_ascii_case(n))
}

/// The hex value of a lowercase color name.
fn named_color(name: &str) -> Option<&'static str> {
    COLOR_MAP.get(name).copied()
}

/// `isColorValue`: a hex color, a color function or a color name.
pub fn is_color_value(ast: &Ast, node: NodeId) -> bool {
    match ast.node_type(node) {
        NodeType::HexColorValue => color_from_hex(ast.get_text(node)).is_some(),
        NodeType::Function => is_color_constructor(ast, node),
        NodeType::Identifier => {
            if ast
                .parent(node)
                .is_some_and(|p| ast.node_type(p) != NodeType::Term)
            {
                return false;
            }
            let candidate = ast.get_text(node).to_lowercase();
            candidate != "none" && named_color(&candidate).is_some()
        }
        _ => false,
    }
}

/// `hexDigit`, for a UTF-16 code unit.
pub fn hex_digit(char_code: u32) -> u32 {
    let mut c = char_code;
    if c < u32::from(b'0') {
        return 0;
    }
    if c <= u32::from(b'9') {
        return c - u32::from(b'0');
    }
    if c < u32::from(b'a') {
        c += u32::from(b'a' - b'A');
    }
    if (u32::from(b'a')..=u32::from(b'f')).contains(&c) {
        return c - u32::from(b'a') + 10;
    }
    0
}

pub fn color_from_hex(text: &str) -> Option<Color> {
    if !text.starts_with('#') {
        return None;
    }
    let t: Vec<u32> = text.encode_utf16().map(u32::from).collect();
    let short = |i: usize| f64::from(hex_digit(t[i]) * 0x11) / 255.0;
    let long = |i: usize| f64::from(hex_digit(t[i]) * 0x10 + hex_digit(t[i + 1])) / 255.0;
    match t.len() {
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

pub fn color_from_256_rgb(red: f64, green: f64, blue: f64, alpha: f64) -> Color {
    Color {
        red: red / 255.0,
        green: green / 255.0,
        blue: blue / 255.0,
        alpha,
    }
}

pub fn color_from_hsl(hue: f64, sat: f64, light: f64, alpha: f64) -> Color {
    let hue = hue / 60.0;
    if sat == 0.0 {
        return Color {
            red: light,
            green: light,
            blue: light,
            alpha,
        };
    }
    let hue_to_rgb = |t1: f64, t2: f64, mut hue: f64| {
        while hue < 0.0 {
            hue += 6.0;
        }
        while hue >= 6.0 {
            hue -= 6.0;
        }
        if hue < 1.0 {
            return (t2 - t1) * hue + t1;
        }
        if hue < 3.0 {
            return t2;
        }
        if hue < 4.0 {
            return (t2 - t1) * (4.0 - hue) + t1;
        }
        t1
    };
    let t2 = if light <= 0.5 {
        light * (sat + 1.0)
    } else {
        light + sat - (light * sat)
    };
    let t1 = light * 2.0 - t2;
    Color {
        red: hue_to_rgb(t1, t2, hue + 2.0),
        green: hue_to_rgb(t1, t2, hue),
        blue: hue_to_rgb(t1, t2, hue - 2.0),
        alpha,
    }
}

pub fn hsl_from_color(rgba: Color) -> Hsla {
    let (r, g, b, a) = (rgba.red, rgba.green, rgba.blue, rgba.alpha);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let mut h = 0.0;
    let mut s = 0.0;
    let l = (min + max) / 2.0;
    let chroma = max - min;
    if chroma > 0.0 {
        s = (if l <= 0.5 {
            chroma / (2.0 * l)
        } else {
            chroma / (2.0 - (2.0 * l))
        })
        .min(1.0);
        if max == r {
            h = (g - b) / chroma + if g < b { 6.0 } else { 0.0 };
        } else if max == g {
            h = (b - r) / chroma + 2.0;
        } else if max == b {
            h = (r - g) / chroma + 4.0;
        }
        h *= 60.0;
        h = h.round();
    }
    Hsla { h, s, l, a }
}

pub fn color_from_hwb(hue: f64, white: f64, black: f64, alpha: f64) -> Color {
    if white + black >= 1.0 {
        let gray = white / (white + black);
        return Color {
            red: gray,
            green: gray,
            blue: gray,
            alpha,
        };
    }
    let rgb = color_from_hsl(hue, 1.0, 0.5, alpha);
    let mix = |c: f64| c * (1.0 - white - black) + white;
    Color {
        red: mix(rgb.red),
        green: mix(rgb.green),
        blue: mix(rgb.blue),
        alpha,
    }
}

pub fn hwb_from_color(rgba: Color) -> Hwba {
    let hsl = hsl_from_color(rgba);
    let white = rgba.red.min(rgba.green).min(rgba.blue);
    let black = 1.0 - rgba.red.max(rgba.green).max(rgba.blue);
    Hwba {
        h: hsl.h,
        w: white,
        b: black,
        a: hsl.a,
    }
}

pub fn xyz_from_lab(lab: Lab) -> Xyz {
    let y = (lab.l + 16.0) / 116.0;
    let x = (lab.a / 500.0) + y;
    let z = y - (lab.b / 200.0);
    let f = |v: f64| {
        let pow = v * v * v;
        if pow > 0.008856 {
            pow
        } else {
            (v - 16.0 / 116.0) / 7.787
        }
    };
    Xyz {
        x: f(x) * 95.047,
        y: f(y) * 100.0,
        z: f(z) * 108.883,
        alpha: lab.alpha,
    }
}

pub fn xyz_from_oklab(lab: Lab) -> Xyz {
    let l = lab.l + 0.396_337_777_4 * lab.a + 0.215_803_757_3 * lab.b;
    let m = lab.l - 0.105_561_345_8 * lab.a - 0.063_854_172_8 * lab.b;
    let s = lab.l - 0.089_484_177_5 * lab.a - 1.291_485_548 * lab.b;
    let l3 = l.powf(3.0);
    let m3 = m.powf(3.0);
    let s3 = s.powf(3.0);
    let x = 1.227_013_851_1 * l3 - 0.557_799_980_7 * m3 + 0.281_256_149 * s3;
    let y = -0.040_580_178_4 * l3 + 1.112_256_869_6 * m3 - 0.071_676_678_7 * s3;
    let z = -0.076_381_284_5 * l3 - 0.421_481_978_4 * m3 + 1.586_163_220_4 * s3;
    Xyz {
        x: x * 100.0,
        y: y * 100.0,
        z: z * 100.0,
        alpha: lab.alpha,
    }
}

pub fn xyz_to_rgb(xyz: Xyz) -> Color {
    let x = xyz.x / 100.0;
    let y = xyz.y / 100.0;
    let z = xyz.z / 100.0;
    let r = 3.2406254773200533 * x - 1.5372079722103187 * y - 0.4986285986982479 * z;
    let g = -0.9689307147293197 * x + 1.8757560608852415 * y + 0.041517523842953964 * z;
    let b = 0.055710120445510616 * x + -0.2040210505984867 * y + 1.0569959422543882 * z;
    let compand = |c: f64| {
        if c <= 0.0031308 {
            12.92 * c
        } else {
            (1.055 * c.powf(1.0 / 2.4) - 0.055).min(1.0)
        }
    };
    Color {
        red: (compand(r) * 255.0).round(),
        green: (compand(g) * 255.0).round(),
        blue: (compand(b) * 255.0).round(),
        alpha: xyz.alpha,
    }
}

#[allow(non_snake_case)]
pub fn RGBtoXYZ(rgba: Color) -> Xyz {
    let linear = |c: f64| {
        if c > 0.04045 {
            ((c + 0.055) / 1.055).powf(2.4)
        } else {
            c / 12.92
        }
    };
    let r = linear(rgba.red) * 100.0;
    let g = linear(rgba.green) * 100.0;
    let b = linear(rgba.blue) * 100.0;
    Xyz {
        x: r * 0.4124 + g * 0.3576 + b * 0.1805,
        y: r * 0.2126 + g * 0.7152 + b * 0.0722,
        z: r * 0.0193 + g * 0.1192 + b * 0.9505,
        alpha: rgba.alpha,
    }
}

#[allow(non_snake_case)]
pub fn XYZtoLAB(xyz: Xyz, round: bool) -> Lab {
    let f = |v: f64| {
        if v > 0.008856 {
            v.powf(1.0 / 3.0)
        } else {
            (7.787 * v) + (16.0 / 116.0)
        }
    };
    let x = f(xyz.x / 95.047);
    let y = f(xyz.y / 100.000);
    let z = f(xyz.z / 108.883);
    let l = (116.0 * y) - 16.0;
    let a = 500.0 * (x - y);
    let b = 200.0 * (y - z);
    let r = |v| if round { round_to(v, 2) } else { v };
    Lab {
        l: r(l),
        a: r(a),
        b: r(b),
        alpha: xyz.alpha,
    }
}

#[allow(non_snake_case)]
pub fn XYZtoOKLAB(xyz: Xyz, round: bool) -> Lab {
    let x = xyz.x / 100.0;
    let y = xyz.y / 100.0;
    let z = xyz.z / 100.0;
    let l = 0.818_933_010_1 * x + 0.361_866_742_4 * y - 0.128_859_713_7 * z;
    let m = 0.032_984_543_6 * x + 0.929_311_871_5 * y + 0.036_145_638_7 * z;
    let s = 0.048_200_301_8 * x + 0.264_366_269_1 * y + 0.633_851_707 * z;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    let big_l = 0.210_454_255_3 * l_ + 0.793_617_785 * m_ - 0.004_072_046_8 * s_;
    let a = 1.977_998_495_1 * l_ - 2.428_592_205 * m_ + 0.450_593_709_9 * s_;
    let b = 0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766 * s_;
    let r = |v| if round { round_to(v, 5) } else { v };
    Lab {
        l: r(big_l),
        a: r(a),
        b: r(b),
        alpha: xyz.alpha,
    }
}

pub fn lab_from_color(rgba: Color, round: bool) -> Lab {
    XYZtoLAB(RGBtoXYZ(rgba), round)
}

pub fn oklab_from_color(rgba: Color, round: bool) -> Lab {
    let lab = XYZtoOKLAB(RGBtoXYZ(rgba), round);
    Lab {
        l: lab.l * 100.0,
        ..lab
    }
}

fn lab_to_lch(lab: Lab) -> Lch {
    let c = (lab.a.powf(2.0) + lab.b.powf(2.0)).sqrt();
    let mut h = lab.b.atan2(lab.a) * RADIANS_TO_DEGREES_FACTOR;
    while h < 0.0 {
        h += 360.0;
    }
    Lch {
        l: lab.l,
        c,
        h,
        alpha: lab.alpha,
    }
}

pub fn lch_from_color(rgba: Color) -> Lch {
    let lch = lab_to_lch(lab_from_color(rgba, false));
    Lch {
        l: round_to(lch.l, 2),
        c: round_to(lch.c, 2),
        h: round_to(lch.h, 2),
        alpha: lch.alpha,
    }
}

pub fn oklch_from_color(rgba: Color) -> Lch {
    let lch = lab_to_lch(oklab_from_color(rgba, false));
    Lch {
        l: round_to(lch.l, 3),
        c: round_to(lch.c, 5),
        h: round_to(lch.h, 3),
        alpha: lch.alpha,
    }
}

fn lab_to_color(lab: Lab, xyz_converter: fn(Lab) -> Xyz) -> Color {
    let rgb = xyz_to_rgb(xyz_converter(lab));
    let clamp = |c: f64| (if c >= 0.0 { c.min(255.0) } else { 0.0 }) / 255.0;
    Color {
        red: clamp(rgb.red),
        green: clamp(rgb.green),
        blue: clamp(rgb.blue),
        alpha: lab.alpha,
    }
}

pub fn color_from_lab(l: f64, a: f64, b: f64, alpha: f64) -> Color {
    let lab = Lab { l, a, b, alpha };
    lab_to_color(lab, xyz_from_lab)
}

pub fn color_from_oklab(l: f64, a: f64, b: f64, alpha: f64) -> Color {
    let lab = Lab { l, a, b, alpha };
    lab_to_color(lab, xyz_from_oklab)
}

pub fn lab_from_lch(l: f64, c: f64, h: f64, alpha: f64) -> Lab {
    Lab {
        l,
        a: c * (h * DEGREES_TO_RADIANS_FACTOR).cos(),
        b: c * (h * DEGREES_TO_RADIANS_FACTOR).sin(),
        alpha,
    }
}

pub fn color_from_lch(l: f64, c: f64, h: f64, alpha: f64) -> Color {
    let lab = lab_from_lch(l, c, h, alpha);
    color_from_lab(lab.l, lab.a, lab.b, alpha)
}

pub fn color_from_oklch(l: f64, c: f64, h: f64, alpha: f64) -> Color {
    let lab = lab_from_lch(l, c, h, alpha);
    color_from_oklab(lab.l, lab.a, lab.b, alpha)
}

/// The values of a color function: `rgb(1, 2, 3)`, or `rgb(1 2 3 / 4)` with
/// the slash.
fn color_values(ast: &Ast, node: NodeId) -> Vec<NodeId> {
    let mut values: Vec<NodeId> = function_arguments(ast, node).collect();
    if let [arg] = values[..]
        && let Some(expr) = ast.only_child(arg)
        && ast.node_type(expr) == NodeType::Expression
    {
        values = ast.children(expr).collect();
        if let [v0, v1, last] = values[..]
            && ast.kind(last) == Kind::BinaryExpression
            && let (Some(left), Some(right), Some(op)) = (
                ast.field(last, Field::Left),
                ast.field(last, Field::Right),
                ast.field(last, Field::Operator),
            )
            && ast.matches(op, "/")
        {
            values = vec![v0, v1, left, right];
        }
    }
    values
}

fn function_color(ast: &Ast, node: NodeId) -> Option<Color> {
    let name = function_name(ast, node);
    let values = color_values(ast, node);
    if name.is_empty() || values.len() < 3 || values.len() > 4 {
        return None;
    }
    let num = |i: usize, factor: f64| get_numeric_value(ast, values[i], factor, 0.0, 1.0);
    let signed = |i: usize, factor: f64| get_numeric_value(ast, values[i], factor, -1.0, 1.0);
    let alpha = if values.len() == 4 { num(3, 1.0)? } else { 1.0 };
    match name {
        "rgb" | "rgba" => Some(Color {
            red: num(0, 255.0)?,
            green: num(1, 255.0)?,
            blue: num(2, 255.0)?,
            alpha,
        }),
        "hsl" | "hsla" => {
            let h = get_angle(ast, values[0])?;
            Some(color_from_hsl(h, num(1, 100.0)?, num(2, 100.0)?, alpha))
        }
        "hwb" => {
            let h = get_angle(ast, values[0])?;
            Some(color_from_hwb(h, num(1, 100.0)?, num(2, 100.0)?, alpha))
        }
        "lab" => {
            let l = num(0, 100.0)?;
            let a = signed(1, 125.0)?;
            let b = signed(2, 125.0)?;
            Some(color_from_lab(l * 100.0, a * 125.0, b * 125.0, alpha))
        }
        "lch" => {
            let l = num(0, 100.0)?;
            let c = num(1, 230.0)?;
            let h = get_angle(ast, values[2])?;
            Some(color_from_lch(l * 100.0, c * 230.0, h, alpha))
        }
        "oklab" => {
            let l = num(0, 1.0)?;
            let a = signed(1, 0.4)?;
            let b = signed(2, 0.4)?;
            Some(color_from_oklab(l, a * 0.4, b * 0.4, alpha))
        }
        "oklch" => {
            let l = num(0, 1.0)?;
            let c = num(1, 0.4)?;
            let h = get_angle(ast, values[2])?;
            Some(color_from_oklch(l, c * 0.4, h, alpha))
        }
        _ => None,
    }
}

/// `getColorValue`.
pub fn get_color_value(ast: &Ast, node: NodeId) -> Option<Color> {
    match ast.node_type(node) {
        NodeType::HexColorValue => color_from_hex(ast.get_text(node)),
        NodeType::Function => function_color(ast, node),
        NodeType::Identifier => {
            let term = ast.parent(node);
            if term.is_some_and(|p| ast.node_type(p) != NodeType::Term) {
                return None;
            }
            if let Some(expression) = term.and_then(|t| ast.parent(t))
                && ast.node_type(expression) == NodeType::BinaryExpression
                && let Some(entry) = ast.parent(expression)
                && ast.node_type(entry) == NodeType::ListEntry
                && ast.field(entry, Field::Key) == Some(expression)
            {
                return None;
            }
            let candidate = ast.get_text(node).to_lowercase();
            if candidate == "none" {
                return None;
            }
            named_color(&candidate).and_then(color_from_hex)
        }
        _ => None,
    }
}
