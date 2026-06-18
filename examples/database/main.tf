# Example: a database tier.
#
# RDS is priced by `instance_class` (now in the built-in catalog). The DynamoDB table isn't
# modeled yet, so costctl reports it under "Not estimated" rather than a fake $0 — even though
# its provisioned capacity (read/write_capacity) is a knowable price. Wiring that up is next.
#
# Regenerate (no real AWS account needed: fake creds + skip flags):
#   cd examples/database
#   terraform init
#   AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test \
#     terraform plan -out plan.tfplan -refresh=false
#   terraform show -json plan.tfplan > plan.json
# Then, from the repo root:
#   cargo run -p costctl -- examples/database/plan.json

terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }
}

provider "aws" {
  region                      = "us-east-1"
  skip_credentials_validation = true
  skip_requesting_account_id  = true
  skip_metadata_api_check     = true
}

resource "aws_db_instance" "primary" {
  identifier                  = "costctl-demo-db"
  engine                      = "mysql"
  instance_class              = "db.m5.large"
  allocated_storage           = 20
  username                    = "appuser"
  manage_master_user_password = true # RDS manages the secret; no password in source
  skip_final_snapshot         = true
}

resource "aws_dynamodb_table" "sessions" {
  name           = "costctl-demo-sessions"
  billing_mode   = "PROVISIONED"
  hash_key       = "id"
  read_capacity  = 5
  write_capacity = 5

  attribute {
    name = "id"
    type = "S"
  }
}
