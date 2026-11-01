#!/usr/bin/env bash
set -euo pipefail
./tools/import accounts --key account_code --backend rows
./tools/import contracts --key contract_ref --backend rows
./tools/import invoices --key invoice_number --backend rows
./tools/import products --key sku --backend rows
./tools/import suppliers --key vat_id --backend rows
./tools/import vehicles --key plate --backend rows
