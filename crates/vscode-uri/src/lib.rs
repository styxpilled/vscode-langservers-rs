//! Port of the parts of `vscode-uri` (3.2.0) that the language services use.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uri {
    pub scheme: String,
    pub authority: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

impl Uri {
    /// `URI.parse`. Returns `None` where `vscode-uri` throws.
    pub fn parse(value: &str) -> Option<Uri> {
        let (scheme, rest) = match value.find([':', '/', '?', '#']) {
            Some(i) if i > 0 && value.as_bytes()[i] == b':' => (&value[..i], &value[i + 1..]),
            _ => ("", value),
        };
        let (authority, rest) = match rest.strip_prefix("//") {
            Some(r) => {
                let end = r.find(['/', '?', '#']).unwrap_or(r.len());
                (&r[..end], &r[end..])
            }
            None => ("", rest),
        };
        let (path_query, fragment) = match rest.split_once('#') {
            Some((a, b)) => (a, b),
            None => (rest, ""),
        };
        let (path, query) = match path_query.split_once('?') {
            Some((a, b)) => (a, b),
            None => (path_query, ""),
        };
        let scheme = if scheme.is_empty() { "file" } else { scheme };
        let mut path = percent_decode(path);
        if matches!(scheme, "https" | "http" | "file") && !path.starts_with('/') {
            path.insert(0, '/');
        }
        let uri = Uri {
            scheme: scheme.to_owned(),
            authority: percent_decode(authority),
            path,
            query: percent_decode(query),
            fragment: percent_decode(fragment),
        };
        uri.is_valid().then_some(uri)
    }

    /// `URI.file`.
    pub fn file(path: &str) -> Uri {
        let mut path = if cfg!(windows) {
            path.replace('\\', "/")
        } else {
            path.to_owned()
        };
        let mut authority = String::new();
        if let Some(rest) = path.strip_prefix("//") {
            match rest.find('/') {
                None => {
                    authority = rest.to_owned();
                    path = "/".to_owned();
                }
                Some(i) => {
                    authority = rest[..i].to_owned();
                    path = rest[i..].to_owned();
                }
            }
        }
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        Uri {
            scheme: "file".to_owned(),
            authority,
            path,
            query: String::new(),
            fragment: String::new(),
        }
    }

    /// `with({ path })`. Returns `None` where `vscode-uri` throws.
    pub fn with_path(&self, mut path: String) -> Option<Uri> {
        if matches!(self.scheme.as_str(), "https" | "http" | "file") && !path.starts_with('/') {
            path.insert(0, '/');
        }
        let uri = Uri {
            path,
            ..self.clone()
        };
        uri.is_valid().then_some(uri)
    }

    fn is_valid(&self) -> bool {
        let mut chars = self.scheme.chars();
        let scheme_ok = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '.' | '-'));
        if !scheme_ok {
            return false;
        }
        if self.path.is_empty() {
            return true;
        }
        if self.authority.is_empty() {
            !self.path.starts_with("//")
        } else {
            self.path.starts_with('/')
        }
    }

    /// `toString()`.
    pub fn to_string_encoded(&self) -> String {
        self.format(false)
    }

    /// `toString(true)`: no encoding except for `#` and `?`.
    pub fn to_string_unencoded(&self) -> String {
        self.format(true)
    }

    fn format(&self, skip_encoding: bool) -> String {
        let encode = |s: &str, is_path: bool, is_authority: bool| {
            if skip_encoding {
                encode_minimal(s)
            } else {
                encode_fast(s, is_path, is_authority)
            }
        };
        let mut res = String::new();
        if !self.scheme.is_empty() {
            res.push_str(&self.scheme);
            res.push(':');
        }
        if !self.authority.is_empty() || self.scheme == "file" {
            res.push_str("//");
        }
        if !self.authority.is_empty() {
            let mut authority = self.authority.as_str();
            if let Some(i) = authority.find('@') {
                let userinfo = &authority[..i];
                authority = &authority[i + 1..];
                match userinfo.rfind(':') {
                    None => res.push_str(&encode(userinfo, false, false)),
                    Some(j) => {
                        res.push_str(&encode(&userinfo[..j], false, false));
                        res.push(':');
                        res.push_str(&encode(&userinfo[j + 1..], false, true));
                    }
                }
                res.push('@');
            }
            let authority = authority.to_lowercase();
            match authority.rfind(':') {
                None => res.push_str(&encode(&authority, false, true)),
                Some(j) => {
                    res.push_str(&encode(&authority[..j], false, true));
                    res.push_str(&authority[j..]);
                }
            }
        }
        if !self.path.is_empty() {
            let b = self.path.as_bytes();
            let path = if b.len() >= 3 && b[0] == b'/' && b[2] == b':' && b[1].is_ascii_uppercase()
            {
                format!("/{}{}", b[1].to_ascii_lowercase() as char, &self.path[2..])
            } else if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_uppercase() {
                format!("{}{}", b[0].to_ascii_lowercase() as char, &self.path[1..])
            } else {
                self.path.clone()
            };
            res.push_str(&encode(&path, true, false));
        }
        if !self.query.is_empty() {
            res.push('?');
            res.push_str(&encode(&self.query, false, false));
        }
        if !self.fragment.is_empty() {
            res.push('#');
            if skip_encoding {
                res.push_str(&self.fragment);
            } else {
                res.push_str(&encode_fast(&self.fragment, false, false));
            }
        }
        res
    }

    /// `fsPath`.
    pub fn fs_path(&self) -> String {
        let b = self.path.as_bytes();
        let value = if !self.authority.is_empty() && self.path.len() > 1 && self.scheme == "file" {
            format!("//{}{}", self.authority, self.path)
        } else if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
            format!("{}{}", b[1].to_ascii_lowercase() as char, &self.path[2..])
        } else {
            self.path.clone()
        };
        if cfg!(windows) {
            value.replace('/', "\\")
        } else {
            value
        }
    }
}

/// `Utils.joinPath`.
pub fn join_path(uri: &Uri, paths: &[&str]) -> Option<Uri> {
    let mut args = vec![uri.path.as_str()];
    args.extend_from_slice(paths);
    uri.with_path(posix::join(&args))
}

/// `Utils.dirname`.
pub fn dirname(uri: &Uri) -> Option<Uri> {
    if uri.path.is_empty() || uri.path == "/" {
        return Some(uri.clone());
    }
    let dir = posix::dirname(&uri.path);
    uri.with_path(if dir == "." {
        String::new()
    } else {
        dir.to_owned()
    })
}

/// `Utils.basename`.
pub fn basename(uri: &Uri) -> &str {
    posix::basename(&uri.path)
}

/// The `path.posix` functions that `vscode-uri` bundles.
pub mod posix {
    /// `normalizeString`.
    fn normalize_string(path: &str, allow_above_root: bool) -> String {
        let b = path.as_bytes();
        let mut res = String::new();
        let mut last_segment_length = 0;
        let mut last_slash: isize = -1;
        let mut dots = 0;
        let mut code = 0u8;
        for i in 0..=b.len() {
            if i < b.len() {
                code = b[i];
            } else if code == b'/' {
                break;
            } else {
                code = b'/';
            }
            if code == b'/' {
                if last_slash == i as isize - 1 || dots == 1 {
                    // Nothing.
                } else if dots == 2 {
                    if res.len() < 2 || last_segment_length != 2 || !res.ends_with("..") {
                        if res.len() > 2 {
                            match res.rfind('/') {
                                None => {
                                    res.clear();
                                    last_segment_length = 0;
                                }
                                Some(j) => {
                                    res.truncate(j);
                                    last_segment_length = res.len() as isize
                                        - 1
                                        - res.rfind('/').map_or(-1, |k| k as isize);
                                }
                            }
                            last_slash = i as isize;
                            dots = 0;
                            continue;
                        } else if !res.is_empty() {
                            res.clear();
                            last_segment_length = 0;
                            last_slash = i as isize;
                            dots = 0;
                            continue;
                        }
                    }
                    if allow_above_root {
                        res.push_str(if res.is_empty() { ".." } else { "/.." });
                        last_segment_length = 2;
                    }
                } else {
                    let segment = &path[(last_slash + 1) as usize..i];
                    if !res.is_empty() {
                        res.push('/');
                    }
                    res.push_str(segment);
                    last_segment_length = segment.len() as isize;
                }
                last_slash = i as isize;
                dots = 0;
            } else if code == b'.' && dots != -1 {
                dots += 1;
            } else {
                dots = -1;
            }
        }
        res
    }

    pub fn normalize(path: &str) -> String {
        if path.is_empty() {
            return ".".into();
        }
        let is_absolute = path.starts_with('/');
        let trailing_separator = path.ends_with('/');
        let mut res = normalize_string(path, !is_absolute);
        if res.is_empty() && !is_absolute {
            res.push('.');
        }
        if !res.is_empty() && trailing_separator {
            res.push('/');
        }
        if is_absolute {
            res.insert(0, '/');
        }
        res
    }

    pub fn join(paths: &[&str]) -> String {
        let joined = paths
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("/");
        if joined.is_empty() {
            return ".".into();
        }
        normalize(&joined)
    }

    pub fn dirname(path: &str) -> &str {
        let b = path.as_bytes();
        if b.is_empty() {
            return ".";
        }
        let has_root = b[0] == b'/';
        let mut end = None;
        let mut matched_slash = true;
        for i in (1..b.len()).rev() {
            if b[i] == b'/' {
                if !matched_slash {
                    end = Some(i);
                    break;
                }
            } else {
                matched_slash = false;
            }
        }
        match end {
            None if has_root => "/",
            None => ".",
            Some(1) if has_root => "//",
            Some(end) => &path[..end],
        }
    }

    pub fn basename(path: &str) -> &str {
        let b = path.as_bytes();
        let mut start = 0;
        let mut end = None;
        let mut matched_slash = true;
        for i in (0..b.len()).rev() {
            if b[i] == b'/' {
                if !matched_slash {
                    start = i + 1;
                    break;
                }
            } else if end.is_none() {
                matched_slash = false;
                end = Some(i + 1);
            }
        }
        end.map_or("", |end| &path[start..end])
    }
}

fn encode_minimal(s: &str) -> String {
    s.replace('#', "%23").replace('?', "%3F")
}

/// `encodeURIComponentFast`.
fn encode_fast(s: &str, is_path: bool, is_authority: bool) -> String {
    let mut res = String::with_capacity(s.len());
    for c in s.chars() {
        let keep = c.is_ascii_alphanumeric()
            || matches!(c, '-' | '.' | '_' | '~')
            || (is_path && c == '/')
            || (is_authority && matches!(c, '[' | ']' | ':'));
        if keep {
            res.push(c);
        } else {
            for b in c.encode_utf8(&mut [0; 4]).bytes() {
                res.push_str(&format!("%{b:02X}"));
            }
        }
    }
    res
}

/// The `workspaceContext.resolveRelativePath` of the VS Code JSON server:
/// `Utils.resolvePath(URI.parse(base), relativePath).toString()`, where `base` is `resource`
/// up to its last `/`.
pub fn resolve_relative_path(relative_path: &str, resource: &str) -> String {
    let base = &resource[..resource.rfind('/').map_or(0, |i| i + 1)];
    Uri::parse(base)
        .and_then(|uri| resolve_path(&uri, relative_path))
        .map_or_else(|| relative_path.to_owned(), |uri| uri.to_string_encoded())
}

/// `Utils.resolvePath` with one path.
pub fn resolve_path(uri: &Uri, path: &str) -> Option<Uri> {
    let slash_added = !uri.path.starts_with('/');
    let base = if slash_added {
        format!("/{}", uri.path)
    } else {
        uri.path.clone()
    };
    let mut resolved = posix_resolve(&base, path);
    if slash_added && uri.authority.is_empty() {
        resolved.remove(0);
    }
    uri.with_path(resolved)
}

/// Node `path.posix.resolve(base, path)` for an absolute `base`.
fn posix_resolve(base: &str, path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_owned()
    } else if path.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{path}")
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    format!("/{}", segments.join("/"))
}

/// `vscode-uri` `percentDecode`: decodes each run of `%XX` sequences, and keeps the
/// sequences that are not valid UTF-8.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let b = s.as_bytes();
    let is_escape = |i: usize| {
        i + 2 < b.len()
            && b[i] == b'%'
            && b[i + 1].is_ascii_alphanumeric()
            && b[i + 2].is_ascii_alphanumeric()
    };
    let mut res = String::new();
    let mut i = 0;
    let mut start = 0;
    while i < b.len() {
        if is_escape(i) {
            res.push_str(&s[start..i]);
            let run_start = i;
            while is_escape(i) {
                i += 3;
            }
            res.push_str(&decode_graceful(&s[run_start..i]));
            start = i;
        } else {
            i += 1;
        }
    }
    res.push_str(&s[start..]);
    res
}

fn decode_graceful(s: &str) -> String {
    match decode_uri_component(s) {
        Some(d) => d,
        None if s.len() > 3 => format!("{}{}", &s[..3], decode_graceful(&s[3..])),
        None => s.to_owned(),
    }
}

/// JS `decodeURIComponent`. Returns `None` where JS throws `URIError`.
pub fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let byte = hex_byte(b.get(i + 1..i + 3)?)?;
        i += 3;
        let len = match byte {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return None,
        };
        let mut seq = vec![byte];
        for _ in 1..len {
            if b.get(i) != Some(&b'%') {
                return None;
            }
            seq.push(hex_byte(b.get(i + 1..i + 3)?)?);
            i += 3;
        }
        out.extend_from_slice(std::str::from_utf8(&seq).ok()?.as_bytes());
    }
    String::from_utf8(out).ok()
}

fn hex_byte(h: &[u8]) -> Option<u8> {
    let s = std::str::from_utf8(h).ok()?;
    u8::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    /// Expected values from `vscode-uri` 3.2.0.
    #[test]
    fn resolve_relative_path() {
        let cases = [
            (
                "./b.json",
                "file:///c:/foo/a.json",
                "file:///c%3A/foo/b.json",
            ),
            (
                "../b.json",
                "file:///C:/foo/bar/a.json",
                "file:///c%3A/foo/b.json",
            ),
            ("/abs/b.json", "file:///foo/a.json", "file:///abs/b.json"),
            (
                "b.json",
                "https://example.com/schemas/a.json",
                "https://example.com/schemas/b.json",
            ),
            (
                "../../../x.json",
                "https://Example.com/a/a.json",
                "https://example.com/x.json",
            ),
            (
                "sub dir/ä#?.json",
                "file:///home/u/a.json",
                "file:///home/u/sub%20dir/%C3%A4%23%3F.json",
            ),
            (
                "b.json",
                "vscode://schemas/custom/0",
                "vscode://schemas/custom/b.json",
            ),
            ("b.json", "zed://schemas/settings", "zed://schemas/b.json"),
            ("b.json", "untitled:Untitled-1", "file:///b.json"),
            ("", "file:///foo/a.json", "file:///foo"),
            ("./x/./y//z/", "file:///foo/a.json", "file:///foo/x/y/z"),
            (
                "b.json?x=1",
                "http://h:8080/p/a.json?q=2#frag",
                "http://h:8080/p/b.json%3Fx%3D1",
            ),
            (
                "b.json",
                "file://server/share/a.json",
                "file://server/share/b.json",
            ),
            ("b.json", "a.json", "file:///b.json"),
        ];
        for (relative, resource, expected) in cases {
            assert_eq!(
                super::resolve_relative_path(relative, resource),
                expected,
                "{relative} {resource}"
            );
        }
    }
}
