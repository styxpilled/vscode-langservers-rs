//! Port of `utils/glob.ts`.

use regress::Regex;

/// `createRegex` with `extended` and `globstar`. Returns `None` where JS throws.
pub fn create_regex(glob: &str, extended: bool, globstar: bool) -> Option<Regex> {
    Regex::new(&glob_to_pattern(glob, extended, globstar)).ok()
}

pub fn glob_to_pattern(glob: &str, extended: bool, globstar: bool) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut re = String::new();
    let mut in_group = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '/' | '$' | '^' | '+' | '.' | '(' | ')' | '=' | '!' | '|' => {
                re.push('\\');
                re.push(c);
            }
            '?' if extended => re.push('.'),
            '[' | ']' if extended => re.push(c),
            '{' if extended => {
                in_group = true;
                re.push('(');
            }
            '}' if extended => {
                in_group = false;
                re.push(')');
            }
            '?' | '[' | ']' | '{' | '}' | ',' => {
                if in_group {
                    re.push('|');
                } else {
                    re.push('\\');
                    re.push(c);
                }
            }
            '*' => {
                let prev = i.checked_sub(1).map(|p| chars[p]);
                let mut star_count = 1;
                while chars.get(i + 1) == Some(&'*') {
                    star_count += 1;
                    i += 1;
                }
                let next = chars.get(i + 1).copied();
                if !globstar {
                    re.push_str(".*");
                } else {
                    let is_globstar = star_count > 1
                        && matches!(prev, None | Some('/' | '{' | ','))
                        && matches!(next, None | Some('/' | ',' | '}'));
                    if is_globstar {
                        if next == Some('/') {
                            i += 1;
                        } else if prev == Some('/') && re.ends_with("\\/") {
                            re.truncate(re.len() - 2);
                        }
                        re.push_str("((?:[^/]*(?:/|$))*)");
                    } else {
                        re.push_str("([^/]*)");
                    }
                }
            }
            _ => re.push(c),
        }
        i += 1;
    }
    format!("^{re}$")
}
