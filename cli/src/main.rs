//! `costctl` — forecast the monthly cost delta of a Terraform plan before you apply it.
//!
//! Pipeline: read a `terraform show -json` plan → parse to the IR → price each change → render
//! a forecast. Exit codes: `0` ok, `1` error, `2` the net monthly change exceeds `--budget`.

use std::io::{IsTerminal, Read, Write};
use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use clap::Parser;
use costctl_core::adapters::terraform;
use costctl_core::cost;
use costctl_core::pricing::{MapPricer, Money, Price, PriceRow, UsageRate};
use costctl_core::report::{self, RenderMeta};

/// Forecast the monthly cost delta of a Terraform plan before you apply it.
#[derive(Parser)]
#[command(name = "costctl", version, about)]
struct Cli {
    /// Plan to read: a `terraform show -json` file, a `.tfplan` (parsed via terraform), or `-` for stdin.
    #[arg(default_value = "-")]
    plan: String,

    /// Emit JSON instead of text.
    #[arg(long)]
    json: bool,

    /// Fail (exit 2) when the net monthly increase exceeds this many dollars.
    #[arg(long, value_name = "USD")]
    budget: Option<f64>,

    /// Region to price against.
    #[arg(long, default_value = "us-east-1")]
    region: String,

    /// Disable ANSI color (also auto-disabled when stdout is not a terminal).
    #[arg(long)]
    no_color: bool,
}

fn main() -> ExitCode {
    // Handle parsing ourselves so usage errors exit 1, leaving exit code 2 exclusively for
    // "over budget" (clap's default would also exit 2 on a usage error). --help / --version
    // aren't errors: print and exit 0.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return match e.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    ExitCode::SUCCESS
                }
                _ => ExitCode::FAILURE,
            };
        }
    };
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("costctl: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let plan_json = read_plan(&cli.plan)?;
    let changes =
        terraform::parse_plan(plan_json.as_bytes()).context("parsing the terraform plan")?;

    let pricer = MapPricer::new(&cli.region, builtin_catalog());
    let report = cost::compute(&changes, &pricer);

    let source = if cli.plan == "-" { "<stdin>" } else { cli.plan.as_str() };
    let meta = RenderMeta { source, region: &cli.region, catalog_date: CATALOG_DATE };
    let output = if cli.json {
        report::render_json(&report, &meta)
    } else {
        let color = !cli.no_color
            && std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal();
        report::render_text(&report, &meta, color)
    };
    // Write directly so a closed pipe (e.g. `costctl … | head`) is a clean exit, not a panic.
    if let Err(e) = writeln!(std::io::stdout(), "{}", output.trim_end()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(ExitCode::SUCCESS);
        }
        return Err(e).context("writing output");
    }

    if let Some(budget) = cli.budget {
        if report.net_monthly.0 > budget {
            eprintln!(
                "costctl: net monthly change +${:.2} exceeds budget ${:.2}",
                report.net_monthly.0, budget
            );
            return Ok(ExitCode::from(2));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Read the plan as `terraform show -json` text: from stdin (`-`), a `.tfplan` (run through
/// terraform), or a plain JSON file.
fn read_plan(path: &str) -> Result<String> {
    if path == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).context("reading plan from stdin")?;
        return Ok(buf);
    }
    if path.ends_with(".tfplan") {
        return terraform_show_json(path);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {path}"))
}

/// Run `terraform show -json <plan>` and return its stdout.
fn terraform_show_json(plan: &str) -> Result<String> {
    let output = Command::new("terraform")
        .args(["show", "-json", plan])
        .output()
        .context("running `terraform show -json` (is terraform on PATH?)")?;
    if !output.status.success() {
        bail!("terraform show failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    String::from_utf8(output.stdout).context("terraform output was not UTF-8")
}

/// Date label for the built-in catalog (see [`builtin_catalog`]).
const CATALOG_DATE: &str = "builtin-dev";

/// A tiny built-in price catalog so the CLI works end-to-end before the fetcher (step 7) ships
/// a real one. Hand-entered and **approximate — NOT authoritative**, for a single baseline:
/// us-east-1 on-demand, EC2 = Linux / shared tenancy, RDS = MySQL single-AZ. Real prices vary
/// by OS, engine, tenancy, and AZ — dimensions this stopgap ignores and the fetcher keys on.
/// Monthly = hourly × 730.
fn builtin_catalog() -> Vec<PriceRow> {
    let row = |rtype: &str, sku: &str, hourly: f64, usage: Option<UsageRate>| PriceRow {
        resource_type: rtype.into(),
        region: "us-east-1".into(),
        sku_key: sku.into(),
        price: Price { fixed_monthly: Money(hourly * 730.0), usage },
    };
    let ec2 = |sku: &str, hourly: f64| row("aws_instance", sku, hourly, None);
    let rds = |sku: &str, hourly: f64| row("aws_db_instance", sku, hourly, None);
    vec![
        // EC2 on-demand
        ec2("t3.micro", 0.0104),
        ec2("t3.small", 0.0208),
        ec2("t3.medium", 0.0416),
        ec2("t3.large", 0.0832),
        ec2("c5.large", 0.085),
        ec2("r5.large", 0.126),
        ec2("m5.large", 0.096),
        ec2("m5.xlarge", 0.192),
        // RDS on-demand, single-AZ
        rds("db.t3.micro", 0.017),
        rds("db.t3.small", 0.034),
        rds("db.t3.medium", 0.068),
        rds("db.m5.large", 0.171),
        // NAT gateway: hourly + per-GB processed
        row(
            "aws_nat_gateway",
            "",
            0.045,
            Some(UsageRate { rate: Money(0.045), unit: "GB processed".into() }),
        ),
    ]
}
