//! `Registry`: SOP type name → (param specs, constructor).
use hexe_geo::Detail;
use hexe_graph::{Node, ParamSpec};
use std::collections::BTreeMap;

/// A cookable SOP node (graph payload is `Detail`).
pub type SopNode = dyn Node<Detail>;
/// Parameter table of a SOP type (token → type/default), also what `hexe-cli nodes` prints.
pub type SpecFn = fn() -> Vec<ParamSpec>;
/// Fresh node instance of a SOP type. Its `params()` must equal `SpecFn()` and `type_name()` the registered name.
pub type CtorFn = fn() -> Box<SopNode>;

#[derive(Clone, Copy)]
struct Entry {
    spec: SpecFn,
    new: CtorFn,
}

#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<&'static str, Entry>,
    duplicates: Vec<&'static str>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }
    /// Register a SOP type. A repeated name keeps the first entry and is recorded in `duplicates()` (a test pins it empty).
    pub fn register(&mut self, type_name: &'static str, spec: SpecFn, new: CtorFn) {
        if self.entries.contains_key(type_name) {
            self.duplicates.push(type_name);
        } else {
            self.entries.insert(type_name, Entry { spec, new });
        }
    }
    /// New node instance of `type_name`, `None` if unknown.
    pub fn create(&self, type_name: &str) -> Option<Box<SopNode>> {
        self.entries.get(type_name).map(|e| (e.new)())
    }
    /// Param specs of `type_name`, `None` if unknown.
    pub fn specs(&self, type_name: &str) -> Option<Vec<ParamSpec>> {
        self.entries.get(type_name).map(|e| (e.spec)())
    }
    /// Registered type names, sorted.
    pub fn type_names(&self) -> Vec<&'static str> {
        self.entries.keys().copied().collect()
    }
    /// Names registered more than once (a bug in `builtins.rs`).
    pub fn duplicates(&self) -> &[&'static str] {
        &self.duplicates
    }
}

#[cfg(test)]
mod tests {
    use crate::registry as builtin_registry;

    /// Generic contract every SOP lane inherits: unique names, ctor/spec/type_name agree, at most 1 output assumed.
    #[test]
    fn every_builtin_is_self_consistent() {
        let r = builtin_registry();
        assert!(r.duplicates().is_empty(), "duplicate registrations: {:?}", r.duplicates());
        assert!(r.type_names().contains(&"null"));
        for name in r.type_names() {
            let node = r.create(name).expect("registered type constructs");
            assert_eq!(node.type_name(), name, "type_name() must equal the registered name");
            assert_eq!(node.params(), r.specs(name).expect("specs").as_slice(), "{name}: params() != spec()");
            let mut seen = std::collections::BTreeSet::new();
            for p in node.params() {
                assert!(seen.insert(p.name.clone()), "{name}: duplicate param token {}", p.name);
                assert!(p.default.is_acceptable(), "{name}.{}: default not acceptable", p.name);
            }
        }
    }

    #[test]
    fn unknown_type_is_none() {
        let r = builtin_registry();
        assert!(r.create("no-such-sop").is_none() && r.specs("no-such-sop").is_none());
    }

    #[test]
    fn duplicate_registration_is_recorded_not_replaced() {
        let mut r = crate::Registry::new();
        r.register("null", crate::builtins::null_spec_for_tests, crate::builtins::null_new_for_tests);
        r.register("null", crate::builtins::null_spec_for_tests, crate::builtins::null_new_for_tests);
        assert_eq!(r.type_names(), vec!["null"]);
        assert_eq!(r.duplicates(), &["null"]);
    }
}
