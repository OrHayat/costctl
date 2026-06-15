# Examples

Runnable demos for `costctl`. Each folder has a Terraform config (`main.tf`) and a
pre-generated plan (`plan.json`), so you can run the tool immediately — no AWS account needed.

## Run (from the repo root)

```sh
cargo run -p costctl -- examples/web-cluster/plan.json
cargo run -p costctl -- examples/mixed/plan.json

# JSON output, or gate on a monthly budget (exit code 2 if exceeded):
cargo run -p costctl -- --json examples/web-cluster/plan.json
cargo run -p costctl -- --budget 100 examples/web-cluster/plan.json; echo "exit=$?"
```

## The examples

| Folder        | Shows                                                           |
|---------------|-----------------------------------------------------------------|
| `web-cluster` | A fully-priced plan: 2× m5.large + 1× t3.medium + a NAT gateway |
| `mixed`       | A priced plan plus an S3 bucket that lands in "Not estimated"   |
| `database`    | RDS priced by instance_class; a DynamoDB table not yet modeled  |

## Regenerating `plan.json` from `main.tf`

`costctl` reads `terraform show -json` output, not `.tf` directly. To regenerate after editing a
config — no real AWS account required, the provider uses fake creds + skip flags:

```sh
cd examples/web-cluster
terraform init
AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test \
  terraform plan -out plan.tfplan -refresh=false
terraform show -json plan.tfplan > plan.json
```

> Prices come from a small built-in dev catalog (us-east-1), so `catalog_date` reads
> `builtin-dev`. The fetcher (step 7) will replace it with a real, versioned catalog.
