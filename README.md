# costctl

Forecast the **monthly cost delta** of a Terraform change before you `apply` it.

`costctl` reads `terraform show -json` output, prices each resource change, and prints the
signed monthly difference — so a pull request can show `+$203/mo` instead of a surprise on next
month's bill.

```text
costctl: forecast for examples/web-cluster/plan.json  (prices: aws/us-east-1, catalog builtin-dev)

  CHANGE                  DETAIL     Δ MONTHLY
  + aws_instance.bastion  t3.medium  +$30.37
  + aws_instance.web[0]   m5.large   +$70.08
  + aws_instance.web[1]   m5.large   +$70.08
  + aws_nat_gateway.main  fixed      +$32.85   (+ $0.045/GB processed)

  Net monthly change:  +$203.38/mo
```

## Run

```sh
cargo run -p costctl -- examples/web-cluster/plan.json
```

Input is a `terraform show -json` plan (also accepts a `.tfplan`, or `-` for stdin):

```sh
terraform plan -out plan.tfplan
terraform show -json plan.tfplan > plan.json
cargo run -p costctl -- plan.json
```

| Flag           | Meaning                                              |
|----------------|-----------------------------------------------------|
| `--json`       | machine-readable output (stable schema)             |
| `--budget USD` | exit `2` if the net monthly increase exceeds `USD`  |
| `--region R`   | region to price against (default `us-east-1`)       |
| `--no-color`   | disable color (also off when stdout isn't a TTY)    |

Exit codes: `0` ok · `1` error · `2` over budget.

## What it can and can't estimate (by design)

Cloud cost comes in kinds, and `costctl` is honest about which it can total:

| Kind                  | Examples                                          | Output                                    |
|-----------------------|---------------------------------------------------|-------------------------------------------|
| **Config-priced**     | EC2 `instance_type`, RDS `instance_class`, NAT    | a real monthly total                      |
| **Usage-priced**      | Lambda, API Gateway, S3, on-demand DynamoDB       | a **unit rate** — never a fabricated total |
| **Not modeled / unknown** | anything else, or a computed-at-apply value   | listed under **Not estimated**            |

It never prints `$0` for something it can't price — that's the whole point. A NAT gateway shows
its fixed monthly *and* its `$/GB` rate separately; a value Terraform computes at apply is
flagged, not guessed.

> **Prices today are a small hand-entered dev catalog** (us-east-1; EC2 Linux/shared tenancy,
> RDS MySQL single-AZ, NAT) — approximate and **not authoritative**. Real, dimension-keyed
> prices will come from a price-catalog fetcher (see Roadmap).

## How it works

```
terraform show -json ─▶ adapter ─▶ IR ─▶ pricer ─▶ cost engine ─▶ renderer (text / JSON)
```

- **IR** — a tool-agnostic `ResourceChange` model, so Pulumi / CloudFormation are future adapters.
- **Pricer** — a flat in-memory catalog behind a trait (swappable for an on-disk backend).
- **Cost engine** — one signed-delta formula for every action; usage rates kept separate from the net.
- **Renderer** — aligned text, or a stable JSON schema.

Runnable demos live in [`examples/`](examples/).

## Roadmap

- **Fetcher** — distill real AWS offer files into a versioned, offline price catalog (no token, no server).
- **More resource types** — a declarative pricing-rules table (EBS, ALB/NLB, FSx, provisioned DynamoDB, …).
- **Track & reconcile** — actual spend vs. forecast.
- **Platform** — environment orchestration, scheduler, web UI.

## Development

Rust workspace: `core` (library) + `cli` (binary `costctl`).

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```
