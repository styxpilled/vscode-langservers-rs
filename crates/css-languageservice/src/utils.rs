//! JS behavior that the services need.

/// `parseFloat`.
pub fn parse_float(s: &str) -> f64 {
    let s = s.trim_start_matches(is_js_whitespace);
    let b = s.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    if s[i..].starts_with("Infinity") {
        return if b.first() == Some(&b'-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int = digits(i);
    i += int;
    let mut frac = 0;
    if b.get(i) == Some(&b'.') {
        frac = digits(i + 1);
        if frac > 0 || int > 0 {
            i += 1 + frac;
        }
    }
    if int == 0 && frac == 0 {
        return f64::NAN;
    }
    let mut end = i;
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(b.get(i + 1), Some(b'+' | b'-')));
        if digits(i + 1 + sign) > 0 {
            end = i + 1 + sign + digits(i + 1 + sign);
        }
    }
    // Rust reads all forms of this prefix (`5.`, `.5`, `1.e2`) as JS does.
    s[..end].parse().unwrap_or(f64::NAN)
}

/// `WhiteSpace` and `LineTerminator` of ECMAScript.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// The UTF-16 length of `s`.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `getLimitedString`: the first 140 UTF-16 units and an ellipsis. Upstream
/// can cut a surrogate pair; here the cut is before the pair.
pub fn limited_string(s: &str) -> String {
    if utf16_len(s) < 140 {
        return s.to_owned();
    }
    let mut units = 0;
    let end = s
        .char_indices()
        .find(|(_, c)| {
            units += c.len_utf16();
            units > 140
        })
        .map_or(s.len(), |(i, _)| i);
    format!("{}\u{2026}", &s[..end])
}

/// `Number.prototype.toString(16)` of V8 (`DoubleToRadixCString`) for a
/// finite, positive number.
pub fn js_to_radix16(value: f64) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut integer = value.floor();
    let mut fraction = value - integer;
    let next_up = f64::from_bits(value.to_bits() + 1);
    let mut delta = (0.5 * (next_up - value)).max(f64::from_bits(1));
    let mut digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= 16.0;
            delta *= 16.0;
            let digit = fraction as usize;
            digits.push(DIGITS[digit]);
            fraction -= digit as f64;
            if (fraction > 0.5 || (fraction == 0.5 && digit & 1 == 1)) && fraction + delta > 1.0 {
                // Round up, with carry over the digits.
                loop {
                    let Some(last) = digits.pop() else {
                        integer += 1.0;
                        break;
                    };
                    let d = DIGITS.iter().position(|&c| c == last).unwrap_or(0);
                    if d + 1 < 16 {
                        digits.push(DIGITS[d + 1]);
                        break;
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    let mut out = Vec::new();
    loop {
        let remainder = integer % 16.0;
        out.push(DIGITS[remainder as usize]);
        integer = (integer - remainder) / 16.0;
        if integer <= 0.0 {
            break;
        }
    }
    out.reverse();
    if !digits.is_empty() {
        out.push(b'.');
        out.extend(digits);
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Formats a number as JS `String(n)` does.
pub fn js_number_to_string(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_owned()
}

#[cfg(test)]
mod tests {
    use super::{js_number_to_string, js_to_radix16, parse_float};

    #[test]
    fn number_to_string_as_js() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (255.0, "255"),
            (0.1, "0.1"),
            (-1.5, "-1.5"),
            (1e21, "1e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
        ];
        for (v, s) in cases {
            assert_eq!(js_number_to_string(v), s, "{v}");
        }
    }

    #[test]
    fn radix16_as_js() {
        let cases = [
            (205.0, "cd"),
            (255.0, "ff"),
            (156.0, "9c"),
            (204.5, "cc.8"),
            (254.9, "fe.e66666666668"),
        ];
        for (v, s) in cases {
            assert_eq!(js_to_radix16(v), s, "{v}");
        }
    }

    #[test]
    fn parse_float_as_js() {
        let cases: &[(&str, f64)] = &[
            ("0", 0.0),
            ("1px", 1.0),
            ("  -1.5e2em", -150.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("1.e2", 100.0),
            ("1e", 1.0),
            ("1e+", 1.0),
            ("+0.0px", 0.0),
            ("-Infinity", f64::NEG_INFINITY),
            ("Infinityx", f64::INFINITY),
            ("1e-400px", 0.0),
            ("1e400", f64::INFINITY),
            ("\u{FEFF}7", 7.0),
            ("0x10", 0.0),
        ];
        for &(s, expected) in cases {
            assert_eq!(parse_float(s), expected, "{s}");
        }
        for s in ["", ".", "-", "+.", "e5", "px", "\u{85}1", "inf"] {
            assert!(parse_float(s).is_nan(), "{s}");
        }
    }
}

/// A JS exception: `vscode-uri` throws on invalid URIs, and a property access
/// can throw on unexpected JSON.
#[derive(Debug)]
pub struct Thrown;

/// `resources.ts` `dirname`.
pub fn dirname(uri: &str) -> Result<String, Thrown> {
    let uri = vscode_uri::Uri::parse(uri).ok_or(Thrown)?;
    Ok(vscode_uri::dirname(&uri)
        .ok_or(Thrown)?
        .to_string_unencoded())
}

/// `resources.ts` `joinPath`.
pub fn join_path(uri: &str, paths: &[&str]) -> Result<String, Thrown> {
    let uri = vscode_uri::Uri::parse(uri).ok_or(Thrown)?;
    Ok(vscode_uri::join_path(&uri, paths)
        .ok_or(Thrown)?
        .to_string_unencoded())
}
