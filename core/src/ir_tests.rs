//! Unit tests for the IR model (`super`). Kept out of `ir.rs` to keep the source lean.

use super::*;
use serde_json::json;

fn attrs(pairs: &[(&str, Value)]) -> Attributes {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[test]
fn action_cost_sides() {
    assert!(Action::Create.adds_after() && !Action::Create.removes_before());
    assert!(Action::Delete.removes_before() && !Action::Delete.adds_after());
    assert!(Action::Update.adds_after() && Action::Update.removes_before());
    assert!(Action::Replace.adds_after() && Action::Replace.removes_before());
    assert!(!Action::NoOp.adds_after() && !Action::NoOp.removes_before());
}

#[test]
fn attribute_selection_by_state() {
    let rc = ResourceChange {
        address: "aws_instance.web".into(),
        rtype: "aws_instance".into(),
        name: "web".into(),
        action: Action::Update,
        before: Some(attrs(&[("instance_type", json!("t3.small"))])),
        after: Some(attrs(&[("instance_type", json!("t3.large"))])),
        after_unknown: None,
    };
    assert_eq!(rc.attr(SpecState::Before, "instance_type"), Some(&json!("t3.small")));
    assert_eq!(rc.attr(SpecState::After, "instance_type"), Some(&json!("t3.large")));
    assert_eq!(rc.attr(SpecState::After, "missing"), None);
}

#[test]
fn unknown_after_values_are_flagged() {
    let rc = ResourceChange {
        address: "aws_instance.web".into(),
        rtype: "aws_instance".into(),
        name: "web".into(),
        action: Action::Create,
        before: None,
        after: Some(attrs(&[("ami", json!("ami-123"))])),
        after_unknown: Some(attrs(&[("instance_type", json!(true))])),
    };
    assert!(rc.is_unknown("instance_type"));
    assert!(!rc.is_unknown("ami"));
}

#[test]
fn nested_unknown_values_are_flagged() {
    let rc = ResourceChange {
        address: "aws_instance.web".into(),
        rtype: "aws_instance".into(),
        name: "web".into(),
        action: Action::Create,
        before: None,
        after: Some(attrs(&[("root_block_device", json!([{}]))])),
        after_unknown: Some(attrs(&[
            // a computed leaf nested inside a list/object
            ("root_block_device", json!([{ "volume_id": true }])),
            ("ebs_optimized", json!(false)),
        ])),
    };
    assert!(rc.is_unknown("root_block_device")); // unknown leaf nested in a list/object
    assert!(!rc.is_unknown("ebs_optimized")); // known scalar
    assert!(!rc.is_unknown("absent")); // not present in after_unknown
}
