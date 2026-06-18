# Example: a mix of priced and not-yet-priced resources.
#
# The S3 bucket isn't in the price catalog, so costctl reports it under "Not estimated"
# instead of pretending it costs $0 — the honest-by-design behavior.
#
# Regenerate (no real AWS account needed: fake creds + skip flags):
#   cd examples/mixed
#   terraform init
#   AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test terraform plan -out plan.tfplan -refresh=false
#   terraform show -json plan.tfplan > plan.json
# Then, from the repo root:
#   cargo run -p costctl -- examples/mixed/plan.json

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

resource "aws_instance" "api" {
  ami           = "ami-0abcdef1234567890"
  instance_type = "t3.large"
}

resource "aws_s3_bucket" "assets" {
  bucket = "costctl-demo-assets"
}

resource "aws_nat_gateway" "main" {
  allocation_id = "eipalloc-0abc"
  subnet_id     = "subnet-0abc"
}
