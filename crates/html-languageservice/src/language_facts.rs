//! Port of `dataProvider.ts` and `dataManager.ts`, with the data types of
//! `htmlLanguageTypes.ts`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};
use lsp_types::{MarkupContent, MarkupKind};
use serde::Deserialize;
use serde_json::Value;

/// `string | MarkupContent`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Description {
    Text(String),
    Markup(DescriptionMarkup),
}

/// `MarkupContent` of custom data: any kind.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DescriptionMarkup {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Reference {
    pub name: String,
    pub url: String,
}

/// `Baseline`: `false`, `'low'` or `'high'`.
#[derive(Clone, Debug, PartialEq)]
pub enum Baseline {
    False,
    Low,
    High,
    Other,
}

impl<'de> Deserialize<'de> for Baseline {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Bool(false) => Baseline::False,
            Value::String(s) if s == "low" => Baseline::Low,
            Value::String(s) if s == "high" => Baseline::High,
            _ => Baseline::Other,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BaselineStatus {
    pub baseline: Baseline,
    pub baseline_low_date: Option<String>,
    pub baseline_high_date: Option<String>,
}

/// The fields of `ITagData`, `IAttributeData` and `IValueData` that
/// `generateDocumentation` reads.
pub trait Entry {
    fn description(&self) -> Option<&Description>;
    fn references(&self) -> Option<&[Reference]>;
    fn browsers(&self) -> Option<&[String]>;
    fn status(&self) -> Option<&BaselineStatus>;
}

macro_rules! entry {
    ($t:ty) => {
        impl Entry for $t {
            fn description(&self) -> Option<&Description> {
                self.description.as_ref()
            }
            fn references(&self) -> Option<&[Reference]> {
                self.references.as_deref()
            }
            fn browsers(&self) -> Option<&[String]> {
                self.browsers.as_deref()
            }
            fn status(&self) -> Option<&BaselineStatus> {
                self.status.as_ref()
            }
        }
    };
}

/// `ITagData`. A tag without `attributes` has none (upstream throws on it).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TagData {
    pub name: String,
    pub description: Option<Description>,
    #[serde(default)]
    pub attributes: Vec<AttributeData>,
    pub references: Option<Vec<Reference>>,
    #[serde(rename = "void")]
    pub is_void: Option<bool>,
    pub browsers: Option<Vec<String>>,
    pub status: Option<BaselineStatus>,
}

/// `IAttributeData`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttributeData {
    pub name: String,
    pub description: Option<Description>,
    pub value_set: Option<String>,
    pub values: Option<Vec<ValueData>>,
    pub references: Option<Vec<Reference>>,
    pub browsers: Option<Vec<String>>,
    pub status: Option<BaselineStatus>,
}

/// `IValueData`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ValueData {
    pub name: String,
    pub description: Option<Description>,
    pub references: Option<Vec<Reference>>,
    pub browsers: Option<Vec<String>>,
    pub status: Option<BaselineStatus>,
}

entry!(TagData);
entry!(AttributeData);
entry!(ValueData);

#[derive(Deserialize)]
struct ValueSet {
    name: String,
    values: Vec<ValueData>,
}

/// `HTMLDataProvider`. `isApplicable` is always true.
#[derive(Debug)]
pub struct HtmlDataProvider {
    id: String,
    tags: Vec<TagData>,
    tag_map: HashMap<String, usize>,
    global_attributes: Vec<AttributeData>,
    value_set_map: HashMap<String, Vec<ValueData>>,
}

static HTML5: LazyLock<Arc<HtmlDataProvider>> = LazyLock::new(|| {
    let data: Value =
        serde_json::from_str(include_str!("data/html5.json")).expect("valid built-in HTML data");
    Arc::new(HtmlDataProvider::new("html5", &data))
});

impl HtmlDataProvider {
    /// `newHTMLDataProvider(id, customData)`. Upstream keeps each entry with
    /// a string `name`. Here an entry with a field of the wrong type is also
    /// dropped.
    pub fn new(id: &str, data: &Value) -> Self {
        fn entries<T: for<'de> Deserialize<'de>>(data: &Value, key: &str) -> Vec<T> {
            match data.get(key) {
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|e| T::deserialize(e).ok())
                    .collect(),
                _ => Vec::new(),
            }
        }
        let tags: Vec<TagData> = entries(data, "tags");
        let mut tag_map = HashMap::new();
        for (i, t) in tags.iter().enumerate() {
            tag_map.insert(t.name.to_lowercase(), i);
        }
        let mut value_set_map = HashMap::new();
        for vs in entries::<ValueSet>(data, "valueSets") {
            value_set_map.insert(vs.name, vs.values);
        }
        HtmlDataProvider {
            id: id.to_owned(),
            tags,
            tag_map,
            global_attributes: entries(data, "globalAttributes"),
            value_set_map,
        }
    }

    /// The data of `webCustomData.ts` (`@vscode/web-custom-data`), id `html5`.
    pub fn built_in() -> Arc<HtmlDataProvider> {
        HTML5.clone()
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn provide_tags(&self) -> &[TagData] {
        &self.tags
    }

    pub fn provide_attributes(&self, tag: &str) -> impl Iterator<Item = &AttributeData> {
        let tag_entry = self
            .tag_map
            .get(&tag.to_lowercase())
            .map(|&i| &self.tags[i]);
        tag_entry
            .into_iter()
            .flat_map(|t| &t.attributes)
            .chain(&self.global_attributes)
    }

    pub fn provide_values(&self, tag: &str, attribute: &str) -> Vec<&ValueData> {
        let attribute = attribute.to_lowercase();
        let tag_attributes = self
            .tag_map
            .get(&tag.to_lowercase())
            .map(|&i| self.tags[i].attributes.as_slice())
            .unwrap_or_default();
        let mut values = Vec::new();
        for a in tag_attributes.iter().chain(&self.global_attributes) {
            if a.name.to_lowercase() == attribute {
                values.extend(a.values.iter().flatten());
                if let Some(set) = a.value_set.as_ref().and_then(|s| self.value_set_map.get(s)) {
                    values.extend(set);
                }
            }
        }
        values
    }
}

const BASELINE_LIMITED: &str = "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMTgiIGhlaWdodD0iMTAiIHZpZXdCb3g9IjAgMCA1NDAgMzAwIiBmaWxsPSJub25lIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPgogIDxzdHlsZT4KICAgIC5ncmF5LXNoYXBlIHsKICAgICAgZmlsbDogI0M2QzZDNjsgLyogTGlnaHQgbW9kZSAqLwogICAgfQoKICAgIEBtZWRpYSAocHJlZmVycy1jb2xvci1zY2hlbWU6IGRhcmspIHsKICAgICAgLmdyYXktc2hhcGUgewogICAgICAgIGZpbGw6ICM1NjU2NTY7IC8qIERhcmsgbW9kZSAqLwogICAgICB9CiAgICB9CiAgPC9zdHlsZT4KICA8cGF0aCBkPSJNMTUwIDBMMjQwIDkwTDIxMCAxMjBMMTIwIDMwTDE1MCAwWiIgZmlsbD0iI0YwOTQwOSIvPgogIDxwYXRoIGQ9Ik00MjAgMzBMNTQwIDE1MEw0MjAgMjcwTDM5MCAyNDBMNDgwIDE1MEwzOTAgNjBMNDIwIDMwWiIgY2xhc3M9ImdyYXktc2hhcGUiLz4KICA8cGF0aCBkPSJNMzMwIDE4MEwzMDAgMjEwTDM5MCAzMDBMNDIwIDI3MEwzMzAgMTgwWiIgZmlsbD0iI0YwOTQwOSIvPgogIDxwYXRoIGQ9Ik0xMjAgMzBMMTUwIDYwTDYwIDE1MEwxNTAgMjQwTDEyMCAyNzBMMCAxNTBMMTIwIDMwWiIgY2xhc3M9ImdyYXktc2hhcGUiLz4KICA8cGF0aCBkPSJNMzkwIDBMNDIwIDMwTDE1MCAzMDBMMTIwIDI3MEwzOTAgMFoiIGZpbGw9IiNGMDk0MDkiLz4KPC9zdmc+";
const BASELINE_LOW: &str = "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMTgiIGhlaWdodD0iMTAiIHZpZXdCb3g9IjAgMCA1NDAgMzAwIiBmaWxsPSJub25lIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPgogIDxzdHlsZT4KICAgIC5ibHVlLXNoYXBlIHsKICAgICAgZmlsbDogI0E4QzdGQTsgLyogTGlnaHQgbW9kZSAqLwogICAgfQoKICAgIEBtZWRpYSAocHJlZmVycy1jb2xvci1zY2hlbWU6IGRhcmspIHsKICAgICAgLmJsdWUtc2hhcGUgewogICAgICAgIGZpbGw6ICMyRDUwOUU7IC8qIERhcmsgbW9kZSAqLwogICAgICB9CiAgICB9CgogICAgLmRhcmtlci1ibHVlLXNoYXBlIHsKICAgICAgICBmaWxsOiAjMUI2RUYzOwogICAgfQoKICAgIEBtZWRpYSAocHJlZmVycy1jb2xvci1zY2hlbWU6IGRhcmspIHsKICAgICAgICAuZGFya2VyLWJsdWUtc2hhcGUgewogICAgICAgICAgICBmaWxsOiAjNDE4NUZGOwogICAgICAgIH0KICAgIH0KCiAgPC9zdHlsZT4KICA8cGF0aCBkPSJNMTUwIDBMMTgwIDMwTDE1MCA2MEwxMjAgMzBMMTUwIDBaIiBjbGFzcz0iYmx1ZS1zaGFwZSIvPgogIDxwYXRoIGQ9Ik0yMTAgNjBMMjQwIDkwTDIxMCAxMjBMMTgwIDkwTDIxMCA2MFoiIGNsYXNzPSJibHVlLXNoYXBlIi8+CiAgPHBhdGggZD0iTTQ1MCA2MEw0ODAgOTBMNDUwIDEyMEw0MjAgOTBMNDUwIDYwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNNTEwIDEyMEw1NDAgMTUwTDUxMCAxODBMNDgwIDE1MEw1MTAgMTIwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNNDUwIDE4MEw0ODAgMjEwTDQ1MCAyNDBMNDIwIDIxMEw0NTAgMTgwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNMzkwIDI0MEw0MjAgMjcwTDM5MCAzMDBMMzYwIDI3MEwzOTAgMjQwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNMzMwIDE4MEwzNjAgMjEwTDMzMCAyNDBMMzAwIDIxMEwzMzAgMTgwWiIgY2xhc3M9ImJsdWUtc2hhcGUiLz4KICA8cGF0aCBkPSJNOTAgNjBMMTIwIDkwTDkwIDEyMEw2MCA5MEw5MCA2MFoiIGNsYXNzPSJibHVlLXNoYXBlIi8+CiAgPHBhdGggZD0iTTM5MCAwTDQyMCAzMEwxNTAgMzAwTDAgMTUwTDMwIDEyMEwxNTAgMjQwTDM5MCAwWiIgY2xhc3M9ImRhcmtlci1ibHVlLXNoYXBlIi8+Cjwvc3ZnPg==";
const BASELINE_HIGH: &str = "data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMTgiIGhlaWdodD0iMTAiIHZpZXdCb3g9IjAgMCA1NDAgMzAwIiBmaWxsPSJub25lIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPgogIDxzdHlsZT4KICAgIC5ncmVlbi1zaGFwZSB7CiAgICAgIGZpbGw6ICNDNEVFRDA7IC8qIExpZ2h0IG1vZGUgKi8KICAgIH0KCiAgICBAbWVkaWEgKHByZWZlcnMtY29sb3Itc2NoZW1lOiBkYXJrKSB7CiAgICAgIC5ncmVlbi1zaGFwZSB7CiAgICAgICAgZmlsbDogIzEyNTIyNTsgLyogRGFyayBtb2RlICovCiAgICAgIH0KICAgIH0KICA8L3N0eWxlPgogIDxwYXRoIGQ9Ik00MjAgMzBMMzkwIDYwTDQ4MCAxNTBMMzkwIDI0MEwzMzAgMTgwTDMwMCAyMTBMMzkwIDMwMEw1NDAgMTUwTDQyMCAzMFoiIGNsYXNzPSJncmVlbi1zaGFwZSIvPgogIDxwYXRoIGQ9Ik0xNTAgMEwzMCAxMjBMNjAgMTUwTDE1MCA2MEwyMTAgMTIwTDI0MCA5MEwxNTAgMFoiIGNsYXNzPSJncmVlbi1zaGFwZSIvPgogIDxwYXRoIGQ9Ik0zOTAgMEw0MjAgMzBMMTUwIDMwMEwwIDE1MEwzMCAxMjBMMTUwIDI0MEwzOTAgMFoiIGZpbGw9IiMxRUE0NDYiLz4KPC9zdmc+";

fn baseline_image(status: &BaselineStatus) -> String {
    let img = match status.baseline {
        Baseline::Low => BASELINE_LOW,
        Baseline::High => BASELINE_HIGH,
        _ => BASELINE_LIMITED,
    };
    format!("![Baseline icon]({img})")
}

fn baseline_status(status: &BaselineStatus, browsers: Option<&[String]>) -> String {
    if status.baseline == Baseline::False {
        let missing = missing_baseline_browsers(browsers);
        let mut s = "Limited availability across major browsers".to_owned();
        if !missing.is_empty() {
            s += &format!(" (Not fully implemented in {missing})");
        }
        return s;
    }
    let year = status
        .baseline_low_date
        .as_deref()
        .map_or("undefined", |d| d.split('-').next().unwrap_or(""));
    let kind = if status.baseline == Baseline::Low {
        "Newly"
    } else {
        "Widely"
    };
    format!("{kind} available across major browsers (Baseline since {year})")
}

/// `browserNames`: id, name, platform.
const BROWSER_NAMES: [(&str, &str, &str); 7] = [
    ("C", "Chrome", "desktop"),
    ("CA", "Chrome", "Android"),
    ("E", "Edge", "desktop"),
    ("FF", "Firefox", "desktop"),
    ("FFA", "Firefox", "Android"),
    ("S", "Safari", "macOS"),
    ("SM", "Safari", "iOS"),
];

/// The first match of `/(E|FFA|FF|SM|S|CA|C|IE|O)([\d|\.]+)?/`, group 1.
fn short_compat_browser(s: &str) -> Option<&'static str> {
    const ALTERNATIVES: [&str; 9] = ["E", "FFA", "FF", "SM", "S", "CA", "C", "IE", "O"];
    (0..s.len())
        .filter(|&i| s.is_char_boundary(i))
        .find_map(|i| ALTERNATIVES.into_iter().find(|a| s[i..].starts_with(a)))
}

/// `getMissingBaselineBrowsers`.
pub fn missing_baseline_browsers(browsers: Option<&[String]>) -> String {
    let Some(browsers) = browsers else {
        return String::new();
    };
    let mut missing: Vec<_> = BROWSER_NAMES.to_vec();
    for b in browsers {
        if let Some(id) = short_compat_browser(b) {
            missing.retain(|(i, ..)| *i != id);
        }
    }
    let mut names: Vec<(&str, String)> = Vec::new();
    for (id, name, platform) in missing {
        let value = if names.iter().any(|(n, _)| *n == name) || id == "E" {
            name.to_owned()
        } else {
            format!("{name} on {platform}")
        };
        match names.iter_mut().find(|(n, _)| *n == name) {
            Some((_, v)) => *v = value,
            None => names.push((name, value)),
        }
    }
    // `Intl.ListFormat('en', { style: 'long', type: 'disjunction' })`.
    let values: Vec<String> = names.into_iter().map(|(_, v)| v).collect();
    match values.as_slice() {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} or {b}"),
        [init @ .., last] => format!("{}, or {last}", init.join(", ")),
    }
}

/// The `settings` argument of `generateDocumentation`.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct HoverSettings {
    pub documentation: Option<bool>,
    pub references: Option<bool>,
}

/// `generateDocumentation`.
pub fn generate_documentation(
    item: &dyn Entry,
    settings: HoverSettings,
    supports_markdown: bool,
) -> Option<MarkupContent> {
    let mut value = String::new();
    let documentation = settings.documentation != Some(false);
    if documentation {
        match item.description() {
            Some(Description::Text(s)) if !s.is_empty() => value += s,
            Some(Description::Markup(m)) => value += &m.value,
            _ => {}
        }
    }
    if let Some(status) = item.status().filter(|_| documentation) {
        if !value.is_empty() {
            value += "\n\n";
        }
        let s = baseline_status(status, item.browsers());
        if supports_markdown {
            value += &format!("{} _{s}_", baseline_image(status));
        } else {
            value += &s;
        }
    }
    if let Some(refs) = item.references().filter(|r| !r.is_empty())
        && settings.references != Some(false)
    {
        if !value.is_empty() {
            value += "\n\n";
        }
        let parts: Vec<String> = if supports_markdown {
            refs.iter()
                .map(|r| format!("[{}]({})", r.name, r.url))
                .collect()
        } else {
            refs.iter()
                .map(|r| format!("{}: {}", r.name, r.url))
                .collect()
        };
        value += &parts.join(if supports_markdown { " | " } else { "\n" });
    }
    if value.is_empty() {
        return None;
    }
    Some(MarkupContent {
        kind: if supports_markdown {
            MarkupKind::Markdown
        } else {
            MarkupKind::PlainText
        },
        value,
    })
}

/// `String.prototype.localeCompare` (ICU collation).
static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    Collator::try_new(Default::default(), CollatorOptions::default())
        .expect("compiled collation data")
});

/// `HTMLDataManager`. Each data provider applies to all languages, so the
/// void elements are computed once for each `set_data_providers`.
#[derive(Debug)]
pub struct HtmlDataManager {
    data_providers: Vec<Arc<HtmlDataProvider>>,
    void_elements: Vec<String>,
}

impl Default for HtmlDataManager {
    fn default() -> Self {
        Self::new(true, Vec::new())
    }
}

impl HtmlDataManager {
    pub fn new(use_default_data_provider: bool, custom: Vec<Arc<HtmlDataProvider>>) -> Self {
        let mut m = HtmlDataManager {
            data_providers: Vec::new(),
            void_elements: Vec::new(),
        };
        m.set_data_providers(use_default_data_provider, custom);
        m
    }

    /// `setDataProviders`.
    pub fn set_data_providers(&mut self, built_in: bool, providers: Vec<Arc<HtmlDataProvider>>) {
        self.data_providers.clear();
        if built_in {
            self.data_providers.push(HtmlDataProvider::built_in());
        }
        self.data_providers.extend(providers);
        self.void_elements = void_elements_of(&self.data_providers);
    }

    pub fn data_providers(&self) -> &[Arc<HtmlDataProvider>] {
        &self.data_providers
    }

    /// `getVoidElements(languageId)`.
    pub fn void_elements(&self) -> &[String] {
        &self.void_elements
    }

    /// `isVoidElement`: a binary search with `localeCompare` in the list that
    /// `sort()` ordered, as upstream.
    pub fn is_void_element(e: &str, void_elements: &[String]) -> bool {
        if e.is_empty() {
            return false;
        }
        if e.bytes()
            .all(|b| is_simple_name_byte(b.to_ascii_lowercase()))
        {
            let key = || e.bytes().map(|b| b.to_ascii_lowercase());
            return binary_search(void_elements, |t| {
                if t.bytes().all(is_simple_name_byte) {
                    t.bytes().cmp(key())
                } else {
                    COLLATOR.compare(t, &e.to_ascii_lowercase())
                }
            });
        }
        let key = e.to_lowercase();
        binary_search(void_elements, |t| COLLATOR.compare(t, &key))
    }

    /// `isPathAttribute`.
    pub fn is_path_attribute(tag: &str, attr: &str) -> bool {
        if attr == "src" || attr == "href" {
            return true;
        }
        let attrs: &[&str] = match tag {
            "a" | "area" | "link" => &["href"],
            "body" => &["background"],
            "blockquote" | "del" | "ins" | "q" => &["cite"],
            "form" => &["action"],
            "frame" | "img" => &["src", "longdesc"],
            "object" => &["data"],
            "script" | "audio" | "embed" | "source" | "track" => &["src"],
            "button" => &["formaction"],
            "command" => &["icon"],
            "html" => &["manifest"],
            "input" => &["src", "formaction"],
            "video" => &["src", "poster"],
            _ => &[],
        };
        attrs.contains(&attr)
    }
}

/// `getVoidElements(dataProviders)`.
pub fn void_elements_of(providers: &[Arc<HtmlDataProvider>]) -> Vec<String> {
    let mut tags: Vec<String> = providers
        .iter()
        .flat_map(|p| p.provide_tags())
        .filter(|t| t.is_void == Some(true))
        .map(|t| t.name.clone())
        .collect();
    tags.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    tags
}

/// The root collation orders strings of `[a-z0-9-]` as their bytes.
fn is_simple_name_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
}

/// `arrays.binarySearch`, true if found.
fn binary_search(items: &[String], compare: impl Fn(&str) -> Ordering) -> bool {
    let (mut low, mut high) = (0isize, items.len() as isize - 1);
    while low <= high {
        let mid = (low + high) / 2;
        match compare(&items[mid as usize]) {
            Ordering::Less => low = mid + 1,
            Ordering::Greater => high = mid - 1,
            Ordering::Equal => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{COLLATOR, is_simple_name_byte, missing_baseline_browsers};

    #[test]
    fn simple_names_collate_as_bytes() {
        let alphabet = b"-09amz";
        let mut names = vec![String::new()];
        for _ in 0..3 {
            let longer: Vec<String> = names
                .iter()
                .flat_map(|n| alphabet.iter().map(move |&c| format!("{n}{}", c as char)))
                .collect();
            names.extend(longer);
        }
        names.sort();
        names.dedup();
        assert!(names.iter().all(|n| n.bytes().all(is_simple_name_byte)));
        for a in &names {
            for b in &names {
                assert_eq!(COLLATOR.compare(a, b), a.cmp(b), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn missing_browser_lists() {
        let missing = |b: &[&str]| {
            let b: Vec<String> = b.iter().map(|s| s.to_string()).collect();
            missing_baseline_browsers(Some(&b))
        };
        assert_eq!(missing(&["S1", "SM2"]), "Chrome, Edge, or Firefox");
        assert_eq!(
            missing(&["C1", "CA1", "S1"]),
            "Edge, Firefox, or Safari on iOS"
        );
        assert_eq!(missing(&[]), "Chrome, Edge, Firefox, or Safari");
    }
}
