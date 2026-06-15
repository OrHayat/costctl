//! The cost diff engine.
//!
//! Turns a set of [`ResourceChange`]s plus a [`Pricer`] into a [`Report`]: the signed monthly
//! delta of each priced change, their net, and a separate bucket for everything that couldn't
//! be estimated (with the reason). Usage-based rates are carried per line but never summed —
//! a variable rate has no fixed monthly total to fold into the net.

use crate::ir::{Action, ResourceChange, SpecState};
use crate::pricing::{price_keys, Money, Price, Pricer, UsageRate};

/// A priced change: its signed monthly delta plus any usage rate to surface.
#[derive(Debug, Clone, PartialEq)]
pub struct CostLine {
    pub address: String,
    pub rtype: String,
    pub action: Action,
    /// Short label for what's priced — a sku like "t3.large", or "fixed" when the type's
    /// cost has no discriminator.
    pub detail: String,
    /// Signed fixed monthly delta: positive adds cost, negative removes it.
    pub delta_monthly: Money,
    /// Usage-based rate from the cost-bearing side, shown alongside but never summed.
    pub usage: Option<UsageRate>,
}

/// Why a change couldn't be priced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotEstimatedReason {
    /// A price-determining attribute is computed and not known until apply.
    ValueUnknown,
    /// The resource type/sku isn't in the price catalog.
    Unpriced,
}

/// A change we deliberately did not assign a number to — reported, never treated as $0.
#[derive(Debug, Clone, PartialEq)]
pub struct NotEstimated {
    pub address: String,
    pub rtype: String,
    pub reason: NotEstimatedReason,
}

/// The forecast: priced lines, their net, and everything we couldn't estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub lines: Vec<CostLine>,
    /// Sum of the fixed monthly deltas. Usage rates are excluded (they'd be fabricated).
    pub net_monthly: Money,
    pub not_estimated: Vec<NotEstimated>,
}

/// Compute the monthly cost delta for a set of resource changes.
pub fn compute(changes: &[ResourceChange], pricer: &dyn Pricer) -> Report {
    let mut lines = Vec::new();
    let mut not_estimated = Vec::new();
    let mut net = 0.0;

    for rc in changes {
        // No-ops carry no cost change. The adapter already drops them, but be explicit so a
        // directly-constructed NoOp never pollutes the report with a $0 line.
        if rc.action == Action::NoOp {
            continue;
        }
        match price_change(rc, pricer) {
            Ok(line) => {
                net += line.delta_monthly.0;
                lines.push(line);
            }
            Err(reason) => not_estimated.push(NotEstimated {
                address: rc.address.clone(),
                rtype: rc.rtype.clone(),
                reason,
            }),
        }
    }

    Report { lines, net_monthly: Money(net), not_estimated }
}

/// Price a single change, or explain why it can't be estimated.
///
/// Delta is uniform across actions via the IR's own helpers: add the `after` cost when the
/// action creates value, remove the `before` cost when it destroys value. Create is `+after`,
/// Delete is `-before`, Update/Replace is `after - before`. If a side we need can't be priced,
/// the whole line is dropped to `not_estimated` — a half-real delta is worse than none.
fn price_change(rc: &ResourceChange, pricer: &dyn Pricer) -> Result<CostLine, NotEstimatedReason> {
    let after = if rc.action.adds_after() {
        Some(price_side(rc, SpecState::After, pricer)?)
    } else {
        None
    };
    let before = if rc.action.removes_before() {
        Some(price_side(rc, SpecState::Before, pricer)?)
    } else {
        None
    };

    let after_fixed = after.as_ref().map_or(0.0, |p| p.fixed_monthly.0);
    let before_fixed = before.as_ref().map_or(0.0, |p| p.fixed_monthly.0);
    // Surface the rate from the cost-bearing side: the new one if we add cost, else the old
    // one that goes away. Presentation (sign, prefix) is the renderer's job.
    let usage = after.and_then(|p| p.usage).or_else(|| before.and_then(|p| p.usage));

    // Label what was priced from the side that bears the cost.
    let detail_state = if rc.action.adds_after() { SpecState::After } else { SpecState::Before };

    Ok(CostLine {
        address: rc.address.clone(),
        rtype: rc.rtype.clone(),
        action: rc.action,
        detail: detail_for(rc, detail_state),
        delta_monthly: Money(after_fixed - before_fixed),
        usage,
    })
}

/// Price one side of a change, or say why not.
fn price_side(
    rc: &ResourceChange,
    state: SpecState,
    pricer: &dyn Pricer,
) -> Result<Price, NotEstimatedReason> {
    // Only the `after` side carries computed-unknowns; `before` is materialized state.
    if state == SpecState::After && price_keys(&rc.rtype).iter().any(|k| rc.is_unknown(k)) {
        return Err(NotEstimatedReason::ValueUnknown);
    }
    let attrs = rc.attributes(state).ok_or(NotEstimatedReason::Unpriced)?;
    pricer.price(&rc.rtype, attrs).ok_or(NotEstimatedReason::Unpriced)
}

/// A short label for what's being priced — the sku value(s) on the cost-bearing side, or
/// "fixed" when the type has no discriminator (a flat monthly charge).
fn detail_for(rc: &ResourceChange, state: SpecState) -> String {
    let keys = price_keys(&rc.rtype);
    if keys.is_empty() {
        return "fixed".into();
    }
    let detail = keys
        .iter()
        .filter_map(|k| rc.attr(state, k).and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    // Keys exist but none resolved to a string on this side — label it rather than blank.
    if detail.is_empty() {
        "?".into()
    } else {
        detail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Attributes;
    use crate::pricing::{MapPricer, PriceRow};
    use serde_json::{json, Value};

    fn attrs(pairs: &[(&str, Value)]) -> Attributes {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn row(sku: &str, fixed: f64, usage: Option<UsageRate>) -> PriceRow {
        PriceRow {
            resource_type: if sku.is_empty() { "aws_nat_gateway" } else { "aws_instance" }.into(),
            region: "us-east-1".into(),
            sku_key: sku.into(),
            price: Price { fixed_monthly: Money(fixed), usage },
        }
    }

    fn pricer() -> MapPricer {
        MapPricer::new(
            "us-east-1",
            vec![
                row("t3.small", 15.0, None),
                row("t3.large", 60.0, None),
                row("", 32.85, Some(UsageRate { rate: Money(0.045), unit: "GB processed".into() })),
            ],
        )
    }

    fn change(
        rtype: &str,
        action: Action,
        before: Option<Attributes>,
        after: Option<Attributes>,
        after_unknown: Option<Attributes>,
    ) -> ResourceChange {
        ResourceChange {
            address: format!("{rtype}.x"),
            rtype: rtype.into(),
            name: "x".into(),
            action,
            before,
            after,
            after_unknown,
        }
    }

    fn instance(action: Action, before: Option<&str>, after: Option<&str>) -> ResourceChange {
        let side = |t: Option<&str>| t.map(|t| attrs(&[("instance_type", json!(t))]));
        change("aws_instance", action, side(before), side(after), None)
    }

    fn approx(got: Money, want: f64) -> bool {
        (got.0 - want).abs() < 1e-6
    }

    #[test]
    fn create_adds_after_cost() {
        let report = compute(&[instance(Action::Create, None, Some("t3.large"))], &pricer());
        assert_eq!(report.lines.len(), 1);
        assert_eq!(report.lines[0].detail, "t3.large");
        assert!(approx(report.lines[0].delta_monthly, 60.0));
        assert!(approx(report.net_monthly, 60.0));
        assert!(report.not_estimated.is_empty());
    }

    #[test]
    fn delete_removes_before_cost() {
        let report = compute(&[instance(Action::Delete, Some("t3.large"), None)], &pricer());
        assert!(approx(report.lines[0].delta_monthly, -60.0));
        assert!(approx(report.net_monthly, -60.0));
    }

    #[test]
    fn update_is_after_minus_before() {
        let report =
            compute(&[instance(Action::Update, Some("t3.small"), Some("t3.large"))], &pricer());
        assert!(approx(report.lines[0].delta_monthly, 60.0 - 15.0)); // +45
    }

    #[test]
    fn replace_is_after_minus_before() {
        let report =
            compute(&[instance(Action::Replace, Some("t3.large"), Some("t3.small"))], &pricer());
        assert!(approx(report.lines[0].delta_monthly, 15.0 - 60.0)); // -45
    }

    #[test]
    fn usage_rate_is_carried_not_summed() {
        let nat = change("aws_nat_gateway", Action::Create, None, Some(attrs(&[])), None);
        let report = compute(&[nat], &pricer());
        let line = &report.lines[0];
        assert_eq!(line.detail, "fixed");
        assert!(approx(line.delta_monthly, 32.85));
        let usage = line.usage.as_ref().expect("nat carries a usage rate");
        assert!(approx(usage.rate, 0.045));
        assert_eq!(usage.unit, "GB processed");
        assert!(approx(report.net_monthly, 32.85)); // fixed only; usage never folded in
    }

    #[test]
    fn computed_price_key_is_value_unknown() {
        // instance_type is computed-at-apply: absent from `after`, flagged in `after_unknown`.
        let rc = change(
            "aws_instance",
            Action::Create,
            None,
            Some(attrs(&[("ami", json!("ami-1"))])),
            Some(attrs(&[("instance_type", json!(true))])),
        );
        let report = compute(&[rc], &pricer());
        assert!(report.lines.is_empty());
        assert_eq!(report.not_estimated[0].reason, NotEstimatedReason::ValueUnknown);
    }

    #[test]
    fn unmodeled_type_is_unpriced() {
        let rc = change("aws_unicorn", Action::Create, None, Some(attrs(&[])), None);
        let report = compute(&[rc], &pricer());
        assert_eq!(report.not_estimated[0].reason, NotEstimatedReason::Unpriced);
    }

    #[test]
    fn known_type_unknown_sku_is_unpriced() {
        let report = compute(&[instance(Action::Create, None, Some("x99.mega"))], &pricer());
        assert_eq!(report.not_estimated[0].reason, NotEstimatedReason::Unpriced);
    }

    #[test]
    fn net_sums_signed_deltas() {
        let report = compute(
            &[
                instance(Action::Create, None, Some("t3.large")),
                instance(Action::Delete, Some("t3.small"), None),
            ],
            &pricer(),
        );
        assert!(approx(report.net_monthly, 60.0 - 15.0)); // +45
    }

    #[test]
    fn noop_is_filtered_not_priced() {
        let report = compute(&[instance(Action::NoOp, Some("t3.large"), Some("t3.large"))], &pricer());
        assert!(report.lines.is_empty());
        assert!(report.not_estimated.is_empty());
        assert!(approx(report.net_monthly, 0.0));
    }

    #[test]
    fn delete_surfaces_before_side_usage() {
        // A NAT delete removes a fixed cost; its usage rate must come from the `before` side.
        let nat = change("aws_nat_gateway", Action::Delete, Some(attrs(&[])), None, None);
        let report = compute(&[nat], &pricer());
        let line = &report.lines[0];
        assert!(approx(line.delta_monthly, -32.85));
        let usage = line.usage.as_ref().expect("usage from the before side on a delete");
        assert!(approx(usage.rate, 0.045));
    }

    #[test]
    fn net_excludes_not_estimated() {
        let report = compute(
            &[
                instance(Action::Create, None, Some("t3.large")), // +60, priced
                change("aws_unicorn", Action::Create, None, Some(attrs(&[])), None), // unpriced
            ],
            &pricer(),
        );
        assert_eq!(report.lines.len(), 1);
        assert_eq!(report.not_estimated.len(), 1);
        assert!(approx(report.net_monthly, 60.0)); // only the priced line counts
    }
}
