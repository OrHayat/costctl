//! Render a cost [`Report`] for humans (`render_text`) or machines (`render_json`).
//!
//! The text renderer aligns columns, shows usage rates as a separate per-line suffix (never
//! folded into the net), and optionally colors amounts (the caller decides, e.g. by TTY).
//! The JSON renderer emits a stable, documented schema independent of the internal types.

use crate::cost::{NotEstimatedReason, Report};
use crate::ir::Action;
use crate::pricing::{Money, UsageRate};
use serde::Serialize;
use std::fmt::Write as _;

/// Header context the report itself doesn't carry — supplied by the caller.
pub struct RenderMeta<'a> {
    pub source: &'a str,
    pub region: &'a str,
    pub catalog_date: &'a str,
}

/// Render the report as aligned, optionally-colored text.
pub fn render_text(report: &Report, meta: &RenderMeta, color: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "costctl: forecast for {}  (prices: aws/{}, catalog {})",
        meta.source, meta.region, meta.catalog_date
    );

    let changes: Vec<String> = report
        .lines
        .iter()
        .map(|l| format!("{} {}", action_glyph(l.action), l.address))
        .collect();
    let wc = changes.iter().map(|s| s.chars().count()).max().unwrap_or(0).max("CHANGE".len());
    let wd =
        report.lines.iter().map(|l| l.detail.chars().count()).max().unwrap_or(0).max("DETAIL".len());

    if !report.lines.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "  {:<wc$}  {:<wd$}  Δ MONTHLY", "CHANGE", "DETAIL");
        for (line, change) in report.lines.iter().zip(&changes) {
            let amount = colorize(&money(line.delta_monthly), line.delta_monthly.0, color);
            let _ = writeln!(
                out,
                "  {change:<wc$}  {detail:<wd$}  {amount}{usage}",
                detail = line.detail,
                usage = usage_suffix(&line.usage),
            );
        }
    }

    let _ = writeln!(out);
    let net =
        colorize(&format!("{}/mo", money(report.net_monthly)), report.net_monthly.0, color);
    let _ = writeln!(out, "  Net monthly change:  {net}");

    if !report.not_estimated.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "  Not estimated ({}):", report.not_estimated.len());
        for ne in &report.not_estimated {
            let _ = writeln!(out, "    {} — {}", ne.address, reason_message(ne.reason));
        }
    }

    out
}

/// Render the report as a stable JSON document.
pub fn render_json(report: &Report, meta: &RenderMeta) -> String {
    let doc = JsonReport {
        source: meta.source,
        region: meta.region,
        catalog_date: meta.catalog_date,
        currency: "USD",
        net_monthly: report.net_monthly.0,
        lines: report
            .lines
            .iter()
            .map(|l| JsonLine {
                address: &l.address,
                resource_type: &l.rtype,
                action: action_str(l.action),
                detail: &l.detail,
                delta_monthly: l.delta_monthly.0,
                usage: l.usage.as_ref().map(|u| JsonUsage { rate: u.rate.0, unit: &u.unit }),
            })
            .collect(),
        not_estimated: report
            .not_estimated
            .iter()
            .map(|n| JsonNotEstimated {
                address: &n.address,
                resource_type: &n.rtype,
                reason: reason_str(n.reason),
            })
            .collect(),
    };
    serde_json::to_string_pretty(&doc).expect("a cost report always serializes")
}

// --- text helpers ---

fn money(m: Money) -> String {
    // Sign from the *rounded* cents, so a sub-cent value never prints as "-$0.00".
    let cents = (m.0 * 100.0).round();
    let sign = if cents < 0.0 { "-" } else { "+" };
    format!("{sign}${:.2}", cents.abs() / 100.0)
}

fn usage_suffix(usage: &Option<UsageRate>) -> String {
    match usage {
        Some(u) => format!("   (+ ${:.3}/{})", u.rate.0, u.unit),
        None => String::new(),
    }
}

/// Color an amount by direction: red raises cost, green lowers it, no color for ~zero.
fn colorize(text: &str, amount: f64, color: bool) -> String {
    // Sub-cent counts as zero (matching `money`'s rounding), so a $0.00 line isn't tinted.
    if !color || amount.abs() < 0.005 {
        return text.to_string();
    }
    let code = if amount > 0.0 { "31" } else { "32" };
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn action_glyph(action: Action) -> char {
    match action {
        Action::Create => '+',
        Action::Delete => '-',
        Action::Update => '~',
        Action::Replace => '±',
        Action::NoOp => ' ',
    }
}

fn reason_message(reason: NotEstimatedReason) -> &'static str {
    match reason {
        NotEstimatedReason::ValueUnknown => "value computed at apply",
        NotEstimatedReason::Unpriced => "not in price catalog",
    }
}

// --- JSON schema (stable wire format, decoupled from internal types) ---

fn action_str(action: Action) -> &'static str {
    match action {
        Action::Create => "create",
        Action::Update => "update",
        Action::Delete => "delete",
        Action::Replace => "replace",
        Action::NoOp => "no-op",
    }
}

fn reason_str(reason: NotEstimatedReason) -> &'static str {
    match reason {
        NotEstimatedReason::ValueUnknown => "value_unknown",
        NotEstimatedReason::Unpriced => "unpriced",
    }
}

#[derive(Serialize)]
struct JsonReport<'a> {
    source: &'a str,
    region: &'a str,
    catalog_date: &'a str,
    currency: &'static str,
    net_monthly: f64,
    lines: Vec<JsonLine<'a>>,
    not_estimated: Vec<JsonNotEstimated<'a>>,
}

#[derive(Serialize)]
struct JsonLine<'a> {
    address: &'a str,
    resource_type: &'a str,
    action: &'static str,
    detail: &'a str,
    delta_monthly: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<JsonUsage<'a>>,
}

#[derive(Serialize)]
struct JsonUsage<'a> {
    rate: f64,
    unit: &'a str,
}

#[derive(Serialize)]
struct JsonNotEstimated<'a> {
    address: &'a str,
    resource_type: &'a str,
    reason: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::{CostLine, NotEstimated};

    fn meta() -> RenderMeta<'static> {
        RenderMeta { source: "plan.json", region: "us-east-1", catalog_date: "2026-06-10" }
    }

    fn sample() -> Report {
        Report {
            lines: vec![
                CostLine {
                    address: "aws_instance.web".into(),
                    rtype: "aws_instance".into(),
                    action: Action::Create,
                    detail: "t3.large".into(),
                    delta_monthly: Money(60.0),
                    usage: None,
                },
                CostLine {
                    address: "aws_nat_gateway.main".into(),
                    rtype: "aws_nat_gateway".into(),
                    action: Action::Create,
                    detail: "fixed".into(),
                    delta_monthly: Money(32.85),
                    usage: Some(UsageRate { rate: Money(0.045), unit: "GB processed".into() }),
                },
                CostLine {
                    address: "aws_db_instance.legacy".into(),
                    rtype: "aws_db_instance".into(),
                    action: Action::Delete,
                    detail: "db.t3.medium".into(),
                    delta_monthly: Money(-49.64),
                    usage: None,
                },
            ],
            net_monthly: Money(60.0 + 32.85 - 49.64),
            not_estimated: vec![NotEstimated {
                address: "aws_lambda.handler".into(),
                rtype: "aws_lambda".into(),
                reason: NotEstimatedReason::Unpriced,
            }],
        }
    }

    #[test]
    fn text_shows_header_rows_net_and_not_estimated() {
        let out = render_text(&sample(), &meta(), false);
        assert!(out.contains("forecast for plan.json"));
        assert!(out.contains("aws/us-east-1, catalog 2026-06-10"));
        assert!(out.contains("+ aws_instance.web"));
        assert!(out.contains("t3.large"));
        assert!(out.contains("+$60.00"));
        assert!(out.contains("(+ $0.045/GB processed)")); // usage shown as a rate
        assert!(out.contains("- aws_db_instance.legacy"));
        assert!(out.contains("-$49.64"));
        assert!(out.contains("Net monthly change:  +$43.21/mo")); // fixed deltas only
        assert!(out.contains("Not estimated (1):"));
        assert!(out.contains("aws_lambda.handler — not in price catalog"));
    }

    #[test]
    fn columns_align_and_plain_text_has_no_ansi() {
        let out = render_text(&sample(), &meta(), false);
        assert!(!out.contains('\x1b'));
        // Detail column is padded to a common width, so amounts start at the same column.
        // Compare display columns (char count), not byte offsets — glyphs can be multi-byte.
        let col = |needle: &str| {
            let line = out.lines().find(|l| l.contains(needle)).unwrap();
            line.chars().take_while(|&c| c != '$').count()
        };
        assert_eq!(col("aws_instance.web"), col("aws_db_instance.legacy"));
    }

    #[test]
    fn color_enabled_emits_ansi() {
        assert!(render_text(&sample(), &meta(), true).contains("\x1b["));
    }

    #[test]
    fn json_is_valid_and_stable() {
        let json = render_json(&sample(), &meta());
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["currency"], "USD");
        assert_eq!(v["region"], "us-east-1");
        assert!((v["net_monthly"].as_f64().unwrap() - 43.21).abs() < 1e-6);
        assert_eq!(v["lines"].as_array().unwrap().len(), 3);
        assert_eq!(v["lines"][0]["action"], "create");
        assert_eq!(v["lines"][0]["detail"], "t3.large");
        assert_eq!(v["lines"][1]["usage"]["unit"], "GB processed");
        assert!(v["lines"][0].get("usage").is_none()); // omitted when absent
        assert_eq!(v["not_estimated"][0]["reason"], "unpriced");
    }

    #[test]
    fn replace_row_aligns_with_multibyte_glyph() {
        // `±` is multi-byte; alignment must hold by display column, not byte offset.
        let line = |addr: &str, action, detail: &str, delta| CostLine {
            address: addr.into(),
            rtype: "aws_instance".into(),
            action,
            detail: detail.into(),
            delta_monthly: Money(delta),
            usage: None,
        };
        let report = Report {
            lines: vec![
                line("aws_instance.a", Action::Create, "t3.large", 60.0),
                line("aws_instance.b", Action::Replace, "t3.small", -45.0),
            ],
            net_monthly: Money(15.0),
            not_estimated: vec![],
        };
        let out = render_text(&report, &meta(), false);
        let col = |needle: &str| {
            let l = out.lines().find(|l| l.contains(needle)).unwrap();
            l.chars().take_while(|&c| c != '$').count()
        };
        assert!(out.contains("± aws_instance.b"));
        assert_eq!(col("aws_instance.a"), col("aws_instance.b"));
    }

    #[test]
    fn json_action_and_reason_strings_are_stable() {
        let line = |action| CostLine {
            address: "x.y".into(),
            rtype: "aws_instance".into(),
            action,
            detail: "t3.large".into(),
            delta_monthly: Money(1.0),
            usage: None,
        };
        let report = Report {
            lines: vec![
                line(Action::Create),
                line(Action::Update),
                line(Action::Delete),
                line(Action::Replace),
            ],
            net_monthly: Money(4.0),
            not_estimated: vec![
                NotEstimated {
                    address: "a".into(),
                    rtype: "t".into(),
                    reason: NotEstimatedReason::ValueUnknown,
                },
                NotEstimated { address: "b".into(), rtype: "t".into(), reason: NotEstimatedReason::Unpriced },
            ],
        };
        let v: serde_json::Value = serde_json::from_str(&render_json(&report, &meta())).unwrap();
        let actions: Vec<&str> =
            v["lines"].as_array().unwrap().iter().map(|l| l["action"].as_str().unwrap()).collect();
        assert_eq!(actions, ["create", "update", "delete", "replace"]);
        let reasons: Vec<&str> = v["not_estimated"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["reason"].as_str().unwrap())
            .collect();
        assert_eq!(reasons, ["value_unknown", "unpriced"]);
    }
}
