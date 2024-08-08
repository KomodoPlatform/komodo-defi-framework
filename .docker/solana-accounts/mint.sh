#!/bin/bash

MINT_AMOUNT_USDC=1000000000
MINT_AMOUNT_ADEX=1000000000

OWNER_KEYPAIR=/root/accounts/iguana

solana config set --url http://127.0.0.1:8899
solana config set --keypair "$OWNER_KEYPAIR"

USDC_TOKEN=$(spl-token create-token --decimals 6 --mint-authority "$OWNER_KEYPAIR" --fee-payer "$OWNER_KEYPAIR" | grep -o -E 'Creating token [A-Za-z0-9]+' | awk '{print $3}')
ADEX_TOKEN=$(spl-token create-token --decimals 9 --mint-authority "$OWNER_KEYPAIR" --fee-payer "$OWNER_KEYPAIR" | grep -o -E 'Creating token [A-Za-z0-9]+' | awk '{print $3}')
ACCOUNT_FILE=/root/accounts/user3.json
spl-token create-account "$USDC_TOKEN" --owner "$OWNER_KEYPAIR"
spl-token create-account "$ADEX_TOKEN" --owner "$OWNER_KEYPAIR"

PUBKEY=$(jq -r '.pubkey' "$ACCOUNT_FILE")

USDC_ASSOCIATED_TOKEN_ADDRESS=$(spl-token accounts --owner "$PUBKEY" | grep "$USDC_TOKEN" | awk '{print $1}')
if [ -z "$USDC_ASSOCIATED_TOKEN_ADDRESS" ]; then
  USDC_ASSOCIATED_TOKEN_ADDRESS=$(spl-token create-account "$USDC_TOKEN" --owner "$PUBKEY" --fee-payer "$OWNER_KEYPAIR" | grep -o -E 'Creating account [A-Za-z0-9]+' | awk '{print $3}')
fi


ADEX_ASSOCIATED_TOKEN_ADDRESS=$(spl-token accounts --owner "$PUBKEY" | grep "$ADEX_TOKEN" | awk '{print $1}')
if [ -z "$ADEX_ASSOCIATED_TOKEN_ADDRESS" ]; then
  ADEX_ASSOCIATED_TOKEN_ADDRESS=$(spl-token create-account "$ADEX_TOKEN" --owner "$PUBKEY" --fee-payer "$OWNER_KEYPAIR" | grep -o -E 'Creating account [A-Za-z0-9]+' | awk '{print $3}')
fi
spl-token mint "$USDC_TOKEN" "$MINT_AMOUNT_USDC" --mint-authority "$OWNER_KEYPAIR" --recipient-owner "$PUBKEY" --fee-payer "$OWNER_KEYPAIR"
spl-token mint "$ADEX_TOKEN" "$MINT_AMOUNT_ADEX" --mint-authority "$OWNER_KEYPAIR" --recipient-owner "$PUBKEY" --fee-payer "$OWNER_KEYPAIR"

spl-token mint "$USDC_TOKEN" 100000
spl-token mint "$ADEX_TOKEN" 100000

echo "New token addresses:"
echo "USDC Token Address: $USDC_TOKEN"
echo "ADEX Token Address: $ADEX_TOKEN"
