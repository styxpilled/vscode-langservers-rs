//! Port of `entry.ts`.

use lsp_types::{MarkupContent, MarkupKind};

use crate::data::{
    AtDirectiveData, BaselineStatus, Description, DescriptionCache, DescriptorData, PropertyData,
    PseudoData, Reference, ValueData,
};

/// `HoverSettings`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct HoverSettings {
    pub documentation: Option<bool>,
    pub references: Option<bool>,
}

/// `IEntry2`: the fields that the descriptions use.
pub trait Entry {
    fn description(&self) -> Option<&Description>;
    fn browsers(&self) -> Option<&[String]>;
    fn baseline(&self) -> Option<&BaselineStatus>;
    fn status(&self) -> Option<&str>;
    fn references(&self) -> Option<&[Reference]>;
    fn descriptions(&self) -> &DescriptionCache;
    /// `'syntax' in entry`: only properties and descriptors have the key.
    fn syntax(&self) -> Option<&str> {
        None
    }
}

macro_rules! impl_entry {
    ($ty:ty $(, $syntax:ident)?) => {
        impl Entry for $ty {
            fn description(&self) -> Option<&Description> {
                self.description.as_ref()
            }
            fn browsers(&self) -> Option<&[String]> {
                self.browsers.as_deref()
            }
            fn baseline(&self) -> Option<&BaselineStatus> {
                self.baseline.as_ref()
            }
            fn status(&self) -> Option<&str> {
                self.status.as_deref()
            }
            fn references(&self) -> Option<&[Reference]> {
                self.references.as_deref()
            }
            fn descriptions(&self) -> &DescriptionCache {
                &self.descriptions
            }
            $(fn syntax(&self) -> Option<&str> {
                self.$syntax.as_deref()
            })?
        }
    };
}

impl_entry!(AtDirectiveData);
impl_entry!(PseudoData);
impl_entry!(ValueData);
impl_entry!(PropertyData, syntax);
impl_entry!(DescriptorData, syntax);

/// `browserNames`: id, name, platform.
pub const BROWSER_NAMES: &[(&str, &str, &str)] = &[
    ("C", "Chrome", "desktop"),
    ("CA", "Chrome", "Android"),
    ("E", "Edge", "desktop"),
    ("FF", "Firefox", "desktop"),
    ("FFA", "Firefox", "Android"),
    ("S", "Safari", "macOS"),
    ("SM", "Safari", "iOS"),
];

pub const BASELINE_LIMITED: &str = "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMTgiIGhlaWdodD0iMTAiIHZpZXdCb3g9IjAgMCA1NDAgMzAwIiBmaWxsPSJub25lIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPgogIDxzdHlsZT4KICAgIC5ncmF5LXNoYXBlIHsKICAgICAgZmlsbDogI0M2QzZDNjsgLyogTGlnaHQgbW9kZSAqLwogICAgfQoKICAgIEBtZWRpYSAocHJlZmVycy1jb2xvci1zY2hlbWU6IGRhcmspIHsKICAgICAgLmdyYXktc2hhcGUgewogICAgICAgIGZpbGw6ICM1NjU2NTY7IC8qIERhcmsgbW9kZSAqLwogICAgICB9CiAgICB9CiAgPC9zdHlsZT4KICA8cGF0aCBkPSJNMTUwIDBMMjQwIDkwTDIxMCAxMjBMMTIwIDMwTDE1MCAwWiIgZmlsbD0iI0YwOTQwOSIvPgogIDxwYXRoIGQ9Ik00MjAgMzBMNTQwIDE1MEw0MjAgMjcwTDM5MCAyNDBMNDgwIDE1MEwzOTAgNjBMNDIwIDMwWiIgY2xhc3M9ImdyYXktc2hhcGUiLz4KICA8cGF0aCBkPSJNMzMwIDE4MEwzMDAgMjEwTDM5MCAzMDBMNDIwIDI3MEwzMzAgMTgwWiIgZmlsbD0iI0YwOTQwOSIvPgogIDxwYXRoIGQ9Ik0xMjAgMzBMMTUwIDYwTDYwIDE1MEwxNTAgMjQwTDEyMCAyNzBMMCAxNTBMMTIwIDMwWiIgY2xhc3M9ImdyYXktc2hhcGUiLz4KICA8cGF0aCBkPSJNMzkwIDBMNDIwIDMwTDE1MCAzMDBMMTIwIDI3MEwzOTAgMFoiIGZpbGw9IiNGMDk0MDkiLz4KPC9zdmc+";
pub const BASELINE_LOW: &str = "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMTgiIGhlaWdodD0iMTAiIHZpZXdCb3g9IjAgMCA1NDAgMzAwIiBmaWxsPSJub25lIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPgogIDxzdHlsZT4KICAgIC5ibHVlLXNoYXBlIHsKICAgICAgZmlsbDogI0E4QzdGQTsgLyogTGlnaHQgbW9kZSAqLwogICAgfQoKICAgIEBtZWRpYSAocHJlZmVycy1jb2xvci1zY2hlbWU6IGRhcmspIHsKICAgICAgLmJsdWUtc2hhcGUgewogICAgICAgIGZpbGw6ICMyRDUwOUU7IC8qIERhcmsgbW9kZSAqLwogICAgICB9CiAgICB9CgogICAgLmRhcmtlci1ibHVlLXNoYXBlIHsKICAgICAgICBmaWxsOiAjMUI2RUYzOwogICAgfQoKICAgIEBtZWRpYSAocHJlZmVycy1jb2xvci1zY2hlbWU6IGRhcmspIHsKICAgICAgICAuZGFya2VyLWJsdWUtc2hhcGUgewogICAgICAgICAgICBmaWxsOiAjNDE4NUZGOwogICAgICAgIH0KICAgIH0KCiAgPC9zdHlsZT4KICA8cGF0aCBkPSJNMTUwIDBMMTgwIDMwTDE1MCA2MEwxMjAgMzBMMTUwIDBaIiBjbGFzcz0iYmx1ZS1zaGFwZSIvPgogIDxwYXRoIGQ9Ik0yMTAgNjBMMjQwIDkwTDIxMCAxMjBMMTgwIDkwTDIxMCA2MFoiIGNsYXNzPSJibHVlLXNoYXBlIi8+CiAgPHBhdGggZD0iTTQ1MCA2MEw0ODAgOTBMNDUwIDEyMEw0MjAgOTBMNDUwIDYwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNNTEwIDEyMEw1NDAgMTUwTDUxMCAxODBMNDgwIDE1MEw1MTAgMTIwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNNDUwIDE4MEw0ODAgMjEwTDQ1MCAyNDBMNDIwIDIxMEw0NTAgMTgwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNMzkwIDI0MEw0MjAgMjcwTDM5MCAzMDBMMzYwIDI3MEwzOTAgMjQwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNMzMwIDE4MEwzNjAgMjEwTDMzMCAyNDBMMzAwIDIxMEwzMzAgMTgwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNOTAgNjBMMTIwIDkwTDkwIDEyMEw2MCA5MEw5MCA2MFoiIGNsYXNzPSJibHVlLXNoYXBlIi8+CiAgPHBhdGggZD0iTTM5MCAwTDQyMCAzMEwxNTAgMzAwTDAgMTUwTDMwIDEyMEwxNTAgMjQwTDM5MCAwWiIgY2xhc3M9ImRhcmtlci1ibHVlLXNoYXBlIi8+Cjwvc3ZnPg==";
pub const BASELINE_HIGH: &str = "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMTgiIGhlaWdodD0iMTAiIHZpZXdCb3g9IjAgMCA1NDAgMzAwIiBmaWxsPSJub25lIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPgogIDxzdHlsZT4KICAgIC5ncmVlbi1zaGFwZSB7CiAgICAgIGZpbGw6ICNDNEVFRDA7IC8qIExpZ2h0IG1vZGUgKi8KICAgIH0KCiAgICBAbWVkaWEgKHByZWZlcnMtY29sb3Itc2NoZW1lOiBkYXJrKSB7CiAgICAgIC5ncmVlbi1zaGFwZSB7CiAgICAgICAgZmlsbDogIzEyNTIyNTsgLyogRGFyayBtb2RlICovCiAgICAgIH0KICAgIH0KICA8L3N0eWxlPgogIDxwYXRoIGQ9Ik00MjAgMzBMMzkwIDYwTDQ4MCAxNTBMMzkwIDI0MEwzMzAgMTgwTDMwMCAyMTBMMzkwIDMwMEw1NDAgMTUwTDQyMCAzMFoiIGNsYXNzPSJncmVlbi1zaGFwZSIvPgogIDxwYXRoIGQ9Ik0xNTAgMEwzMCAxMjBMNjAgMTUwTDE1MCA2MEwyMTAgMTIwTDI0MCA5MEwxNTAgMFoiIGNsYXNzPSJncmVlbi1zaGFwZSIvPgogIDxwYXRoIGQ9Ik0zOTAgMEw0MjAgMzBMMTUwIDMwMEwwIDE1MEwzMCAxMjBMMTUwIDI0MEwzOTAgMFoiIGZpbGw9IiMxRUE0NDYiLz4KPC9zdmc+";

fn entry_status(status: &str) -> &'static str {
    match status {
        "nonstandard" => "🚨️ Property is nonstandard. Avoid using it.\n\n",
        "obsolete" => "🚨️️️ Property is obsolete. Avoid using it.\n\n",
        _ => "",
    }
}

fn entry_baseline_status(baseline: &BaselineStatus, browsers: Option<&[String]>) -> String {
    if baseline.status == "false" {
        let missing = get_missing_baseline_browsers(browsers);
        let mut status = String::from("Limited availability across major browsers");
        if !missing.is_empty() {
            status += &format!(" (Not fully implemented in {missing})");
        }
        return status;
    }
    let level = if baseline.status == "low" {
        "Newly"
    } else {
        "Widely"
    };
    let mut status = format!("{level} available across major browsers");
    if let Some(date) = &baseline.baseline_low_date {
        let year = date.split('-').next().unwrap_or(date);
        status += &format!(" (Baseline since {year})");
    }
    status
}

fn entry_baseline_image(baseline: &BaselineStatus) -> String {
    let image = match baseline.status.as_str() {
        "low" => BASELINE_LOW,
        "high" => BASELINE_HIGH,
        _ => BASELINE_LIMITED,
    };
    format!("![Baseline icon]({image})")
}

/// `getEntryDescription`.
pub fn get_entry_description(
    entry: &(impl Entry + ?Sized),
    does_support_markdown: bool,
    settings: Option<&HoverSettings>,
) -> Option<MarkupContent> {
    if documentation(settings) && references(settings) {
        return entry.descriptions().0[usize::from(does_support_markdown)]
            .get_or_init(|| build_entry_description(entry, does_support_markdown, None))
            .clone();
    }
    build_entry_description(entry, does_support_markdown, settings)
}

fn build_entry_description(
    entry: &(impl Entry + ?Sized),
    does_support_markdown: bool,
    settings: Option<&HoverSettings>,
) -> Option<MarkupContent> {
    let (kind, value) = if does_support_markdown {
        (
            MarkupKind::Markdown,
            entry_markdown_description(entry, settings),
        )
    } else {
        (
            MarkupKind::PlainText,
            entry_string_description(entry, settings),
        )
    };
    (!value.is_empty()).then_some(MarkupContent { kind, value })
}

/// Escapes markdown syntax tokens and `<`, `>`.
pub fn text_to_marked_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '.'
            | '!' => {
                out.push('\\');
                out.push(c);
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn documentation(settings: Option<&HoverSettings>) -> bool {
    settings.and_then(|s| s.documentation) != Some(false)
}

fn references(settings: Option<&HoverSettings>) -> bool {
    settings.and_then(|s| s.references) != Some(false)
}

/// The description, or `None` for `!entry.description || entry.description === ''`.
fn non_empty_description(entry: &(impl Entry + ?Sized)) -> Option<&Description> {
    entry
        .description()
        .filter(|d| !matches!(d, Description::Text(t) if t.is_empty()))
}

fn entry_string_description(
    entry: &(impl Entry + ?Sized),
    settings: Option<&HoverSettings>,
) -> String {
    let Some(description) = non_empty_description(entry) else {
        return String::new();
    };
    let text = match description {
        Description::Text(t) => t,
        Description::Markup(m) => return m.value.clone(),
    };
    let mut result = String::new();
    if documentation(settings) {
        let status = entry.status().map_or("", entry_status);
        result += status;
        result += text;
        if let Some(baseline) = entry.baseline()
            && status.is_empty()
        {
            result += "\n\n";
            result += &entry_baseline_status(baseline, entry.browsers());
        }
        if let Some(syntax) = entry.syntax() {
            result += "\n\nSyntax: ";
            result += syntax;
        }
    }
    if let Some(refs) = entry.references().filter(|r| !r.is_empty())
        && references(settings)
    {
        if !result.is_empty() {
            result += "\n\n";
        }
        let refs: Vec<String> = refs
            .iter()
            .map(|r| format!("{}: {}", r.name, r.url))
            .collect();
        result += &refs.join(" | ");
    }
    result
}

fn entry_markdown_description(
    entry: &(impl Entry + ?Sized),
    settings: Option<&HoverSettings>,
) -> String {
    let Some(description) = non_empty_description(entry) else {
        return String::new();
    };
    let mut result = String::new();
    if documentation(settings) {
        let status = entry.status().map_or("", entry_status);
        result += status;
        match description {
            Description::Text(t) => result += &text_to_marked_string(t),
            Description::Markup(m) if m.kind == "markdown" => result += &m.value,
            Description::Markup(m) => result += &text_to_marked_string(&m.value),
        }
        if let Some(baseline) = entry.baseline()
            && status.is_empty()
        {
            result += &format!(
                "\n\n{} _{}_",
                entry_baseline_image(baseline),
                entry_baseline_status(baseline, entry.browsers())
            );
        }
        if let Some(syntax) = entry.syntax().filter(|s| !s.is_empty()) {
            result += "\n\nSyntax: ";
            result += &text_to_marked_string(syntax);
        }
    }
    if let Some(refs) = entry.references().filter(|r| !r.is_empty())
        && references(settings)
    {
        if !result.is_empty() {
            result += "\n\n";
        }
        let refs: Vec<String> = refs
            .iter()
            .map(|r| format!("[{}]({})", r.name, r.url))
            .collect();
        result += &refs.join(" | ");
    }
    result
}

/// The browser id of `shortCompatPattern`
/// (`/(E|FFA|FF|SM|S|CA|C|IE|O)([\d|\.]+)?/`, not anchored).
fn short_compat_browser(s: &str) -> Option<&str> {
    const IDS: [&str; 9] = ["E", "FFA", "FF", "SM", "S", "CA", "C", "IE", "O"];
    (0..s.len()).find_map(|i| {
        let rest = s.get(i..)?;
        IDS.into_iter().find(|id| rest.starts_with(id))
    })
}

/// `Intl.ListFormat("en", { style: "long", type: "disjunction" })`.
fn format_disjunction(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} or {b}"),
        [init @ .., last] => format!("{}, or {last}", init.join(", ")),
    }
}

/// `getMissingBaselineBrowsers`: the major browsers without an entry in
/// `browsers`, for example `Safari`.
pub fn get_missing_baseline_browsers(browsers: Option<&[String]>) -> String {
    let Some(browsers) = browsers else {
        return String::new();
    };
    let mut missing: Vec<_> = BROWSER_NAMES.to_vec();
    for s in browsers {
        if let Some(id) = short_compat_browser(s) {
            missing.retain(|(m, _, _)| *m != id);
        }
    }
    let mut names: Vec<(&str, String)> = Vec::new();
    for (id, name, platform) in missing {
        match names.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = name.to_owned(),
            None if id == "E" => names.push((name, name.to_owned())),
            None => names.push((name, format!("{name} on {platform}"))),
        }
    }
    let values: Vec<String> = names.into_iter().map(|(_, v)| v).collect();
    format_disjunction(&values)
}
