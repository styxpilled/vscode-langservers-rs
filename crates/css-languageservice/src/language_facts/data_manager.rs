//! Port of `dataManager.ts`.

use foldhash::HashMap;
use std::borrow::Cow;
use std::sync::Arc;

use crate::data::{AtDirectiveData, CssDataProvider, PropertyData, PseudoData};

/// `CSSDataManager`.
#[derive(Clone, Debug, Default)]
pub struct CssDataManager {
    properties: Entries<PropertyData>,
    at_directives: Entries<AtDirectiveData>,
    pseudo_classes: Entries<PseudoData>,
    pseudo_elements: Entries<PseudoData>,
}

/// One `_xSet` object of upstream and its values in insertion order.
#[derive(Clone, Debug)]
struct Entries<T> {
    by_name: HashMap<String, Arc<T>>,
    list: Vec<Arc<T>>,
}

impl<T> Default for Entries<T> {
    fn default() -> Self {
        Entries {
            by_name: HashMap::default(),
            list: Vec::new(),
        }
    }
}

impl<T> Entries<T> {
    fn collect<'a>(
        providers: &'a [&'a CssDataProvider],
        get: impl Fn(&'a CssDataProvider) -> &'a [Arc<T>],
        name: impl Fn(&T) -> &str,
    ) -> Self
    where
        T: 'a,
    {
        let mut by_name = HashMap::default();
        let mut list = Vec::new();
        for provider in providers {
            for entry in get(provider) {
                let key = name(entry);
                if !by_name.contains_key(key) {
                    by_name.insert(key.to_owned(), entry.clone());
                    list.push(entry.clone());
                }
            }
        }
        Entries { by_name, list }
    }
}

impl CssDataManager {
    /// `new CSSDataManager({ useDefaultDataProvider, customDataProviders })`.
    pub fn new(built_in: bool, providers: &[CssDataProvider]) -> Self {
        let mut manager = CssDataManager::default();
        manager.set_data_providers(built_in, providers);
        manager
    }

    pub fn set_data_providers(&mut self, built_in: bool, providers: &[CssDataProvider]) {
        let mut all = Vec::with_capacity(providers.len() + 1);
        if built_in {
            all.push(CssDataProvider::built_in());
        }
        all.extend(providers);
        self.properties = Entries::collect(&all, |p| &p.properties, |e| &e.name);
        self.at_directives = Entries::collect(&all, |p| &p.at_directives, |e| &e.name);
        self.pseudo_classes = Entries::collect(&all, |p| &p.pseudo_classes, |e| &e.name);
        self.pseudo_elements = Entries::collect(&all, |p| &p.pseudo_elements, |e| &e.name);
    }

    pub fn get_property(&self, name: &str) -> Option<&PropertyData> {
        self.properties.by_name.get(name).map(|e| &**e)
    }

    pub fn get_at_directive(&self, name: &str) -> Option<&AtDirectiveData> {
        self.at_directives.by_name.get(name).map(|e| &**e)
    }

    pub fn get_pseudo_class(&self, name: &str) -> Option<&PseudoData> {
        self.pseudo_classes.by_name.get(name).map(|e| &**e)
    }

    pub fn get_pseudo_element(&self, name: &str) -> Option<&PseudoData> {
        self.pseudo_elements.by_name.get(name).map(|e| &**e)
    }

    pub fn get_properties(&self) -> &[Arc<PropertyData>] {
        &self.properties.list
    }

    pub fn get_at_directives(&self) -> &[Arc<AtDirectiveData>] {
        &self.at_directives.list
    }

    pub fn get_pseudo_classes(&self) -> &[Arc<PseudoData>] {
        &self.pseudo_classes.list
    }

    pub fn get_pseudo_elements(&self) -> &[Arc<PseudoData>] {
        &self.pseudo_elements.list
    }

    pub fn is_known_property(&self, name: &str) -> bool {
        self.properties.by_name.contains_key(&*lowercase(name))
    }

    pub fn is_standard_property(&self, name: &str) -> bool {
        match self.properties.by_name.get(&*lowercase(name)) {
            Some(p) => p
                .status
                .as_deref()
                .is_none_or(|s| s.is_empty() || s == "standard"),
            None => false,
        }
    }
}

/// `toLowerCase()` without an allocation when the text is already lowercase.
fn lowercase(s: &str) -> Cow<'_, str> {
    if s.is_ascii() && !s.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.to_lowercase())
    }
}
