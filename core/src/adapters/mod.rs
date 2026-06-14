//! Adapters turn a specific IaC tool's plan output into the normalized [`crate::ir`].
//!
//! Terraform is the only adapter today; Pulumi and CloudFormation slot in here later
//! without touching the pricing / cost / report layers downstream. (Named "adapters", not
//! "frontends", to avoid clashing with the web UI built in the platform tier.)

pub mod terraform;
