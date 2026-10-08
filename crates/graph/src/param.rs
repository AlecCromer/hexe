//! Parameter model: `ParamValue` (DESIGN.md `crates/graph contract`), `ParamSpec`, `ParamSet`.
use std::collections::BTreeMap;
use std::mem::discriminant;

/// Longest string parameter accepted (capped allocations, AGENTS.md §4).
pub const MAX_PARAM_STRING: usize = 1 << 20;

/// One parameter value. (Channel expressions come later.)
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    F(f32),
    I(i32),
    B(bool),
    V3([f32; 3]),
    S(String),
    Menu(u32),
}

/// Task-spec alias: `Param` = `ParamValue` (variants keep DESIGN.md names F/I/B/V3/S/Menu).
pub type Param = ParamValue;

impl ParamValue {
    /// Same variant (type) as `other`, ignoring the payload.
    pub fn same_kind(&self, other: &ParamValue) -> bool {
        discriminant(self) == discriminant(other)
    }

    /// `false` for NaN/inf floats and over-long strings: hostile values never enter the graph.
    pub fn is_acceptable(&self) -> bool {
        match self {
            ParamValue::F(v) => v.is_finite(),
            ParamValue::V3(v) => v.iter().all(|c| c.is_finite()),
            ParamValue::S(s) => s.len() <= MAX_PARAM_STRING,
            ParamValue::I(_) | ParamValue::B(_) | ParamValue::Menu(_) => true,
        }
    }
}

/// Declaration of one parameter of a node type.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSpec {
    pub name: String,
    pub label: String,
    pub default: ParamValue,
    /// UI hint only; the engine does not clamp.
    pub range: Option<(f32, f32)>,
    /// Menu entries `(token, label)`; the menu value (`ParamValue::Menu(i)`) is the position. Empty = none declared.
    pub menu: Vec<(String, String)>,
    /// Code-editor hint for a string parameter (`"vex"`): the UI shows a multi-line editor. `None` = plain text field.
    pub language: Option<String>,
    /// Allowed values of a string parameter (UI hint: drawn as a dropdown; the node still validates in `cook`). Empty = free text.
    pub choices: Vec<String>,
}

impl ParamSpec {
    pub fn new(name: &str, default: ParamValue) -> Self {
        ParamSpec {
            name: name.to_string(),
            label: name.to_string(),
            default,
            range: None,
            menu: Vec::new(),
            language: None,
            choices: Vec::new(),
        }
    }
    #[must_use]
    pub fn label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }
    /// Declare the menu entries `(token, label)` in menu-index order (UI hint; the engine still range-checks in `cook`).
    #[must_use]
    pub fn menu(mut self, entries: &[(&str, &str)]) -> Self {
        self.menu = entries.iter().map(|(t, l)| ((*t).to_string(), (*l).to_string())).collect();
        self
    }
    /// Mark a string parameter as source code in `lang` (UI hint: multi-line editor with inline compile errors).
    #[must_use]
    pub fn language(mut self, lang: &str) -> Self {
        self.language = Some(lang.to_string());
        self
    }
    /// Declare the allowed values of a string parameter (UI dropdown hint).
    #[must_use]
    pub fn choices(mut self, values: &[&str]) -> Self {
        self.choices = values.iter().map(|v| (*v).to_string()).collect();
        self
    }
    #[must_use]
    pub fn range(mut self, lo: f32, hi: f32) -> Self {
        self.range = Some((lo, hi));
        self
    }
}

/// Name → value store of one node instance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamSet(BTreeMap<String, ParamValue>);

impl ParamSet {
    pub fn new() -> Self {
        Self::default()
    }
    /// Builder: used for overrides handed to `Graph::add_node` (validated there against the specs).
    #[must_use]
    pub fn with(mut self, name: &str, value: ParamValue) -> Self {
        self.0.insert(name.to_string(), value);
        self
    }
    pub(crate) fn set(&mut self, name: &str, value: ParamValue) {
        self.0.insert(name.to_string(), value);
    }
    pub fn get(&self, name: &str) -> Option<&ParamValue> {
        self.0.get(name)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ParamValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
    pub fn float(&self, name: &str) -> Option<f32> {
        match self.get(name)? {
            ParamValue::F(v) => Some(*v),
            _ => None,
        }
    }
    pub fn int(&self, name: &str) -> Option<i32> {
        match self.get(name)? {
            ParamValue::I(v) => Some(*v),
            _ => None,
        }
    }
    pub fn toggle(&self, name: &str) -> Option<bool> {
        match self.get(name)? {
            ParamValue::B(v) => Some(*v),
            _ => None,
        }
    }
    pub fn vec3(&self, name: &str) -> Option<[f32; 3]> {
        match self.get(name)? {
            ParamValue::V3(v) => Some(*v),
            _ => None,
        }
    }
    pub fn string(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            ParamValue::S(v) => Some(v),
            _ => None,
        }
    }
    pub fn menu(&self, name: &str) -> Option<u32> {
        match self.get(name)? {
            ParamValue::Menu(v) => Some(*v),
            _ => None,
        }
    }
}
