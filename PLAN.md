# Plan: `costctl` — a cost-aware infrastructure tool (CLI → platform)

## Context

Greenfield project; `/Users/orhayat/cl` is empty and not a git repo. What began as a
pre-apply cost *estimator* grew, through discussion, into a **cost-aware infrastructure
lifecycle tool**: forecast spend before apply, track what it actually cost after, reconcile
the two, and eventually orchestrate/schedule environments to control that cost. The CLI is
the first deliverable; the architecture is laid out so the later platform is an extension,
not a rewrite.

Toolchain confirmed: Rust 1.91.1 (rustc/cargo), Terraform 1.5.7.

## Product vision (tiers) — only Tier 1 is v1

```
Tier 1 (v1)   CLI: FORECAST    plan → projected monthly cost          offline, no server
Tier 2        CLI: TRACK        billing APIs → actual spend, by usage   needs user cloud creds
              CLI: RECONCILE    forecast vs actual → drift              (falls out of T1+T2)
Tier 3        PLATFORM          env orchestration, scheduler, web UI,   needs a control-plane
                                RBAC/ACL                                server
```

## Resolved decisions (the "why", settled in discussion)

- **All-Rust where the engine is shared.** Single Cargo workspace; `core` is a library that
  `cli` (now) and `server` (later) both link — **no FFI/WASM/cgo**. The one polyglot seam is
  the TS/React web UI over an HTTP API (forced — nobody writes the browser layer in Rust).
- **Not Go for the server:** its only edges were mature multi-cloud SDKs + faster server dev.
  v1 is AWS-only (AWS Rust SDK is solid), Azure/GCP billing+pricing are plain REST via
  `reqwest`, orchestration shells out to `terraform`/`pulumi`. Accepted cost: Rust async
  server (tokio/axum) is heavier to learn / slower to compile.
- **Prices:** periodic maintainer-run fetcher → hosted, versioned **flat-file catalog**
  (a serialized map, bincode + zstd) → client downloads/caches, runs **offline, no user
  token**; checksummed + bundled fallback. Not a DB: the catalog is small and read-only and
  lookups are pure point queries — a `Pricer` trait is the swap point if it outgrows RAM.
- **Tracking uses the user's own cloud creds** (actual spend is their private bill). Cost
  Explorer for finalized actuals (~24h lag) + optional CloudWatch live estimate, labeled.
- **Usage-based costs:** v1 = split report (fixed $ + unit rate, flag the variable part;
  never fabricate). Optional `usage.yml` is Phase 2.
- **Attribution: tag-based** — IaC stamps a stack/owner tag; billing grouped by tag + usage-type.
- **Input:** Terraform plan JSON first, behind a normalized IR so Pulumi/CFN are later adapters.
- **Terraform compatibility:** accept plan `format_version` **1.x** (Terraform 1.1+); reject other
  JSON as "not a plan." Unknown fields are ignored (forward-compatible within 1.x). The binary
  `.tfplan` from `-out` is an internal/unstable format — never parsed directly.
- **HCL:** out of v1 — a future second adapter (no-plan-needed, best-effort eval, less accurate).

## Research findings (validated, with sources)

- **Terraform plan schema** ([json-format docs](https://developer.hashicorp.com/terraform/internals/json-format)):
  `resource_changes[]` has `address, type, name, mode, module_address, index, change`.
  `change.actions` ∈ `["no-op"] | ["create"] | ["read"] | ["update"] | ["delete"] |
  ["create","delete"] | ["delete","create"]` (last two = replace). Docs' own tip: *scan for
  `"delete"`* to catch all destruction cases. Use `change.after` for create/update/replace,
  `change.before` for delete; `change.after_unknown` marks computed leaves `true` → if a
  cost-relevant attr is unknown, report "not estimated", never $0. Filter `mode == "managed"`
  (data sources cost nothing).
- **AWS prices are public, no creds** ([bulk API docs](https://docs.aws.amazon.com/awsaccountbilling/latest/aboutv2/finding-prices-in-service-price-list-files.html)):
  offer files = `products[SKU].attributes` (instanceType, location, OS, tenancy…) + `terms.OnDemand[SKU][term].priceDimensions[dim].pricePerUnit.USD`. The global EC2 file is multi-GB,
  but **per-region** files (`…/AmazonEC2/current/us-east-1/index.json`) are far smaller — the
  fetcher distills these into the compact serialized catalog snapshot.
- **Azure Retail Prices API is public/unauthenticated** ([MS docs](https://learn.microsoft.com/en-us/rest/api/cost-management/retail-prices/azure-retail-prices)) — `https://prices.azure.com/api/retail/prices`, OData filters. Proves the live-fetch path for a future provider.
- **Prior art — Infracost** ([cloud pricing API](https://www.infracost.io/docs/supported_resources/cloud_pricing_api/)):
  same model (parse plan → look up prices; never sends the plan or creds), but relies on a
  hosted GraphQL pricing **server** (10M+ prices, weekly job). **Our differentiator:** an
  offline, self-contained catalog snapshot (no server) + the Track/Reconcile/platform tiers.
- **Storage = flat-file map, not a DB.** The catalog is read-only and small (v1 = an AWS
  subset), accessed by exact-key point lookups — no joins, ranges, or concurrency. A
  serialized map loaded into a `HashMap` needs no extra dependency and ships as one file.
  SQLite / embedded-KV stay behind the `Pricer` trait as the "if it outgrows RAM" backend.

## Architecture

```
adapters                  normalized IR            analysis            renderers
terraform show -json  ┐                        ┌─ pricing (forecast) ┐
(pulumi/cfn later)    ┼──►  ResourceChange ────┼─ billing  (track)   ┼──► text / JSON / (TUI)
                      ┘     (canonical enum)   └─ reconcile (drift)  ┘
sharing:  core (lib) ─linked by─► cli (bin) & server (bin);  web/ (TS) → server API
```

## Delivery: git-town stack (stacked PRs; remote `OrHayat/costctl`, trunk `master`)

Built as a **stack of branches** — each a child of the one below, each an independently
reviewable PR that compiles and passes tests on its own. A crate is introduced **only on the
branch that implements it**, so there are **no stub binaries** anywhere.

```
master           clean baseline: PLAN.md + .gitignore
└─ 01-core-ir    workspace (core lib only) + IR model            [Steps 0–1]
   └─ 02-tf-parser   terraform plan parser + golden fixture      [Step 2]
      └─ 03-pricing  Pricer trait + MapPricer (in-mem catalog)   [Step 3]
         └─ 04-cost  cost diff engine                            [Step 4]
            └─ 05-report  text + JSON renderers                  [Step 5]
               └─ 06-cli     add cli crate — real wiring         [Step 6]
                  └─ 07-fetcher  add fetcher crate + DB refresh  [Step 7]
                     └─ 08-polish   README, clippy, ship         [Step 8]
```

git-town: `git town append <branch>` to stack a child, `git town sync` to keep the stack
current, `git town propose` to open PRs (after a remote exists).

---

# Implementation steps

Each step is independently buildable and has a **Done** bar; the stack above maps steps to
branches. Steps 0–8 = v1. Tier 2/3 after.

### Step 0 — Repo + workspace (folded into branch `01-core-ir`)
- `git init`; Rust `.gitignore` (`/target`); `PLAN.md` committed to `master` as the stack baseline.
- Cargo **workspace with only the `core` library crate** — *no* `cli`/`fetcher` stub binaries
  (those are added on their own branches at Steps 6–7). Deps used by `core`:
  `serde`/`serde_json`, `thiserror` (no `rusqlite` — pricing is a flat-file map; `anyhow`
  lands with the `cli`).
- **Done:** `cargo build -p costctl-core` compiles (verified together with Step 1).

### Step 1 — IR model (`core/src/ir.rs`)
- `ResourceChange { address, rtype, name, action, before, after }`; `Action` enum
  (Create/Update/Delete/Replace/NoOp); `SpecState` selector (before vs after).
- **Done:** types compile; a unit test constructs each `Action`.

### Step 2 — Terraform parser (`core/src/adapters/terraform.rs`)
- serde structs for the *subset* of `terraform show -json`: `resource_changes[].{type, name,
  address, mode, change{actions, before, after, after_unknown}}`. `parse_plan(reader) ->
  Vec<ResourceChange>`.
- Action mapping (per research): `["create"]`→Create, `["update"]`→Update, `["delete"]`→Delete,
  `["create","delete"]`/`["delete","create"]`→Replace, `["no-op"]`/`["read"]`→skip. Filter
  `mode == "managed"`. Pick attrs from `after` (create/update/replace) or `before` (delete);
  if a cost-relevant attr appears in `after_unknown`, flag it unknown for "not estimated".
- Validate it's a plan: require `format_version` (else "not a plan"); accept major **1.x** only.
- Compat-matrix test: real plans from Terraform 1.1.9 / 1.5.7 / 1.9.8 (format_version 1.0–1.2) all parse.
- Generate a **real** plan and commit it as `testdata/sample-plan.json` (see Verification).
- **Done:** golden test parses the fixture, incl. a Replace and a computed-unknown attribute.

### Step 3 — Pricing read path (`core/src/pricing.rs`)
- `Pricer` trait: `price(rtype, &attrs) -> Option<Price>` (`None` = unpriced). `MapPricer`
  holds the catalog in memory, keyed by `(type, region, sku)`, built for a fixed region;
  `sku_key_for` picks the per-type discriminator (EC2→`instance_type`, RDS→`instance_class`,
  NAT→none). `Price { fixed_monthly, usage: Option<UsageRate> }` — variable costs as a unit
  rate, never fabricated. `PriceRow` derives serde for the Step-7 artifact.
- Real catalog is distilled by the fetcher (Step 7); the dev test builds an inline catalog
  (no seed file). Unknown/computed attrs are the cost engine's concern, not the pricer's.
- **Done:** unit test prices `aws_instance`=t3.large; returns `None` for an unmodeled type.

### Step 4 — Cost diff engine (`core/src/cost.rs`)
- `compute(changes, &dyn Pricer) -> Report { lines, net_monthly, not_estimated }`; signed
  deltas (create +, delete −, replace/update = after − before); collect unknowns separately.
- **Done:** sign-logic unit test covers all four actions.

### Step 5 — Report rendering (`core/src/report.rs`)
- `render_text` (aligned columns, color gated on TTY, split-report unit rates, "not
  estimated" section — matches the UX mock below) + `render_json` (stable schema).
- **Done:** text output matches the mock on the fixture; JSON validates.

### Step 6 — CLI wiring — adds the `cli` crate (branch `06-cli`)
- Add the `cli` crate as a workspace member (bin name `costctl`) — the first binary, fully wired.
- Accept the plan JSON directly; also accept a `.tfplan` by shelling out to
  `terraform show -json` internally (needs `terraform` on PATH).
- clap: positional `plan.json` or stdin; flags `--json`, `--budget N`, `--no-color`,
  `--offline`. Pipeline: parse → compute → render → budget check. Exit: 0 ok / 1 error /
  2 budget exceeded.
- **Done:** `costctl plan.json` prints the report on the *real* generated plan; `--budget`
  exits non-zero.

### Step 7 — Price catalog fetcher + refresh — adds the `fetcher` crate (branch `07-fetcher`)
- `fetcher/`: download AWS **per-region** offer files (e.g.
  `…/AmazonEC2/current/us-east-1/index.json` — far smaller than the multi-GB global file),
  walk `products[SKU].attributes` → `terms.OnDemand[SKU][…].priceDimensions[…].pricePerUnit.USD`,
  convert hourly → monthly (×730), and distill into a compact date-versioned catalog
  (serialized `PriceRow`s, bincode + zstd) + a small manifest carrying the sha256.
- Catalog loader in `core`: refresh only past a TTL (~24h), `ETag`/`304` so the unchanged
  case is ~0 bytes; on change, verify checksum → atomic swap into the cache dir; fall back to
  the bundled snapshot, warn if stale; `--offline` skips refresh. Per-region files keep
  transfers small; a CDN + immutable per-version files absorb the client fan-out.
- **Done:** fetcher emits a catalog the CLI loads; offline run uses the bundled fallback.

### Step 8 — Polish + README
- README: usage + the honest "what it can / can't estimate" boundary (fixed vs usage-based).
  `cargo clippy --workspace` clean; the two targeted tests green.
- **Done:** v1 is shippable.

---

## UX (the target for Step 5)

```
costctl: forecast for plan.json  (prices: aws/us-east-1, db 2026-06-10)

  CHANGE                       DETAIL              Δ MONTHLY
  + aws_instance.web (×3)      t3.large            +$190.46
  + aws_nat_gateway.main       fixed               +$32.85   (+ $0.045/GB processed)
  - aws_db_instance.legacy     db.t3.medium        -$49.64

  Net monthly change:  +$173.67/mo

  Not estimated (1): aws_lambda.handler — usage-dependent (invocations, duration)
```
Honest-by-design: variable costs show the *unit rate*, never a fabricated total.

## Verification (end to end, used by Steps 2 & 6)
```
mkdir -p /tmp/costctl-demo && cd /tmp/costctl-demo
cat > main.tf <<'EOF'
provider "aws" { region = "us-east-1" }
resource "aws_instance" "web" { count = 3  ami = "ami-000"  instance_type = "t3.large" }
resource "aws_nat_gateway" "main" { allocation_id = "eipalloc-000"  subnet_id = "subnet-000" }
EOF
terraform init -backend=false
terraform plan -out plan.tfplan -refresh=false
terraform show -json plan.tfplan > plan.json
```
Run `costctl plan.json`; confirm totals vs a hand calc (t3.large ≈ $0.0832/hr × 730 × 3 +
NAT $0.045/hr × 730). Copy `plan.json` → `testdata/sample-plan.json` as the golden fixture.
`costctl --budget 100 plan.json; echo $?` → report + non-zero exit.

## Roadmap (post-v1, coarse steps)
- **Tier 2 — Track:** parse `terraform show -json` of *state* for inventory; query Cost
  Explorer grouped by tag + usage-type for actuals; CloudWatch live estimate.
- **Tier 2 — Reconcile:** diff forecast vs actual → drift report. **Usage file:** optional
  `usage.yml` turns split-report unit rates into concrete totals.
- **Tier 3 — Platform:** `server` crate (axum + sqlx) over the same `core`; env/stack
  orchestration (wraps apply/destroy); **scheduler** (timed spin-up/tear-down = the biggest
  cost lever); web UI (TS/React) dashboards; RBAC/ACL; Pulumi/CFN IR adapters.
- **More adapters:** HCL (no-plan-needed, best-effort var/module eval — can't tell create vs
  replace without state, so less accurate than plan JSON); Pulumi `preview --json`; CloudFormation.
