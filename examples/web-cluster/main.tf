# Example: a small web cluster — every resource is priced by the built-in dev catalog.
#
# Regenerate the plan costctl reads (no real AWS account needed: fake creds + skip flags):
#   cd examples/web-cluster
#   terraform init
#   AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test terraform plan -out plan.tfplan -refresh=false
#   terraform show -json plan.tfplan > plan.json
# Then, from the repo root:
#   cargo run -p costctl -- examples/web-cluster/plan.json

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

resource "aws_instance" "web" {
  count         = 2
  ami           = "ami-0abcdef1234567890"
  instance_type = "m5.large"
}

resource "aws_instance" "bastion" {
  ami           = "ami-0abcdef1234567890"
  instance_type = "t3.medium"
}

resource "aws_nat_gateway" "main" {
  allocation_id = "eipalloc-0abc"
  subnet_id     = "subnet-0abc"
}
