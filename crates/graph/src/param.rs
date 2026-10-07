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
}

impl ParamSpec {
    pub fn new(name: &str, default: ParamValue) -> Self {
        ParamSpec { name: name.to_string(), label: name.to_string(), default, range: None }
    }
    #[must_use]
    pub fn label(mut self, label: &str) -> Self {
        self.label = label.to_string();
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
