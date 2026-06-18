//! Integration tests for the `costctl` binary — they pin the documented exit-code contract
//! (`0` ok / `1` error / `2` over budget) against a committed example plan.

use std::io::Write;
use std::process::{Command, Stdio};

/// Path to the committed web-cluster example plan (net ≈ +$203.38/mo with the dev catalog).
const PLAN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/web-cluster/plan.json");

fn run(args: &[&str]) -> Option<i32> {
    Command::new(env!("CARGO_BIN_EXE_costctl"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .code()
}

#[test]
fn under_budget_exits_0() {
    assert_eq!(run(&["--budget", "100000", PLAN]), Some(0));
}

#[test]
fn over_budget_exits_2() {
    assert_eq!(run(&["--budget", "100", PLAN]), Some(2));
}

#[test]
fn missing_file_exits_1() {
    assert_eq!(run(&["/no/such/plan.json"]), Some(1));
}

#[test]
fn usage_error_exits_1_not_2() {
    // A bad --budget value is a usage error: exit 1, not 2 (which means "over budget").
    assert_eq!(run(&["--budget", "not-a-number", PLAN]), Some(1));
}

#[test]
fn reads_plan_from_stdin() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_costctl"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let plan = std::fs::read(PLAN).unwrap();
    child.stdin.take().unwrap().write_all(&plan).unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(0));
}
