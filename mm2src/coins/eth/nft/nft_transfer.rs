use mm2_number::BigDecimal;

pub struct WithdrawErc721Request {
    coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
}

pub struct WithdrawErc1155Request {
    coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
}

pub struct TransactionNFTDetails {
    contract_type: String,
    // todo
}
