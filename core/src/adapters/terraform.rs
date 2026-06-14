//! Terraform adapter: parse `terraform show -json` plan output into the IR.
//!
//! Only the cost-relevant slice of the schema is modeled; serde ignores the rest. See
//! <https://developer.hashicorp.com/terraform/internals/json-format>.

use crate::ir::{Action, Attributes, ResourceChange};
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;

/// Lowest Terraform plan `format_version` major we accept (1.x ⇒ Terraform 1.1+).
const SUPPORTED_FORMAT_MAJOR: &str = "1";

/// Errors that can occur while reading a Terraform plan document.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("invalid terraform plan JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error(
        "input is not a terraform plan (missing format_version or resource_changes); \
         generate it with `terraform show -json <planfile>`"
    )]
    NotAPlan,

    #[error("unsupported terraform plan format_version {found:?}; costctl supports 1.x (Terraform 1.1+)")]
    UnsupportedFormat { found: String },
}

#[derive(Deserialize)]
struct Plan {
    // Both optional at the serde layer (missing -> None) so we reject a non-plan with a
    // clear error instead of silently producing an empty result.
    format_version: Option<String>,
    resource_changes: Option<Vec<RawChange>>,
}

#[derive(Deserialize)]
struct RawChange {
    address: String,
    #[serde(rename = "type")]
    rtype: String,
    name: String,
    mode: String,
    change: RawDelta,
}

#[derive(Deserialize)]
struct RawDelta {
    actions: Vec<String>,
    #[serde(default)]
    before: Option<Value>,
    #[serde(default)]
    after: Option<Value>,
    #[serde(default)]
    after_unknown: Option<Value>,
}

/// Parse a `terraform show -json` plan into normalized resource changes.
///
/// Returns only cost-relevant entries: **managed** resources (data sources cost nothing)
/// with a real action. No-ops and data reads are dropped.
pub fn parse_plan<R: Read>(reader: R) -> Result<Vec<ResourceChange>, ParseError> {
    let plan: Plan = serde_json::from_reader(reader)?;
    check_format(plan.format_version.as_deref())?;
    let changes = plan
        .resource_changes
        .ok_or(ParseError::NotAPlan)?
        .into_iter()
        .filter(|raw| raw.mode == "managed")
        .filter_map(|raw| {
            let action = map_action(&raw.change.actions)?;
            Some(ResourceChange {
                address: raw.address,
                rtype: raw.rtype,
                name: raw.name,
                action,
                before: as_attributes(raw.change.before),
                after: as_attributes(raw.change.after),
                after_unknown: as_attributes(raw.change.after_unknown),
            })
        })
        .collect();
    Ok(changes)
}

/// Confirm the document is a Terraform plan we understand: `format_version` must be present
/// (otherwise it isn't a plan at all) and its major version must be supported.
fn check_format(format_version: Option<&str>) -> Result<(), ParseError> {
    let version = format_version.ok_or(ParseError::NotAPlan)?;
    let major = version.split('.').next().unwrap_or_default();
    if major != SUPPORTED_FORMAT_MAJOR {
        return Err(ParseError::UnsupportedFormat { found: version.to_string() });
    }
    Ok(())
}

/// Map Terraform's `change.actions` array to an [`Action`].
///
/// Returns `None` for actions with no cost impact (`no-op`, `read`). Replacement is encoded
/// by Terraform as either ordering of create+delete; both normalize to [`Action::Replace`].
fn map_action(actions: &[String]) -> Option<Action> {
    let actions: Vec<&str> = actions.iter().map(String::as_str).collect();
    match actions.as_slice() {
        ["create"] => Some(Action::Create),
        ["update"] => Some(Action::Update),
        ["delete"] => Some(Action::Delete),
        ["create", "delete"] | ["delete", "create"] => Some(Action::Replace),
        _ => None, // ["no-op"], ["read"], or anything unrecognized
    }
}

/// Convert a plan-side value (a JSON object, or null) into [`Attributes`].
fn as_attributes(value: Option<Value>) -> Option<Attributes> {
    match value {
        Some(Value::Object(map)) => Some(map.into_iter().collect()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::SpecState;

    // The real `terraform show -json` output generated during development.
    const SAMPLE: &str =
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/sample-plan.json"));

    #[test]
    fn parses_real_terraform_plan() {
        let changes = parse_plan(SAMPLE.as_bytes()).unwrap();

        // 3 aws_instance + 1 aws_nat_gateway, all managed creates.
        assert_eq!(changes.len(), 4);
        assert!(changes.iter().all(|c| c.action == Action::Create));

        let instances: Vec<_> = changes.iter().filter(|c| c.rtype == "aws_instance").collect();
        assert_eq!(instances.len(), 3);

        // instance_type is a real value on the `after` side...
        assert_eq!(
            instances[0]
                .attr(SpecState::After, "instance_type")
                .and_then(Value::as_str),
            Some("t3.large")
        );
        // ...while `id` is computed-at-apply, so it must read as unknown ("not estimated").
        assert!(instances[0].is_unknown("id"));
    }

    #[test]
    fn maps_all_actions_and_filters_noise() {
        let json = r#"{
          "format_version": "1.2",
          "resource_changes": [
            {"address":"aws_instance.a","type":"aws_instance","name":"a","mode":"managed",
             "change":{"actions":["create"],"before":null,"after":{"instance_type":"t3.large"},"after_unknown":{"id":true}}},
            {"address":"aws_instance.b","type":"aws_instance","name":"b","mode":"managed",
             "change":{"actions":["update"],"before":{"instance_type":"t3.small"},"after":{"instance_type":"t3.large"}}},
            {"address":"aws_instance.c","type":"aws_instance","name":"c","mode":"managed",
             "change":{"actions":["delete"],"before":{"instance_type":"t3.large"},"after":null}},
            {"address":"aws_instance.d","type":"aws_instance","name":"d","mode":"managed",
             "change":{"actions":["create","delete"],"before":{"x":1},"after":{"x":2}}},
            {"address":"aws_instance.e","type":"aws_instance","name":"e","mode":"managed",
             "change":{"actions":["delete","create"],"before":{"x":1},"after":{"x":2}}},
            {"address":"aws_instance.f","type":"aws_instance","name":"f","mode":"managed",
             "change":{"actions":["no-op"],"before":{"x":1},"after":{"x":1}}},
            {"address":"data.aws_ami.g","type":"aws_ami","name":"g","mode":"data",
             "change":{"actions":["read"],"before":null,"after":{"id":"ami-1"}}},
            {"address":"data.aws_x.h","type":"aws_x","name":"h","mode":"data",
             "change":{"actions":["create"],"before":null,"after":{}}}
          ]
        }"#;

        let changes = parse_plan(json.as_bytes()).unwrap();

        // create, update, delete, replace, replace — no-op dropped; both data sources dropped.
        let got: Vec<_> = changes.iter().map(|c| (c.name.as_str(), c.action)).collect();
        assert_eq!(
            got,
            vec![
                ("a", Action::Create),
                ("b", Action::Update),
                ("c", Action::Delete),
                ("d", Action::Replace),
                ("e", Action::Replace),
            ]
        );

        // A delete reads its spec from the `before` side.
        let deleted = changes.iter().find(|c| c.name == "c").unwrap();
        assert_eq!(
            deleted.attr(SpecState::Before, "instance_type").and_then(Value::as_str),
            Some("t3.large")
        );
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_plan("not json".as_bytes()).is_err());
    }

    #[test]
    fn rejects_json_that_is_not_a_plan() {
        // Valid JSON, but no format_version => not a terraform plan.
        let err = parse_plan(r#"{"hello":"world"}"#.as_bytes()).unwrap_err();
        assert!(matches!(err, ParseError::NotAPlan));
    }

    #[test]
    fn rejects_plan_missing_resource_changes() {
        // Supported format_version but no resource_changes => not a usable plan
        // (must not be silently treated as zero changes).
        let err = parse_plan(r#"{"format_version":"1.2"}"#.as_bytes()).unwrap_err();
        assert!(matches!(err, ParseError::NotAPlan));
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let err =
            parse_plan(r#"{"format_version":"2.0","resource_changes":[]}"#.as_bytes()).unwrap_err();
        assert!(matches!(err, ParseError::UnsupportedFormat { .. }));
    }

    /// Compatibility matrix: real plans generated by several Terraform versions must all
    /// parse — format_version 1.0 (TF 1.1) through 1.2 (TF 1.5/1.9). Fixtures use a
    /// provider-agnostic `null_resource`, so this exercises the plan *format*, not pricing.
    #[test]
    fn parses_plans_across_terraform_versions() {
        let fixtures: &[(&str, &str)] = &[
            ("1.1.9", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/compat/plan-1.1.9.json"))),
            ("1.5.7", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/compat/plan-1.5.7.json"))),
            ("1.9.8", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/compat/plan-1.9.8.json"))),
        ];
        for (version, json) in fixtures {
            let changes = parse_plan(json.as_bytes())
                .unwrap_or_else(|e| panic!("TF {version} plan failed to parse: {e}"));
            assert_eq!(changes.len(), 1, "TF {version}: expected one resource change");
            let rc = &changes[0];
            assert_eq!(rc.rtype, "null_resource", "TF {version}");
            assert_eq!(rc.action, Action::Create, "TF {version}");
            assert!(rc.is_unknown("id"), "TF {version}: computed id should read as unknown");
        }
    }
}
