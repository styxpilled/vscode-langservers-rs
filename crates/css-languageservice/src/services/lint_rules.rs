//! Port of `lintRules.ts`.

use serde_json::{Map, Value};

use crate::parser::nodes::{Level, Rule};

/// `LintSettings`: rule ids to `"ignore"`, `"warning"` or `"error"`, and
/// `validProperties`.
pub type LintSettings = Map<String, Value>;

macro_rules! rules {
    ($($name:ident = $id:literal, $level:ident, $message:literal;)*) => {
        $(pub static $name: Rule = Rule { id: $id, message: $message };)*
        /// The rules with their default level, in upstream order.
        pub static RULES: &[(&Rule, Level)] = &[$((&$name, Level::$level)),*];
    };
}

rules! {
    ALL_VENDOR_PREFIXES = "compatibleVendorPrefixes", Ignore, "When using a vendor-specific prefix make sure to also include all other vendor-specific properties";
    INCLUDE_STANDARD_PROPERTY_WHEN_USING_VENDOR_PREFIX = "vendorPrefix", Warning, "When using a vendor-specific prefix also include the standard property";
    DUPLICATE_DECLARATIONS = "duplicateProperties", Ignore, "Do not use duplicate style definitions";
    EMPTY_RULE_SET = "emptyRules", Warning, "Do not use empty rulesets";
    IMPORT_STATEMENT = "importStatement", Ignore, "Import statements do not load in parallel";
    BEWARE_OF_BOX_MODEL_SIZE = "boxModel", Ignore, "Do not use width or height when using padding or border";
    UNIVERSAL_SELECTOR = "universalSelector", Ignore, "The universal selector (*) is known to be slow";
    ZERO_WITH_UNIT = "zeroUnits", Ignore, "No unit for zero needed";
    REQUIRED_PROPERTIES_FOR_FONT_FACE = "fontFaceProperties", Warning, "@font-face rule must define 'src' and 'font-family' properties";
    HEX_COLOR_LENGTH = "hexColorLength", Error, "Hex colors must consist of three, four, six or eight hex numbers";
    ARGS_IN_COLOR_FUNCTION = "argumentsInColorFunction", Error, "Invalid number of parameters";
    UNKNOWN_PROPERTY = "unknownProperties", Warning, "Unknown property.";
    UNKNOWN_AT_RULES = "unknownAtRules", Warning, "Unknown at-rule.";
    IE_STAR_HACK = "ieHack", Ignore, "IE hacks are only necessary when supporting IE7 and older";
    UNKNOWN_VENDOR_SPECIFIC_PROPERTY = "unknownVendorSpecificProperties", Ignore, "Unknown vendor specific property.";
    PROPERTY_IGNORED_DUE_TO_DISPLAY = "propertyIgnoredDueToDisplay", Warning, "Property is ignored due to the display.";
    AVOID_IMPORTANT = "important", Ignore, "Avoid using !important. It is an indication that the specificity of the entire CSS has gotten out of control and needs to be refactored.";
    AVOID_FLOAT = "float", Ignore, "Avoid using 'float'. Floats lead to fragile CSS that is easy to break if one aspect of the layout changes.";
    AVOID_ID_SELECTOR = "idSelector", Ignore, "Selectors should not contain IDs because these rules are too tightly coupled with the HTML.";
}

/// The id of the `validProperties` setting.
pub const VALID_PROPERTIES: &str = "validProperties";

/// `LintConfigurationSettings`.
#[derive(Clone, Copy, Debug)]
pub struct LintConfigurationSettings<'a> {
    conf: Option<&'a LintSettings>,
}

impl<'a> LintConfigurationSettings<'a> {
    pub fn new(conf: Option<&'a LintSettings>) -> Self {
        LintConfigurationSettings { conf }
    }

    pub fn get_rule(&self, rule: &Rule) -> Level {
        let level = self
            .get_setting(rule.id)
            .and_then(Value::as_str)
            .and_then(|s| match s {
                "ignore" => Some(Level::Ignore),
                "warning" => Some(Level::Warning),
                "error" => Some(Level::Error),
                _ => None,
            });
        level.unwrap_or_else(|| {
            RULES
                .iter()
                .find(|(r, _)| r.id == rule.id)
                .map_or(Level::Ignore, |(_, l)| *l)
        })
    }

    pub fn get_setting(&self, id: &str) -> Option<&'a Value> {
        self.conf?.get(id)
    }
}
