extern crate serde_derive;

use crate::{NumConversResult, SolanaCoin, SolanaFeeDetails, TransactionDetails, TransactionType};
use mm2_number::BigDecimal;
use solana_sdk::native_token::lamports_to_sol;
use std::convert::TryFrom;

#[derive(Debug, Serialize, Deserialize)]
pub struct SolanaConfirmedTransaction {
    slot: u64,
    transaction: Transaction,
    meta: Meta,
    #[serde(rename = "blockTime")]
    block_time: u64,
}

#[allow(dead_code)]
impl SolanaConfirmedTransaction {
    pub fn extract_account_index(&self, address: String) -> usize {
        // find the equivalent of index_of(needle) in rust, and return result later
        let mut idx = 0_usize;
        for account in self.transaction.message.account_keys.iter() {
            if account.pubkey == address {
                return idx;
            }
            idx += 1;
        }
        idx
    }

    pub fn extract_solana_transactions(&self, solana_coin: &SolanaCoin) -> NumConversResult<Vec<TransactionDetails>> {
        let mut transactions = Vec::new();
        let account_idx = self.extract_account_index(solana_coin.my_address.clone());
        for instruction in self.transaction.message.instructions.iter() {
            if instruction.is_solana_transfer() {
                let lamports = instruction.parsed.info.lamports.unwrap_or_default();
                let amount = BigDecimal::try_from(lamports_to_sol(lamports))?;
                let is_self_transfer = instruction.parsed.info.source == instruction.parsed.info.destination;
                let am_i_sender = instruction.parsed.info.source == solana_coin.my_address;
                let spent_by_me = if am_i_sender && !is_self_transfer {
                    amount.clone()
                } else {
                    0.into()
                };
                let received_by_me = if is_self_transfer { amount.clone() } else { 0.into() };
                let my_balance_change = if am_i_sender {
                    BigDecimal::try_from(lamports_to_sol(
                        self.meta.pre_balances[account_idx] - self.meta.post_balances[account_idx],
                    ))?
                } else {
                    BigDecimal::try_from(lamports_to_sol(
                        self.meta.post_balances[account_idx] - self.meta.pre_balances[account_idx],
                    ))?
                };
                let fee = BigDecimal::try_from(lamports_to_sol(self.meta.fee))?;
                let tx = TransactionDetails {
                    tx_hex: Default::default(),
                    tx_hash: self.transaction.signatures[0].to_string(),
                    from: vec![instruction.parsed.info.source.clone()],
                    to: vec![instruction.parsed.info.destination.clone()],
                    total_amount: amount,
                    spent_by_me,
                    received_by_me,
                    my_balance_change,
                    block_height: self.slot,
                    timestamp: self.block_time,
                    fee_details: Some(SolanaFeeDetails { amount: fee }.into()),
                    coin: solana_coin.ticker.clone(),
                    internal_id: Default::default(),
                    kmd_rewards: None,
                    transaction_type: TransactionType::StandardTransfer,
                };
                transactions.push(tx);
            }
        }
        Ok(transactions)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Meta {
    err: Option<serde_json::Value>,
    status: Status,
    fee: u64,
    #[serde(rename = "preBalances")]
    pre_balances: Vec<u64>,
    #[serde(rename = "postBalances")]
    post_balances: Vec<u64>,
    #[serde(rename = "innerInstructions")]
    inner_instructions: Vec<Option<serde_json::Value>>,
    #[serde(rename = "logMessages")]
    log_messages: Vec<String>,
    #[serde(rename = "preTokenBalances")]
    pre_token_balances: Vec<TokenBalance>,
    #[serde(rename = "postTokenBalances")]
    post_token_balances: Vec<TokenBalance>,
    rewards: Vec<Option<serde_json::Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenBalance {
    #[serde(rename = "accountIndex")]
    account_index: u64,
    mint: String,
    #[serde(rename = "uiTokenAmount")]
    ui_token_amount: TokenAmount,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenAmount {
    #[serde(rename = "uiAmount")]
    ui_amount: f64,
    decimals: u64,
    amount: String,
    #[serde(rename = "uiAmountString")]
    ui_amount_string: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    #[serde(rename = "Ok")]
    ok: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Transaction {
    signatures: Vec<String>,
    message: Message,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "accountKeys")]
    account_keys: Vec<AccountKey>,
    #[serde(rename = "recentBlockhash")]
    recent_blockhash: String,
    instructions: Vec<Instruction>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountKey {
    pubkey: String,
    writable: bool,
    signer: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Instruction {
    program: Program,
    #[serde(rename = "programId")]
    program_id: String,
    parsed: Parsed,
}

#[allow(dead_code)]
impl Instruction {
    pub fn is_solana_transfer(&self) -> bool {
        let is_system = match self.program {
            Program::SplToken => return false,
            Program::System => true,
        };
        let is_transfer = match self.parsed.parsed_type {
            Type::Transfer => true,
            Type::TransferChecked => true,
            Type::Unknown => false,
        };
        is_system && is_transfer && self.parsed.info.lamports.is_some()
    }

    // Will be used later
    pub fn is_spl_transfer(&self) -> bool {
        let is_spl_token = match self.program {
            Program::SplToken => true,
            Program::System => return false,
        };
        let is_transfer = match self.parsed.parsed_type {
            Type::Transfer => true,
            Type::TransferChecked => true,
            Type::Unknown => false,
        };
        is_spl_token && is_transfer
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Parsed {
    info: Info,
    #[serde(rename = "type")]
    parsed_type: Type,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Info {
    destination: String,
    lamports: Option<u64>,
    source: String,
    mint: Option<String>,
    #[serde(rename = "multisigAuthority")]
    multisig_authority: Option<String>,
    signers: Option<Vec<String>>,
    #[serde(rename = "tokenAmount")]
    token_amount: Option<TokenAmount>,
    authority: Option<String>,
    amount: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Type {
    #[serde(rename = "transfer")]
    Transfer,
    #[serde(rename = "transferChecked")]
    TransferChecked,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Program {
    #[serde(rename = "spl-token")]
    SplToken,
    #[serde(rename = "system")]
    System,
}
