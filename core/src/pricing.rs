//! Unit-price lookup.
//!
//! A [`Pricer`] answers "what does this resource spec cost per month?" for a single resource
//! type + its attributes. The cost engine asks for the `before` and `after` sides separately
//! and nets the deltas; the pricer itself is a pure, side-agnostic lookup.
//!
//! [`MapPricer`] is the default backend: the whole price catalog held in memory as a map.
//! It's enough because the catalog is small (v1 = an AWS subset) and read-only — it ships
//! beside the binary and runs offline. The [`Pricer`] trait is the swap point: if the catalog
//! ever outgrows RAM, a lazy/on-disk backend slots in here without touching the cost engine.

use crate::ir::Attributes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A USD amount.
///
/// `f64`-backed on purpose: this is a *forecast*, and the source prices are themselves
/// approximations, so float drift sits orders of magnitude below the estimate's uncertainty.
/// A newtype (not a bare `f64`) so rounding and formatting live in one place later.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Money(pub f64);

/// A usage-dependent rate that can't become a fixed total without knowing usage — e.g. NAT
/// gateway data processing. Reported as a unit rate so the variable part is shown, never faked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRate {
    /// Price per unit, e.g. `0.045`.
    pub rate: Money,
    /// What the rate is charged per, e.g. `"GB processed"`.
    pub unit: String,
}

/// The price of one resource spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Price {
    /// Fixed monthly cost — the always-on component, already normalized to a month.
    pub fixed_monthly: Money,
    /// Optional usage-based component, shown as a unit rate rather than a fabricated total.
    pub usage: Option<UsageRate>,
}

/// Looks up the price of a resource spec (a type + its attributes).
///
/// Returns `None` when the type/attrs aren't in the catalog ("unpriced"). Unknown/computed
/// attributes are *not* this layer's concern — the cost engine flags those and won't ask the
/// pricer to price a spec it can't pin down.
pub trait Pricer {
    fn price(&self, rtype: &str, attrs: &Attributes) -> Option<Price>;
}

/// One catalog entry: a `(resource_type, region, sku_key)` and its [`Price`]. This is the unit
/// the fetcher emits and the loader feeds to [`MapPricer`]; serde-ready for the shipped artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceRow {
    pub resource_type: String,
    pub region: String,
    /// The discriminating attribute value (e.g. `"t3.large"`); empty when the type has a
    /// single price with no discriminator (e.g. NAT gateway).
    pub sku_key: String,
    pub price: Price,
}

/// The in-memory price catalog for one region: `resource_type -> (sku_key -> Price)`.
///
/// Built for a fixed region — rows for other regions are dropped at construction, so the
/// region never enters the key. Lookups are then two borrowed `HashMap` probes with no
/// allocation on the hot path (only the matched `Price` is cloned, on a hit).
pub struct MapPricer {
    catalog: HashMap<String, HashMap<String, Price>>,
}

impl MapPricer {
    /// Build a pricer for `region` from catalog rows; rows for other regions are ignored.
    pub fn new(region: &str, rows: impl IntoIterator<Item = PriceRow>) -> Self {
        let mut catalog: HashMap<String, HashMap<String, Price>> = HashMap::new();
        for row in rows {
            if row.region == region {
                catalog.entry(row.resource_type).or_default().insert(row.sku_key, row.price);
            }
        }
        Self { catalog }
    }
}

impl Pricer for MapPricer {
    fn price(&self, rtype: &str, attrs: &Attributes) -> Option<Price> {
        let sku = sku_key_for(rtype, attrs)?;
        self.catalog.get(rtype)?.get(sku).cloned()
    }
}

/// Which attribute value discriminates the price for a resource type — its "sku key".
///
/// `None` means the type isn't modeled (→ unpriced). `Some("")` means the type has a single
/// price with no discriminator. `Some(v)` is the discriminating attribute's value; if that
/// attribute is absent (e.g. still computed) the spec can't be keyed, so it's also `None`.
fn sku_key_for<'a>(rtype: &str, attrs: &'a Attributes) -> Option<&'a str> {
    let attr = |key: &str| attrs.get(key).and_then(|v| v.as_str());
    match rtype {
        "aws_instance" => attr("instance_type"),
        "aws_db_instance" => attr("instance_class"),
        "aws_nat_gateway" => Some(""),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn attrs(pairs: &[(&str, Value)]) -> Attributes {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    /// `Money` is `f64`-backed, so compare with a tolerance rather than `==`.
    fn approx(got: Money, want: f64) -> bool {
        (got.0 - want).abs() < 1e-6
    }

    fn catalog() -> Vec<PriceRow> {
        vec![
            PriceRow {
                resource_type: "aws_instance".into(),
                region: "us-east-1".into(),
                sku_key: "t3.large".into(),
                // us-east-1 t3.large: $0.0832/hr ≈ $60.74/mo.
                price: Price { fixed_monthly: Money(60.736), usage: None },
            },
            PriceRow {
                resource_type: "aws_nat_gateway".into(),
                region: "us-east-1".into(),
                sku_key: String::new(),
                // NAT gateway: $0.045/hr ≈ $32.85/mo fixed, plus $0.045 per GB processed.
                price: Price {
                    fixed_monthly: Money(32.85),
                    usage: Some(UsageRate { rate: Money(0.045), unit: "GB processed".into() }),
                },
            },
        ]
    }

    #[test]
    fn prices_known_instance_type() {
        let pricer = MapPricer::new("us-east-1", catalog());
        let price = pricer
            .price("aws_instance", &attrs(&[("instance_type", json!("t3.large"))]))
            .expect("t3.large should be priced");
        assert!(approx(price.fixed_monthly, 60.736));
        assert!(price.usage.is_none());
    }

    #[test]
    fn nat_gateway_carries_a_usage_rate() {
        let pricer = MapPricer::new("us-east-1", catalog());
        // No discriminating attribute, yet it prices.
        let price = pricer.price("aws_nat_gateway", &attrs(&[])).expect("nat gateway should price");
        let usage = price.usage.expect("nat gateway has a usage rate");
        assert!(approx(usage.rate, 0.045));
        assert_eq!(usage.unit, "GB processed");
    }

    #[test]
    fn unmodeled_type_is_unpriced() {
        let pricer = MapPricer::new("us-east-1", catalog());
        assert!(pricer.price("aws_unicorn", &attrs(&[])).is_none());
    }

    #[test]
    fn known_type_with_sku_not_in_catalog_is_unpriced() {
        let pricer = MapPricer::new("us-east-1", catalog());
        let got = pricer.price("aws_instance", &attrs(&[("instance_type", json!("x99.mega"))]));
        assert!(got.is_none());
    }

    #[test]
    fn instance_without_its_type_attribute_is_unpriced() {
        let pricer = MapPricer::new("us-east-1", catalog());
        // instance_type missing (e.g. computed) -> can't be keyed.
        assert!(pricer.price("aws_instance", &attrs(&[("ami", json!("ami-1"))])).is_none());
    }

    #[test]
    fn other_region_is_unpriced() {
        // Catalog only holds us-east-1 rows; a eu-west-1 pricer finds nothing.
        let pricer = MapPricer::new("eu-west-1", catalog());
        let got = pricer.price("aws_instance", &attrs(&[("instance_type", json!("t3.large"))]));
        assert!(got.is_none());
    }

    #[test]
    fn price_row_round_trips_through_serde() {
        // Guards the derives the step-7 artifact relies on. Use the NAT row: it exercises
        // both Some(usage) and an empty sku_key. serde_json formats floats round-trip-exact,
        // so equality is sound here.
        let row = catalog().remove(1);
        let json = serde_json::to_string(&row).unwrap();
        assert_eq!(serde_json::from_str::<PriceRow>(&json).unwrap(), row);
    }
}
