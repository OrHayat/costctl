//! `costctl` core engine — the shared library linked by the CLI today and the platform
//! server later. Modules are added branch by branch up the stack:
//!
//! - `ir`        — canonical resource-change model        (01-core-ir)
//! - `adapters`  — adapt IaC plan formats into the IR     (02-tf-parser)
//! - `pricing`   — unit-price lookup from the price catalog (03-pricing)
//! - `cost`      — compute signed monthly deltas          (04-cost)
//! - `report`    — render text / JSON output              (05-report)

pub mod ir;
pub mod adapters;
pub mod pricing;
pub mod cost;
