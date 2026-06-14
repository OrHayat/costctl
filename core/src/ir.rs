//! The normalized intermediate representation (IR).
//!
//! A tool-agnostic model of resource changes. Each adapter parses its own IaC tool's plan
//! into this single canonical model, so the pricing, cost, and reporting layers never need
//! to know which tool produced the change.
//!
//! Unit tests live in `ir_tests.rs`.

use serde_json::Value;
use std::collections::BTreeMap;

/// A resource's attributes (e.g. `instance_type` -> `"t3.large"`). `BTreeMap` keeps
/// ordering stable for deterministic output and tests.
pub type Attributes = BTreeMap<String, Value>;

/// What a plan intends to do to a single resource.
///
/// A replacement (destroy then recreate, in either order) normalizes to [`Action::Replace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Update,
    Delete,
    Replace,
    NoOp,
}

impl Action {
    /// Whether this action adds the resource's *new* (`after`) cost.
    pub fn adds_after(self) -> bool {
        matches!(self, Action::Create | Action::Update | Action::Replace)
    }

    /// Whether this action removes the resource's *old* (`before`) cost.
    pub fn removes_before(self) -> bool {
        matches!(self, Action::Delete | Action::Update | Action::Replace)
    }
}

/// Which side of a change to read attributes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecState {
    Before,
    After,
}

/// A single normalized resource change.
#[derive(Debug, Clone)]
pub struct ResourceChange {
    /// Stable, unique identifier for the resource within the plan (e.g. `aws_instance.web`).
    pub address: String,
    /// Resource type (e.g. `aws_instance`).
    pub rtype: String,
    /// Resource name (e.g. `web`).
    pub name: String,
    pub action: Action,
    /// Attributes prior to the change (absent for creates).
    pub before: Option<Attributes>,
    /// Attributes after the change (absent for deletes).
    pub after: Option<Attributes>,
    /// Attribute keys whose values are computed and not yet known — they won't be resolved
    /// until the change is applied. A cost-relevant attribute here can't be priced, so it is
    /// reported as "not estimated" rather than $0.
    pub after_unknown: Option<Attributes>,
}

impl ResourceChange {
    /// Attributes for the requested side, if present.
    pub fn attributes(&self, state: SpecState) -> Option<&Attributes> {
        match state {
            SpecState::Before => self.before.as_ref(),
            SpecState::After => self.after.as_ref(),
        }
    }

    /// Look up a single attribute on the requested side.
    pub fn attr(&self, state: SpecState, key: &str) -> Option<&Value> {
        self.attributes(state)?.get(key)
    }

    /// True if `key`'s value is computed and won't be known until the change is applied,
    /// so it can't be priced. Handles nested unknowns: Terraform mirrors structure in
    /// `after_unknown`, so a partially-computed object/array attribute is flagged when *any*
    /// leaf inside it is unknown — not just when the whole value is the scalar `true`.
    pub fn is_unknown(&self, key: &str) -> bool {
        self.after_unknown
            .as_ref()
            .and_then(|u| u.get(key))
            .is_some_and(contains_unknown)
    }
}

/// Recursively detect an unknown leaf in an `after_unknown` value. Terraform encodes a scalar
/// unknown as `true` and a nested unknown as the surrounding object/array with `true` at the
/// computed leaves (known leaves are omitted entirely).
fn contains_unknown(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Array(items) => items.iter().any(contains_unknown),
        Value::Object(map) => map.values().any(contains_unknown),
        _ => false,
    }
}

#[cfg(test)]
#[path = "ir_tests.rs"]
mod tests;
