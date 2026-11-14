//! The data types of `cssLanguageTypes.ts` (`CSSDataV1` and its entries) and
//! `CSSDataProvider` (`dataProvider.ts`).

use std::sync::{Arc, LazyLock, OnceLock};

use serde::Deserialize;
use serde_json::Value;

/// The descriptions of an entry for the default hover settings, plain text
/// and markdown, built on first use.
#[derive(Clone, Debug, Default)]
pub struct DescriptionCache(pub(crate) [OnceLock<Option<lsp_types::MarkupContent>>; 2]);

impl PartialEq for DescriptionCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// `string | MarkupContent`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Description {
    Text(String),
    Markup(MarkupContent),
}

/// `MarkupContent`. The kind is a free string: custom data can have any kind.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct MarkupContent {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Reference {
    pub name: String,
    pub url: String,
}

/// `BaselineStatus`. `status` is `"false"`, `"low"` or `"high"`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BaselineStatus {
    pub status: String,
    pub baseline_low_date: Option<String>,
    pub baseline_high_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PropertyData {
    pub name: String,
    #[serde(skip)]
    pub descriptions: DescriptionCache,
    pub description: Option<Description>,
    pub browsers: Option<Vec<String>>,
    pub baseline: Option<BaselineStatus>,
    pub restrictions: Option<Vec<String>>,
    /// `EntryStatus`: `standard`, `experimental`, `nonstandard` or `obsolete`.
    pub status: Option<String>,
    pub syntax: Option<String>,
    pub values: Option<Vec<ValueData>>,
    pub references: Option<Vec<Reference>>,
    pub relevance: Option<f64>,
    pub at_rule: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DescriptorData {
    pub name: String,
    #[serde(skip)]
    pub descriptions: DescriptionCache,
    pub description: Option<Description>,
    pub references: Option<Vec<Reference>>,
    pub syntax: Option<String>,
    #[serde(rename = "type")]
    pub ty: Option<String>,
    pub values: Option<Vec<ValueData>>,
    pub browsers: Option<Vec<String>>,
    pub baseline: Option<BaselineStatus>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AtDirectiveData {
    pub name: String,
    #[serde(skip)]
    pub descriptions: DescriptionCache,
    pub description: Option<Description>,
    pub browsers: Option<Vec<String>>,
    pub baseline: Option<BaselineStatus>,
    pub status: Option<String>,
    pub references: Option<Vec<Reference>>,
    pub descriptors: Option<Vec<DescriptorData>>,
}

/// `IPseudoClassData` and `IPseudoElementData`, which have the same fields.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PseudoData {
    pub name: String,
    #[serde(skip)]
    pub descriptions: DescriptionCache,
    pub description: Option<Description>,
    pub browsers: Option<Vec<String>>,
    pub baseline: Option<BaselineStatus>,
    pub status: Option<String>,
    pub references: Option<Vec<Reference>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ValueData {
    pub name: String,
    #[serde(skip)]
    pub descriptions: DescriptionCache,
    pub description: Option<Description>,
    pub browsers: Option<Vec<String>>,
    pub baseline: Option<BaselineStatus>,
    pub status: Option<String>,
    pub references: Option<Vec<Reference>>,
}

/// `CSSDataV1`, typed. Only the built-in data uses it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CssDataV1 {
    properties: Vec<PropertyData>,
    at_directives: Vec<AtDirectiveData>,
    pseudo_classes: Vec<PseudoData>,
    pseudo_elements: Vec<PseudoData>,
}

/// `CSSDataProvider`.
#[derive(Clone, Debug, Default)]
pub struct CssDataProvider {
    pub properties: Vec<Arc<PropertyData>>,
    pub at_directives: Vec<Arc<AtDirectiveData>>,
    pub pseudo_classes: Vec<Arc<PseudoData>>,
    pub pseudo_elements: Vec<Arc<PseudoData>>,
}

static BUILT_IN: LazyLock<CssDataProvider> = LazyLock::new(|| {
    let data: CssDataV1 =
        serde_json::from_str(include_str!("data/browsers.json")).expect("valid built-in CSS data");
    fn arcs<T>(v: Vec<T>) -> Vec<Arc<T>> {
        v.into_iter().map(Arc::new).collect()
    }
    CssDataProvider {
        properties: arcs(data.properties),
        at_directives: arcs(data.at_directives),
        pseudo_classes: arcs(data.pseudo_classes),
        pseudo_elements: arcs(data.pseudo_elements),
    }
});

impl CssDataProvider {
    /// The data of `webCustomData.ts` (`@vscode/web-custom-data`).
    pub fn built_in() -> &'static CssDataProvider {
        &BUILT_IN
    }

    /// `new CSSDataProvider(data)` for custom data (`CSSDataV1` as JSON).
    /// Upstream keeps each entry with a string `name`. Here an entry with a
    /// field of the wrong type is also dropped.
    pub fn new(data: &Value) -> Self {
        fn entries<T: for<'de> Deserialize<'de>>(data: &Value, key: &str) -> Vec<Arc<T>> {
            match data.get(key) {
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|e| T::deserialize(e).ok().map(Arc::new))
                    .collect(),
                _ => Vec::new(),
            }
        }
        CssDataProvider {
            properties: entries(data, "properties"),
            at_directives: entries(data, "atDirectives"),
            pseudo_classes: entries(data, "pseudoClasses"),
            pseudo_elements: entries(data, "pseudoElements"),
        }
    }
}
