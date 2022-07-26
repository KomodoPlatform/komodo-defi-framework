use super::bytes::Bytes;
use super::hash::H256;
use super::script::ScriptType;
use keys::Address;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use v1::types;

/// Hex-encoded transaction
pub type RawTransaction = Bytes;

/// Transaction input
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TransactionInput {
    /// Previous transaction id
    pub txid: H256,
    /// Previous transaction output index
    pub vout: u32,
    /// Sequence number
    pub sequence: Option<u32>,
}

/// Transaction output of form "address": amount
#[derive(Debug, PartialEq)]
pub struct TransactionOutputWithAddress {
    /// Receiver' address
    pub address: Address,
    /// Amount in BTC
    pub amount: f64,
}

/// Transaction output of form "data": serialized(output script data)
#[derive(Debug, PartialEq)]
pub struct TransactionOutputWithScriptData {
    /// Serialized script data
    pub script_data: Bytes,
}

/// Transaction output
#[derive(Debug, PartialEq)]
pub enum TransactionOutput {
    /// Of form address: amount
    Address(TransactionOutputWithAddress),
    /// Of form data: script_data_bytes
    ScriptData(TransactionOutputWithScriptData),
}

/// Transaction outputs, which serializes/deserializes as KV-map
#[derive(Debug, PartialEq)]
pub struct TransactionOutputs {
    /// Transaction outputs
    pub outputs: Vec<TransactionOutput>,
}

/// Transaction input script
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TransactionInputScript {
    /// Script code
    pub asm: String,
    /// Script hex
    pub hex: Bytes,
}

/// Transaction output script
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TransactionOutputScript {
    /// Script code
    pub asm: String,
    /// Script hex
    pub hex: Bytes,
    /// Number of required signatures
    #[serde(rename = "reqSigs")]
    #[serde(default)]
    pub req_sigs: u32,
    /// Type of script
    #[serde(rename = "type")]
    pub script_type: ScriptType,
    /// Array of bitcoin addresses
    #[serde(default)]
    pub addresses: Vec<String>,
}

impl TransactionOutputScript {
    pub fn is_empty(&self) -> bool { self.asm.is_empty() && self.hex.is_empty() }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TransactionInputEnum {
    Signed(SignedTransactionInput),
    Coinbase(CoinbaseTransactionInput),
    /// FIRO specific
    Sigma(SigmaInput),
    /// FIRO specific
    Lelantus(LelantusInput),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SigmaInput {
    #[serde(rename = "anonymityGroup")]
    anonymity_group: i64,
    #[serde(rename = "scriptSig")]
    pub script_sig: TransactionInputScript,
    value: f64,
    #[serde(rename = "valueSat")]
    value_sat: u64,
    sequence: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LelantusInput {
    #[serde(rename = "scriptSig")]
    pub script_sig: TransactionInputScript,
    #[serde(rename = "nFees")]
    pub n_fees: f64,
    serials: Vec<String>,
    sequence: u32,
}

impl TransactionInputEnum {
    pub fn is_coinbase(&self) -> bool { matches!(self, TransactionInputEnum::Coinbase(_)) }
}

/// Signed transaction input
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SignedTransactionInput {
    /// Previous transaction id
    pub txid: H256,
    /// Previous transaction output index
    pub vout: u32,
    /// Input script
    #[serde(rename = "scriptSig")]
    pub script_sig: TransactionInputScript,
    /// Sequence number
    pub sequence: u32,
    /// Hex-encoded witness data (if any)
    pub txinwitness: Option<Vec<String>>,
}

/// Coinbase transaction input
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CoinbaseTransactionInput {
    /// coinbase
    pub coinbase: Bytes,
    /// Sequence number
    pub sequence: u32,
}

/// Signed transaction output
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SignedTransactionOutput {
    /// Output value in BTC
    pub value: Option<f64>,
    /// Output index
    pub n: u32,
    /// Output script
    #[serde(rename = "scriptPubKey")]
    pub script: TransactionOutputScript,
}

impl SignedTransactionOutput {
    pub fn is_empty(&self) -> bool { self.value == Some(0.0) && self.script.is_empty() }
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// Transaction
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Transaction {
    /// Raw transaction
    pub hex: RawTransaction,
    /// The transaction id (same as provided)
    pub txid: H256,
    /// The transaction hash (differs from txid for witness transactions)
    pub hash: Option<H256>,
    /// The serialized transaction size
    pub size: Option<usize>,
    /// The virtual transaction size (differs from size for witness transactions)
    pub vsize: Option<usize>,
    /// The version
    pub version: i32,
    /// The lock time
    pub locktime: u32,
    /// Transaction inputs
    pub vin: Vec<TransactionInputEnum>,
    /// Transaction outputs
    pub vout: Vec<SignedTransactionOutput>,
    /// Hash of the block this transaction is included in
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub blockhash: H256,
    /// Number of confirmations of this transaction
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub confirmations: u32,
    /// Number of rawconfirmations of this transaction, KMD specific
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rawconfirmations: Option<u32>,
    /// The transaction time in seconds since epoch (Jan 1 1970 GMT)
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub time: u32,
    /// The block time in seconds since epoch (Jan 1 1970 GMT)
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_null_default")]
    pub blocktime: u32,
    /// The block height transaction mined in
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
}

impl Transaction {
    pub fn is_coinbase(&self) -> bool { self.vin.iter().any(|input| input.is_coinbase()) }
}

/// Return value of `getrawtransaction` method
#[derive(Debug, PartialEq)]
pub enum GetRawTransactionResponse {
    /// Return value when asking for raw transaction
    Raw(RawTransaction),
    /// Return value when asking for verbose transaction
    Verbose(Box<Transaction>),
}

impl Serialize for GetRawTransactionResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            GetRawTransactionResponse::Raw(ref raw_transaction) => raw_transaction.serialize(serializer),
            GetRawTransactionResponse::Verbose(ref verbose_transaction) => verbose_transaction.serialize(serializer),
        }
    }
}

impl TransactionOutputs {
    pub fn len(&self) -> usize { self.outputs.len() }

    pub fn is_empty(&self) -> bool { self.outputs.is_empty() }
}

impl Serialize for TransactionOutputs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_map(Some(self.len()))?;
        for output in &self.outputs {
            match *output {
                TransactionOutput::Address(ref address_output) => {
                    state.serialize_entry(&address_output.address.to_string(), &address_output.amount)?;
                },
                TransactionOutput::ScriptData(ref script_output) => {
                    state.serialize_entry("data", &script_output.script_data)?;
                },
            }
        }
        state.end()
    }
}

impl<'a> Deserialize<'a> for TransactionOutputs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
    {
        use serde::de::{MapAccess, Visitor};

        struct TransactionOutputsVisitor;

        impl<'b> Visitor<'b> for TransactionOutputsVisitor {
            type Value = TransactionOutputs;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a transaction output object")
            }

            fn visit_map<V>(self, mut visitor: V) -> Result<TransactionOutputs, V::Error>
            where
                V: MapAccess<'b>,
            {
                let mut outputs: Vec<TransactionOutput> = Vec::with_capacity(visitor.size_hint().unwrap_or(0));

                while let Some(key) = visitor.next_key::<String>()? {
                    if &key == "data" {
                        let value: Bytes = visitor.next_value()?;
                        outputs.push(TransactionOutput::ScriptData(TransactionOutputWithScriptData {
                            script_data: value,
                        }));
                    } else {
                        let address = types::address::AddressVisitor::default().visit_str(&key)?;
                        let amount: f64 = visitor.next_value()?;
                        outputs.push(TransactionOutput::Address(TransactionOutputWithAddress {
                            address,
                            amount,
                        }));
                    }
                }

                Ok(TransactionOutputs { outputs })
            }
        }

        deserializer.deserialize_identifier(TransactionOutputsVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::super::bytes::Bytes;
    use super::super::hash::H256;
    use super::super::script::ScriptType;
    use super::*;
    use serde_json;

    #[test]
    fn transaction_input_serialize() {
        let txinput = TransactionInput {
            txid: H256::from(7),
            vout: 33,
            sequence: Some(88),
        };
        assert_eq!(
            serde_json::to_string(&txinput).unwrap(),
            r#"{"txid":"0700000000000000000000000000000000000000000000000000000000000000","vout":33,"sequence":88}"#
        );
    }

    #[test]
    fn transaction_input_deserialize() {
        let txinput = TransactionInput {
            txid: H256::from(7),
            vout: 33,
            sequence: Some(88),
        };

        assert_eq!(
            serde_json::from_str::<TransactionInput>(
                r#"{"txid":"0700000000000000000000000000000000000000000000000000000000000000","vout":33,"sequence":88}"#
            )
            .unwrap(),
            txinput
        );
    }

    #[test]
    fn transaction_outputs_serialize() {
        let txout = TransactionOutputs {
            outputs: vec![
                TransactionOutput::Address(TransactionOutputWithAddress {
                    address: "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
                    amount: 123.45,
                }),
                TransactionOutput::Address(TransactionOutputWithAddress {
                    address: "1H5m1XzvHsjWX3wwU781ubctznEpNACrNC".into(),
                    amount: 67.89,
                }),
                TransactionOutput::ScriptData(TransactionOutputWithScriptData {
                    script_data: Bytes::new(vec![1, 2, 3, 4]),
                }),
                TransactionOutput::ScriptData(TransactionOutputWithScriptData {
                    script_data: Bytes::new(vec![5, 6, 7, 8]),
                }),
            ],
        };
        assert_eq!(
            serde_json::to_string(&txout).unwrap(),
            r#"{"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa":123.45,"1H5m1XzvHsjWX3wwU781ubctznEpNACrNC":67.89,"data":"01020304","data":"05060708"}"#
        );
    }

    #[ignore]
    #[test]
    fn transaction_outputs_deserialize() {
        let txout = TransactionOutputs {
            outputs: vec![
                TransactionOutput::Address(TransactionOutputWithAddress {
                    address: "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
                    amount: 123.45,
                }),
                TransactionOutput::Address(TransactionOutputWithAddress {
                    address: "1H5m1XzvHsjWX3wwU781ubctznEpNACrNC".into(),
                    amount: 67.89,
                }),
                TransactionOutput::ScriptData(TransactionOutputWithScriptData {
                    script_data: Bytes::new(vec![1, 2, 3, 4]),
                }),
                TransactionOutput::ScriptData(TransactionOutputWithScriptData {
                    script_data: Bytes::new(vec![5, 6, 7, 8]),
                }),
            ],
        };
        assert_eq!(
			serde_json::from_str::<TransactionOutputs>(r#"{"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa":123.45,"1H5m1XzvHsjWX3wwU781ubctznEpNACrNC":67.89,"data":"01020304","data":"05060708"}"#).unwrap(),
			txout);
    }

    #[test]
    fn transaction_input_script_serialize() {
        let txin = TransactionInputScript {
            asm: "Hello, world!!!".to_owned(),
            hex: Bytes::new(vec![1, 2, 3, 4]),
        };
        assert_eq!(
            serde_json::to_string(&txin).unwrap(),
            r#"{"asm":"Hello, world!!!","hex":"01020304"}"#
        );
    }

    #[test]
    fn transaction_input_script_deserialize() {
        let txin = TransactionInputScript {
            asm: "Hello, world!!!".to_owned(),
            hex: Bytes::new(vec![1, 2, 3, 4]),
        };
        assert_eq!(
            serde_json::from_str::<TransactionInputScript>(r#"{"asm":"Hello, world!!!","hex":"01020304"}"#).unwrap(),
            txin
        );
    }

    #[test]
    fn transaction_output_script_serialize() {
        let txout = TransactionOutputScript {
            asm: "Hello, world!!!".to_owned(),
            hex: Bytes::new(vec![1, 2, 3, 4]),
            req_sigs: 777,
            script_type: ScriptType::Multisig,
            addresses: vec![
                "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
                "1H5m1XzvHsjWX3wwU781ubctznEpNACrNC".into(),
            ],
        };
        assert_eq!(
            serde_json::to_string(&txout).unwrap(),
            r#"{"asm":"Hello, world!!!","hex":"01020304","reqSigs":777,"type":"multisig","addresses":["1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa","1H5m1XzvHsjWX3wwU781ubctznEpNACrNC"]}"#
        );
    }

    #[test]
    fn transaction_output_script_deserialize() {
        let txout = TransactionOutputScript {
            asm: "Hello, world!!!".to_owned(),
            hex: Bytes::new(vec![1, 2, 3, 4]),
            req_sigs: 777,
            script_type: ScriptType::Multisig,
            addresses: vec![
                "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
                "1H5m1XzvHsjWX3wwU781ubctznEpNACrNC".into(),
            ],
        };

        assert_eq!(
			serde_json::from_str::<TransactionOutputScript>(r#"{"asm":"Hello, world!!!","hex":"01020304","reqSigs":777,"type":"multisig","addresses":["1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa","1H5m1XzvHsjWX3wwU781ubctznEpNACrNC"]}"#).unwrap(),
			txout);
    }

    #[test]
    fn signed_transaction_input_serialize() {
        let txin = SignedTransactionInput {
            txid: H256::from(77),
            vout: 13,
            script_sig: TransactionInputScript {
                asm: "Hello, world!!!".to_owned(),
                hex: Bytes::new(vec![1, 2, 3, 4]),
            },
            sequence: 123,
            txinwitness: None,
        };
        assert_eq!(
            serde_json::to_string(&txin).unwrap(),
            r#"{"txid":"4d00000000000000000000000000000000000000000000000000000000000000","vout":13,"scriptSig":{"asm":"Hello, world!!!","hex":"01020304"},"sequence":123,"txinwitness":null}"#
        );
    }

    #[test]
    fn signed_transaction_input_deserialize() {
        let txin = SignedTransactionInput {
            txid: H256::from(77),
            vout: 13,
            script_sig: TransactionInputScript {
                asm: "Hello, world!!!".to_owned(),
                hex: Bytes::new(vec![1, 2, 3, 4]),
            },
            sequence: 123,
            txinwitness: Some(vec![]),
        };
        assert_eq!(
			serde_json::from_str::<SignedTransactionInput>(r#"{"txid":"4d00000000000000000000000000000000000000000000000000000000000000","vout":13,"scriptSig":{"asm":"Hello, world!!!","hex":"01020304"},"sequence":123,"txinwitness":[]}"#).unwrap(),
			txin);
    }

    #[test]
    fn signed_transaction_output_serialize() {
        let txout = SignedTransactionOutput {
            value: Some(777.79),
            n: 12,
            script: TransactionOutputScript {
                asm: "Hello, world!!!".to_owned(),
                hex: Bytes::new(vec![1, 2, 3, 4]),
                req_sigs: 777,
                script_type: ScriptType::Multisig,
                addresses: vec![
                    "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
                    "1H5m1XzvHsjWX3wwU781ubctznEpNACrNC".into(),
                ],
            },
        };
        assert_eq!(
            serde_json::to_string(&txout).unwrap(),
            r#"{"value":777.79,"n":12,"scriptPubKey":{"asm":"Hello, world!!!","hex":"01020304","reqSigs":777,"type":"multisig","addresses":["1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa","1H5m1XzvHsjWX3wwU781ubctznEpNACrNC"]}}"#
        );
    }

    #[test]
    fn signed_transaction_output_deserialize() {
        let txout = SignedTransactionOutput {
            value: Some(777.79),
            n: 12,
            script: TransactionOutputScript {
                asm: "Hello, world!!!".to_owned(),
                hex: Bytes::new(vec![1, 2, 3, 4]),
                req_sigs: 777,
                script_type: ScriptType::Multisig,
                addresses: vec![
                    "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
                    "1H5m1XzvHsjWX3wwU781ubctznEpNACrNC".into(),
                ],
            },
        };
        assert_eq!(
			serde_json::from_str::<SignedTransactionOutput>(r#"{"value":777.79,"n":12,"scriptPubKey":{"asm":"Hello, world!!!","hex":"01020304","reqSigs":777,"type":"multisig","addresses":["1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa","1H5m1XzvHsjWX3wwU781ubctznEpNACrNC"]}}"#).unwrap(),
			txout);
    }

    #[test]
    fn transaction_serialize() {
        let tx = Transaction {
            hex: "DEADBEEF".into(),
            txid: H256::from(4),
            hash: Some(H256::from(5)),
            size: Some(33),
            vsize: Some(44),
            version: 55,
            locktime: 66,
            vin: vec![],
            vout: vec![],
            blockhash: H256::from(6),
            confirmations: 77,
            rawconfirmations: None,
            time: 88,
            blocktime: 99,
            height: Some(0),
        };
        assert_eq!(
            serde_json::to_string(&tx).unwrap(),
            r#"{"hex":"deadbeef","txid":"0400000000000000000000000000000000000000000000000000000000000000","hash":"0500000000000000000000000000000000000000000000000000000000000000","size":33,"vsize":44,"version":55,"locktime":66,"vin":[],"vout":[],"blockhash":"0600000000000000000000000000000000000000000000000000000000000000","confirmations":77,"time":88,"blocktime":99,"height":0}"#
        );
    }

    #[test]
    fn transaction_deserialize() {
        let tx = Transaction {
            hex: "DEADBEEF".into(),
            txid: H256::from(4),
            hash: Some(H256::from(5)),
            size: Some(33),
            vsize: Some(44),
            version: 55,
            locktime: 66,
            vin: vec![],
            vout: vec![],
            blockhash: H256::from(6),
            confirmations: 77,
            rawconfirmations: None,
            time: 88,
            blocktime: 99,
            height: None,
        };
        assert_eq!(
			serde_json::from_str::<Transaction>(r#"{"hex":"deadbeef","txid":"0400000000000000000000000000000000000000000000000000000000000000","hash":"0500000000000000000000000000000000000000000000000000000000000000","size":33,"vsize":44,"version":55,"locktime":66,"vin":[],"vout":[],"blockhash":"0600000000000000000000000000000000000000000000000000000000000000","confirmations":77,"time":88,"blocktime":99}"#).unwrap(),
			tx);
    }

    #[test]
    // https://kmdexplorer.io/tx/88893f05764f5a781f2e555a5b492c064f2269a4a44c51afdbe98fab54361bb5
    fn test_kmd_json_transaction_parse_fail() {
        let tx_str = r#"{
			"hex":"0100000001ebca38fa14b1ec029c3e08a2e87940c1f796b1588674b4c386f09626ee702576010000006a4730440220070963b9460d9bafe7865563574594fc3f823e5cdf7c49a5642dade76502547f022023fd90d41e34e514237f4b5967f83c9af27673d6de2eae3d88079a988fa5be3e012103668e3368c9fb67d8fc808a5fe74d5a8d21b6eed726838122d5f7716fb3328998ffffffff03e87006060000000017a914fef59ae800bb89050d25f67be432b231097e1849878758c100000000001976a91473122bcec852f394e51496e39fca5111c3d7ae5688ac00000000000000000a6a08303764643135633400000000",
			"txid":"88893f05764f5a781f2e555a5b492c064f2269a4a44c51afdbe98fab54361bb5",
			"overwintered":false,
			"version":1,
			"last_notarized_height":1415230,
			"locktime":0,
			"vin":[
				{
					"txid":"762570ee2696f086c3b4748658b196f7c14079e8a2083e9c02ecb114fa38caeb",
					"vout":1,
					"address":"RKmdZ8QA7XbJ4JGUAvtHtWEogKxfgaQuqv",
					"scriptSig":{
					"asm":"30440220070963b9460d9bafe7865563574594fc3f823e5cdf7c49a5642dade76502547f022023fd90d41e34e514237f4b5967f83c9af27673d6de2eae3d88079a988fa5be3e[ALL] 03668e3368c9fb67d8fc808a5fe74d5a8d21b6eed726838122d5f7716fb3328998",
					"hex":"4730440220070963b9460d9bafe7865563574594fc3f823e5cdf7c49a5642dade76502547f022023fd90d41e34e514237f4b5967f83c9af27673d6de2eae3d88079a988fa5be3e012103668e3368c9fb67d8fc808a5fe74d5a8d21b6eed726838122d5f7716fb3328998"
				},
					"value":1.13766527,
					"valueSat":113766527,
					"sequence":4294967295
				}
			],
			"vout":[
				{
					"value":1.01085416,
					"valueSat":101085416,
					"n":0,
					"scriptPubKey":{
					"asm":"OP_HASH160 fef59ae800bb89050d25f67be432b231097e1849 OP_EQUAL",
					"hex":"a914fef59ae800bb89050d25f67be432b231097e184987",
					"reqSigs":1,
					"type":"scripthash",
					"addresses":[
						"bbyNYu11Qs3PowiPr1Su4ozQk7hsVmv821"
					]
				}
				},
				{
					"value":0.12671111,
					"valueSat":12671111,
					"n":1,
					"scriptPubKey":{
					"asm":"OP_DUP OP_HASH160 73122bcec852f394e51496e39fca5111c3d7ae56 OP_EQUALVERIFY OP_CHECKSIG",
					"hex":"76a91473122bcec852f394e51496e39fca5111c3d7ae5688ac",
					"reqSigs":1,
					"type":"pubkeyhash",
					"addresses":[
						"RKmdZ8QA7XbJ4JGUAvtHtWEogKxfgaQuqv"
					]
				}
				},
				{
					"value":0.0,
					"valueSat":0,
					"n":2,
					"scriptPubKey":{
					"asm":"OP_RETURN 3037646431356334",
					"hex":"6a083037646431356334",
					"type":"nulldata"
				}
				}
			],
			"vjoinsplit":[

			],
			"blockhash":"086c0807a67d8411743f7eaf0a687721eadaa6c8190dfd36f4de9d939c796e82",
			"height":865648,
			"confirmations":549608,
			"rawconfirmations":549608,
			"time":1528215344,
			"blocktime":1528215344
		}"#;

        let _tx: Transaction = serde_json::from_str(tx_str).unwrap();
    }

    #[test]
    fn test_kmd_coinbase_transaction_parse() {
        let tx_str = r#"{
			"hex": "0400008085202f89010000000000000000000000000000000000000000000000000000000000000000ffffffff06030a4b020101ffffffff0178e600000000000023210388392e0885e449ea9745ce7ad2631fdca5288f9d790cee1b696e67c75ad54a2dac1ad92f5d000000000000000000000000000000",
			"txid": "6f173d96987e765b0fd8a47fdb976e8edc767207f3c0028e17a224380d9a14a3",
			"overwintered": true,
			"version": 4,
			"versiongroupid": "892f2085",
			"locktime": 1563416858,
			"expiryheight": 0,
			"vin": [
				{
				  "coinbase": "030a4b020101",
				  "sequence": 4294967295
				}
			],
			"vout": [
				{
				  "value": 0.00059000,
				  "valueSat": 59000,
				  "n": 0,
				  "scriptPubKey": {
					"asm": "0388392e0885e449ea9745ce7ad2631fdca5288f9d790cee1b696e67c75ad54a2d OP_CHECKSIG",
					"hex": "210388392e0885e449ea9745ce7ad2631fdca5288f9d790cee1b696e67c75ad54a2dac",
					"reqSigs": 1,
					"type": "pubkey",
					"addresses": [
					  "RM5wffThEVKQdG98uLa2gc8Nk4CzX9Fq4q"
					]
				  }
				}
			],
			"vjoinsplit": [
			],
			"valueBalance": 0.00000000,
			"vShieldedSpend": [
			],
			"vShieldedOutput": [
			],
			"blockhash": "04b08f77065a70c86fd47e92cbff2cd73b1768428da7c8e328d903d76e8dc37e",
			"height": 150282,
			"confirmations": 1,
			"rawconfirmations": 6,
			"time": 1563416858,
			"blocktime": 1563416858
		}"#;

        let _tx: Transaction = serde_json::from_str(tx_str).unwrap();
    }

    // https://live.blockcypher.com/btc/tx/4ab5828480046524afa3fac5eb7f93f768c3eeeaeb5d4d6b6ff22801d3dc521e/
    #[test]
    fn test_btc_4ab5828480046524afa3fac5eb7f93f768c3eeeaeb5d4d6b6ff22801d3dc521e() {
        let tx_str = r#"{
			"txid":"4ab5828480046524afa3fac5eb7f93f768c3eeeaeb5d4d6b6ff22801d3dc521e",
			"hash":"89f9ae508f67ce79181f43cd4823e9899ef3116d658457c992b8411674f80c5c",
			"version":2,
			"size":3316,
			"vsize":3231,
			"weight":12922,
			"locktime":582070,
			"vin":[
				{
					"txid":"bc1cac1354e18195bbcb56e9b6212bc7ceb481ea46d18ed39493fbe028af370e",
					"vout":0,
					"scriptSig":{
						"asm":"3045022100a8fdfac02ecba2cfa25d74f76dcfba41791563d9aac29063dab7f9865009212002200a79c035e48f675c0527f33926ebdbb8dbae89c0a77f1e7ba229126b9fa97cc6[ALL] 02679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67",
						"hex":"483045022100a8fdfac02ecba2cfa25d74f76dcfba41791563d9aac29063dab7f9865009212002200a79c035e48f675c0527f33926ebdbb8dbae89c0a77f1e7ba229126b9fa97cc6012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67"
					},
					"sequence":4294967294
				},
				{
					"txid":"40bbaf2e6f209fd798c5d4dbbb53059b1b3fbe74d1bdd4defda3041a67d72122",
					"vout":0,
					"scriptSig":{
						"asm":"3045022100913d8dd7fc3e2114bec634886b0189cc400cba036c168228b9423f5526a9d361022008b3b02d3c0270911def718c1859aba34233e2e3c7327e2f5ac7d1a7fd65b9eb[ALL] 02679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67",
						"hex":"483045022100913d8dd7fc3e2114bec634886b0189cc400cba036c168228b9423f5526a9d361022008b3b02d3c0270911def718c1859aba34233e2e3c7327e2f5ac7d1a7fd65b9eb012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67"
					},
					"sequence":4294967294
				},
				{
					"txid":"f5abe9270190bb39a1b45ff8229913c7edc684e896df86033d2d0994e67fcb6b",
					"vout":0,
					"scriptSig":{
						"asm":"30440220103bac3e985912b388f48cc979f82821cb637f690fdd497efe4fceb86e00122f022026173b0e6a5e5eef7483b94f7589e78810eae8f8249ff7b03876f6ae24faa19b[ALL] 02679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67",
						"hex":"4730440220103bac3e985912b388f48cc979f82821cb637f690fdd497efe4fceb86e00122f022026173b0e6a5e5eef7483b94f7589e78810eae8f8249ff7b03876f6ae24faa19b012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67"
					},
					"sequence":4294967294
				},
				{
					"txid":"7cdd760d5d4ce952c9d25128a1f475b1a058cd71506cb7af956f2ab933b4d8a8",
					"vout":1,
					"scriptSig":{
						"asm":"30450221009f9188ef194366c3bb4cd520eb9d8a68c3f2fb6ea591f671a00039f05f67b9420220579874562e721bf8d07a34adc1ac587b6b48609100c43ba8e8bf180c86763adf[ALL] 02679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67",
						"hex":"4830450221009f9188ef194366c3bb4cd520eb9d8a68c3f2fb6ea591f671a00039f05f67b9420220579874562e721bf8d07a34adc1ac587b6b48609100c43ba8e8bf180c86763adf012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67"
					},
					"sequence":4294967294
				},
				{
					"txid":"73a7faa14c4e654f327d6be4f685f91234a6682b97bf0f5384e90bff861786ce",
					"vout":45,
					"scriptSig":{
						"asm":"",
						"hex":""
					},
					"txinwitness":[
						"3045022100b7b6368e45383b2da463ba56397a1966b94be5ef860ac95f1067e62a4531e75a022077bc58f3ea606219fe086f291d39b805faec10c848b525f4997f32979bab5aca01",
						"0253a13bae39c5604dc4e9634c10e87e33d0c2d1a618efc0726af5a4a4ea81f7ab"
					],
					"sequence":4294967294
				}
			],
			"vout":[
				{
					"value":0.0095,
					"n":0,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 66f8da41c6bb10975f565bde68b5df07003c59cb OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a91466f8da41c6bb10975f565bde68b5df07003c59cb88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1APU39UZbmpV3RB2EXQmKikKEgovLVoXzv"
						]
					}
				},
				{
					"value":0.56054866,
					"n":1,
					"scriptPubKey":{
						"asm":"OP_HASH160 46e14b4a4ff41785017080cd63aa5d17513e1854 OP_EQUAL",
						"hex":"a91446e14b4a4ff41785017080cd63aa5d17513e185487",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"389o7gRfw13GnRYg4yuhsATJ1iiJ8QFiBv"
						]
					}
				},
				{
					"value":1.9995,
					"n":2,
					"scriptPubKey":{
						"asm":"OP_HASH160 99bbebbdf7f2dc038b904103237765a77282b42b OP_EQUAL",
						"hex":"a91499bbebbdf7f2dc038b904103237765a77282b42b87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3FhtRs6hwos3uS62XhfkoP9PwnGFb9u9AT"
						]
					}
				},
				{
					"value":0.04788773,
					"n":3,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 c78ac0df6b8241075d66f7f986653604a2c6a6fc OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914c78ac0df6b8241075d66f7f986653604a2c6a6fc88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1KC5gvwy5SSarNgL7pVcEdrB7Gj1upeni9"
						]
					}
				},
				{
					"value":0.137,
					"n":4,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 79f1db0274de574d49f9fc794b349ef81529fb18 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a91479f1db0274de574d49f9fc794b349ef81529fb1888ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1C7nVdM4vYjSE23SnyEEbdEhzp3LXAwssr"
						]
					}
				},
				{
					"value":0.55,
					"n":5,
					"scriptPubKey":{
						"asm":"OP_HASH160 840f4d27071f400c5674b1a686235cb641ef34b8 OP_EQUAL",
						"hex":"a914840f4d27071f400c5674b1a686235cb641ef34b887",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3DjHT2Ks4bmJUQh8exeQYcGpXuHe4deVy8"
						]
					}
				},
				{
					"value":2.87351761,
					"n":6,
					"scriptPubKey":{
						"asm":"OP_HASH160 c2f1c77b4ab921d9a2b7a36b250e4ac5a29afe92 OP_EQUAL",
						"hex":"a914c2f1c77b4ab921d9a2b7a36b250e4ac5a29afe9287",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3KTnese7izLxXTSBe86fY7Cg2tHyApdsV5"
						]
					}
				},
				{
					"value":0.00343558,
					"n":7,
					"scriptPubKey":{
						"asm":"OP_HASH160 d58ee5f1a2bc153ce58145676a679d7b31a1a5ae OP_EQUAL",
						"hex":"a914d58ee5f1a2bc153ce58145676a679d7b31a1a5ae87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3MAD39R5NPANmrbgybN93jMsiTWd9sgB7A"
						]
					}
				},
				{
					"value":0.0345,
					"n":8,
					"scriptPubKey":{
						"asm":"OP_HASH160 a61b218139c3cd63abbfc6d221f28019d86837d6 OP_EQUAL",
						"hex":"a914a61b218139c3cd63abbfc6d221f28019d86837d687",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3GqJbUSzTZomr9Jjz9Rj4Kb3idbaA7FwvA"
						]
					}
				},
				{
					"value":0.04904543,
					"n":9,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 8aeadc4ab5fbdf6fba1396405388868395cf4f1b OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9148aeadc4ab5fbdf6fba1396405388868395cf4f1b88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1DfXbsJTxDuhtbBNz5njZdSZcGGBcerYPr"
						]
					}
				},
				{
					"value":0.0355541,
					"n":10,
					"scriptPubKey":{
						"asm":"OP_HASH160 f9e4dab5529cda97fe7d0ea9c6dfd828c9160c82 OP_EQUAL",
						"hex":"a914f9e4dab5529cda97fe7d0ea9c6dfd828c9160c8287",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3QULLPQRcFmQNCUyR66wSgJm8YAFBAq7Yg"
						]
					}
				},
				{
					"value":0.0085,
					"n":11,
					"scriptPubKey":{
						"asm":"OP_HASH160 551343b34a385e392562ead50b2588ee97307c37 OP_EQUAL",
						"hex":"a914551343b34a385e392562ead50b2588ee97307c3787",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"39SrSNFBzzMqZw1mAaLwotixK4oRQymBw2"
						]
					}
				},
				{
					"value":0.00991826,
					"n":12,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 633a3cd7a6ce04165619539a87ee5671d0537e4e OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914633a3cd7a6ce04165619539a87ee5671d0537e4e88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1A3ffR5ag9iJM8jrkYdF4ohx9E87RkLBGt"
						]
					}
				},
				{
					"value":0.02,
					"n":13,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 9f59e0163f592c3de094bc12ae338d8140c77c54 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9149f59e0163f592c3de094bc12ae338d8140c77c5488ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1FXa79ea27eR92vxSiVGnwtjRNjSMLkHzx"
						]
					}
				},
				{
					"value":0.2495,
					"n":14,
					"scriptPubKey":{
						"asm":"OP_HASH160 7b7f9a5fa10a45fc828d6a47ee6dbbbb2364cee2 OP_EQUAL",
						"hex":"a9147b7f9a5fa10a45fc828d6a47ee6dbbbb2364cee287",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3Cx1u9nW5Q585bBqkVz1ETjogZZB67d1KZ"
						]
					}
				},
				{
					"value":0.47461784,
					"n":15,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 221a6189701ce0874c4ba6fc0f91579f68f05895 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914221a6189701ce0874c4ba6fc0f91579f68f0589588ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"147KaWNp6T7BRBhWkwKaYobPXX7ydyuo3S"
						]
					}
				},
				{
					"value":0.029,
					"n":16,
					"scriptPubKey":{
						"asm":"OP_HASH160 1e5f0577643f2c17ecd5037034824e6b55f2f37f OP_EQUAL",
						"hex":"a9141e5f0577643f2c17ecd5037034824e6b55f2f37f87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"34Tc2Vqb4xC2UanZaDUBRWvGXrETVrSmiY"
						]
					}
				},
				{
					"value":0.0495,
					"n":17,
					"scriptPubKey":{
						"asm":"OP_HASH160 a70f43b2b0bded27e58ba7997e15936d86b5b4cd OP_EQUAL",
						"hex":"a914a70f43b2b0bded27e58ba7997e15936d86b5b4cd87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3GvM4AupUGYA5asAZWcR88qBbRepyd8VE4"
						]
					}
				},
				{
					"value":0.018,
					"n":18,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 2388eb0f84b2ec9d0e35ceda9019e389aee2243f OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9142388eb0f84b2ec9d0e35ceda9019e389aee2243f88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"14EtfxYvGBbCMDZGCW2sp7NB2qDwFZBB8L"
						]
					}
				},
				{
					"value":0.26611821,
					"n":19,
					"scriptPubKey":{
						"asm":"OP_HASH160 e0a8d9fe6832f56524ad51e40c6b34cc212dad4c OP_EQUAL",
						"hex":"a914e0a8d9fe6832f56524ad51e40c6b34cc212dad4c87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3NAuZXN4HCHnAubDpcXHJBQXJsShuER2Rs"
						]
					}
				},
				{
					"value":0.0295,
					"n":20,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 97dfc57e73ab8a3b9bda027b79a28bc2e9fc1931 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a91497dfc57e73ab8a3b9bda027b79a28bc2e9fc193188ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1Er36t3XCRycpJYZ4J4FH5jQFSu8K9VVyU"
						]
					}
				},
				{
					"value":0.0314192,
					"n":21,
					"scriptPubKey":{
						"asm":"OP_HASH160 be3d917f8b403b3e6b1cf900e29d686bddc8ce64 OP_EQUAL",
						"hex":"a914be3d917f8b403b3e6b1cf900e29d686bddc8ce6487",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3K2v4qRPzHD2J2VRwwXtUf5BpFtTB8HfRj"
						]
					}
				},
				{
					"value":0.00322,
					"n":22,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 f04d6dc750f0b2d3e648ab5afcc5b1c2cedb36f7 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914f04d6dc750f0b2d3e648ab5afcc5b1c2cedb36f788ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1NubvtWw5ZfcFaKNAgg199wpuvFcnP4BoD"
						]
					}
				},
				{
					"value":1.9995,
					"n":23,
					"scriptPubKey":{
						"asm":"OP_HASH160 e366f89679d01a89599c9794a35872e5f3cb3d29 OP_EQUAL",
						"hex":"a914e366f89679d01a89599c9794a35872e5f3cb3d2987",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3NRQfeFCWG3SqbK1ATfsZTVXS6ZD33i6Rr"
						]
					}
				},
				{
					"value":0.15653823,
					"n":24,
					"scriptPubKey":{
						"asm":"OP_HASH160 af5c84f9b702a4c60611b6272c6670c4e9614741 OP_EQUAL",
						"hex":"a914af5c84f9b702a4c60611b6272c6670c4e961474187",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3HgF1UdMmdgv1kmTc8BZVBhGMXmywny2qL"
						]
					}
				},
				{
					"value":0.026613,
					"n":25,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 2a80328a0c51051bf0e76eddbf5342178128096f OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9142a80328a0c51051bf0e76eddbf5342178128096f88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"14sixNSgFLN9zEmtcTtWAX7WsAWiwfiRir"
						]
					}
				},
				{
					"value":0.00707306,
					"n":26,
					"scriptPubKey":{
						"asm":"OP_HASH160 370628b7101a7ff461de2ab0a80a8703317c7811 OP_EQUAL",
						"hex":"a914370628b7101a7ff461de2ab0a80a8703317c781187",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"36hxU2pmkkU1TnHn7wdrDYSkYpdm7bQ5Co"
						]
					}
				},
				{
					"value":0.00880634,
					"n":27,
					"scriptPubKey":{
						"asm":"OP_HASH160 166c9a23dc39fbd57e58ff794069d083933cbc4c OP_EQUAL",
						"hex":"a914166c9a23dc39fbd57e58ff794069d083933cbc4c87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"33jata4jJH1a4tnpvSGzmNwfd4yQvKigEB"
						]
					}
				},
				{
					"value":1.657691,
					"n":28,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 6f4bceafb26023db265d9abc763ab2ccbd0213ae OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9146f4bceafb26023db265d9abc763ab2ccbd0213ae88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1B9UpUvgSNEqCJZPWGCTQj9Veg21jkvaGR"
						]
					}
				},
				{
					"value":0.00593755,
					"n":29,
					"scriptPubKey":{
						"asm":"OP_HASH160 fc8d98b2a4ea22f24e50261fd065afd99a8274a0 OP_EQUAL",
						"hex":"a914fc8d98b2a4ea22f24e50261fd065afd99a8274a087",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3QiPqBRvzvVQYpVAJtuwjNY4bsLX1CF6F5"
						]
					}
				},
				{
					"value":0.00308227,
					"n":30,
					"scriptPubKey":{
						"asm":"OP_HASH160 d1803af27bed138379b501e91f368d500b0b49e7 OP_EQUAL",
						"hex":"a914d1803af27bed138379b501e91f368d500b0b49e787",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3LnkmEMq99eLSSJRH3q1UickE2nKq5QH3C"
						]
					}
				},
				{
					"value":0.01524004,
					"n":31,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 e39546887c31afee7a067432902239f44e644067 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914e39546887c31afee7a067432902239f44e64406788ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1MkMDPeYqNL4UytZCQt9QQnWYiuDwdxqom"
						]
					}
				},
				{
					"value":0.00320799,
					"n":32,
					"scriptPubKey":{
						"asm":"OP_HASH160 62830624a7d20d6c86ceeeac5a3e7bdea6773927 OP_EQUAL",
						"hex":"a91462830624a7d20d6c86ceeeac5a3e7bdea677392787",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3Afu73rZdGhcuvZXLy868Q45ZfNmuwsJqC"
						]
					}
				},
				{
					"value":0.16818798,
					"n":33,
					"scriptPubKey":{
						"asm":"OP_HASH160 66b967a217fc91d260025d46c9c9eacb746b5f9d OP_EQUAL",
						"hex":"a91466b967a217fc91d260025d46c9c9eacb746b5f9d87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3B4AxGCwpojNPr6o2VFkqjSadnrMzDseqX"
						]
					}
				},
				{
					"value":0.02369191,
					"n":34,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 0f530ba894b185be3fd809e3992145f533e99536 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9140f530ba894b185be3fd809e3992145f533e9953688ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"12Q2cz1AcyMrcttBrVTyQLaJpWjoEjyQQ7"
						]
					}
				},
				{
					"value":0.001815,
					"n":35,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 df9443d2b7b497d1e7a950379f95be6ba9ea5628 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914df9443d2b7b497d1e7a950379f95be6ba9ea562888ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1MPBJQa8fCy8ieQicaFRVj965uTdi6Ax9z"
						]
					}
				},
				{
					"value":0.0015,
					"n":36,
					"scriptPubKey":{
						"asm":"OP_HASH160 ccfccb33575cfe97d39b6d0d0fad8f09cce2fe1a OP_EQUAL",
						"hex":"a914ccfccb33575cfe97d39b6d0d0fad8f09cce2fe1a87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3LNtc6TnCNaYQE2s5tm3CXsBxzS1GvSGLD"
						]
					}
				},
				{
					"value":0.23089985,
					"n":37,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 1fec4449c7ba080cf0c85eb87ab0c855f0c3959d OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9141fec4449c7ba080cf0c85eb87ab0c855f0c3959d88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"13unz3oktsAkNXiopiCGQV1X8E4z35CAKo"
						]
					}
				},
				{
					"value":0.04740793,
					"n":38,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 e5527898cbf243993a8b5b967120cc9a9a96d092 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914e5527898cbf243993a8b5b967120cc9a9a96d09288ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1MuYY7yugviXBcFY7i9ikFxjZ6hDVCkLHC"
						]
					}
				},
				{
					"value":0.00610039,
					"n":39,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 34f2329553b026ee1aa0c02dc0743ae0cf0062a7 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a91434f2329553b026ee1aa0c02dc0743ae0cf0062a788ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"15pxHADHoN45jPxjNyxxgE3Lw8cTEDEQNF"
						]
					}
				},
				{
					"value":0.11053671,
					"n":40,
					"scriptPubKey":{
						"asm":"OP_HASH160 9f2ad2868872be8c065cc9e2e20adf31e0cc44d5 OP_EQUAL",
						"hex":"a9149f2ad2868872be8c065cc9e2e20adf31e0cc44d587",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3GCcfPrgAAqpE9vLPeNcUHdUUFMkNh2jX7"
						]
					}
				},
				{
					"value":0.03754281,
					"n":41,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 50c4073088d9ecfa0791033d17a992e8b779f127 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a91450c4073088d9ecfa0791033d17a992e8b779f12788ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"18N3tckwus8UXADPJEYzmxBMZ5m8JbG9hU"
						]
					}
				},
				{
					"value":0.06452373,
					"n":42,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 2a27eb2171827358522c29a659aaea0f50b77579 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9142a27eb2171827358522c29a659aaea0f50b7757988ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"14quCeCm2ngSgU2HqZpcdDtCQDT8rsCFMm"
						]
					}
				},
				{
					"value":0.30842538,
					"n":43,
					"scriptPubKey":{
						"asm":"OP_HASH160 b8d0465ed10eac76fc86646ace6fa64b64cf357e OP_EQUAL",
						"hex":"a914b8d0465ed10eac76fc86646ace6fa64b64cf357e87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3JYDm2sy128oyvENgtXhE1YUfugo8Ym3qd"
						]
					}
				},
				{
					"value":0.0311296,
					"n":44,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 faecf0266209f760f5d5ec498f74a0ecca351a62 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914faecf0266209f760f5d5ec498f74a0ecca351a6288ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1Psmmhrdg3FSPdq57ApmkMyVpbrjTBWctt"
						]
					}
				},
				{
					"value":0.01411207,
					"n":45,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 2e30c6bb9396a24c4cdd56f20c74f7681d812d2b OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9142e30c6bb9396a24c4cdd56f20c74f7681d812d2b88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"15DEWrw7xzinkHAarjHo33cNrfyAvj87mg"
						]
					}
				},
				{
					"value":1.2005,
					"n":46,
					"scriptPubKey":{
						"asm":"OP_HASH160 69f3751d9b18b84c15ddb3d1a5349657585c61a7 OP_EQUAL",
						"hex":"a91469f3751d9b18b84c15ddb3d1a5349657585c61a787",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3BMEXQeB9Mr5hpgYdpnJLC1MQKHQ1NfYtM"
						]
					}
				},
				{
					"value":0.04702,
					"n":47,
					"scriptPubKey":{
						"asm":"OP_HASH160 65a11389f21ba13527b1c7629e999719f3241259 OP_EQUAL",
						"hex":"a91465a11389f21ba13527b1c7629e999719f324125987",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3AxP8hReJhzjozXfaY34gDeYbGQp6LSbJh"
						]
					}
				},
				{
					"value":0.9995,
					"n":48,
					"scriptPubKey":{
						"asm":"OP_HASH160 dd005ce549e1a57453dfcb8fef3522d83f069432 OP_EQUAL",
						"hex":"a914dd005ce549e1a57453dfcb8fef3522d83f06943287",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3MqZh9ips9W5ekHbzLaRxs8xZZTbJzTLwd"
						]
					}
				},
				{
					"value":0.0023939,
					"n":49,
					"scriptPubKey":{
						"asm":"OP_HASH160 3ab72a89b9706691ac4de3871e0f63efaeed880b OP_EQUAL",
						"hex":"a9143ab72a89b9706691ac4de3871e0f63efaeed880b87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"373UYH5oSBagXbohkbVDiT45bvM9ARVJiN"
						]
					}
				},
				{
					"value":0.15724136,
					"n":50,
					"scriptPubKey":{
						"asm":"OP_HASH160 b5022f11a874eea98b9f7e34c80d143f3b036789 OP_EQUAL",
						"hex":"a914b5022f11a874eea98b9f7e34c80d143f3b03678987",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3JC6r16n98UKgR5urase5cpYExr4eJtKBn"
						]
					}
				},
				{
					"value":1.4145,
					"n":51,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 87db0ef6cde94004fabec6bb7dfb675fd691b670 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a91487db0ef6cde94004fabec6bb7dfb675fd691b67088ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1DPLeM2Xzr9aW5qxSX1TaN8MYVwm2nFtgU"
						]
					}
				},
				{
					"value":0.04914942,
					"n":52,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 b827bbd222b251930da17d0a86ba0c5e19e3b27c OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914b827bbd222b251930da17d0a86ba0c5e19e3b27c88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1Hniw3NkC35q2N9J2ZKAgQtiCcJdo1HMom"
						]
					}
				},
				{
					"value":0.00990259,
					"n":53,
					"scriptPubKey":{
						"asm":"OP_HASH160 a0c63d441be7fd967ae9ef4af028092b446a43cb OP_EQUAL",
						"hex":"a914a0c63d441be7fd967ae9ef4af028092b446a43cb87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3GM7X9RgBkBL2TvbfmAzLfzVVxaEoBfSkY"
						]
					}
				},
				{
					"value":0.0015,
					"n":54,
					"scriptPubKey":{
						"asm":"OP_HASH160 cf8df73caf54d7a8e54b1247c51b2566ae128fc1 OP_EQUAL",
						"hex":"a914cf8df73caf54d7a8e54b1247c51b2566ae128fc187",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3LcTsCyUCfNmkoZ3Jvr1cnnq27A4AjAEqj"
						]
					}
				},
				{
					"value":0.0368,
					"n":55,
					"scriptPubKey":{
						"asm":"OP_HASH160 de796fa9d384058fcaab5b37c45803af4a739931 OP_EQUAL",
						"hex":"a914de796fa9d384058fcaab5b37c45803af4a73993187",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3MyMQfUWdbrcokj7A4AKFLZqDBMq3gsbjx"
						]
					}
				},
				{
					"value":0.13156522,
					"n":56,
					"scriptPubKey":{
						"asm":"OP_HASH160 04308a751559f8af188dc67a0dac238447e91416 OP_EQUAL",
						"hex":"a91404308a751559f8af188dc67a0dac238447e9141687",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"325AmzHDJ7XaTPiuudj8iBCnJiUsiZVpwM"
						]
					}
				},
				{
					"value":0.15517116,
					"n":57,
					"scriptPubKey":{
						"asm":"OP_HASH160 28c28ea4ab911d65d2568fe2a2ade143f1804b15 OP_EQUAL",
						"hex":"a91428c28ea4ab911d65d2568fe2a2ade143f1804b1587",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"35QY2HfqzscoKuMWR9GRqyELcRvguNYvdm"
						]
					}
				},
				{
					"value":0.02739512,
					"n":58,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 6e9f5b3aefdd8b079e2d77a682a6276640b5a779 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9146e9f5b3aefdd8b079e2d77a682a6276640b5a77988ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1B5vENoZK1VAEt5xfuQLZ5cApGoyjNspWB"
						]
					}
				},
				{
					"value":0.024003,
					"n":59,
					"scriptPubKey":{
						"asm":"OP_HASH160 185f5481e1c5ab6d9926207fbfd86d85d51d7bdc OP_EQUAL",
						"hex":"a914185f5481e1c5ab6d9926207fbfd86d85d51d7bdc87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"33utLoLCbf2WwccKyvpfRYdCncQUWGLBXK"
						]
					}
				},
				{
					"value":0.0995,
					"n":60,
					"scriptPubKey":{
						"asm":"OP_HASH160 f4761ebdd81b9b7e06a207a7a3d55332d016db3e OP_EQUAL",
						"hex":"a914f4761ebdd81b9b7e06a207a7a3d55332d016db3e87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3PycJUfHHBZmGehQUGbSTmD9Pdz2s61CtY"
						]
					}
				},
				{
					"value":0.33990654,
					"n":61,
					"scriptPubKey":{
						"asm":"OP_HASH160 9841711ba7b69aa821e5e4e78b07013789c0f1cf OP_EQUAL",
						"hex":"a9149841711ba7b69aa821e5e4e78b07013789c0f1cf87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3Fa52gPVesWsG66JZGqRyjY3Pu8jPYPSYA"
						]
					}
				},
				{
					"value":0.0095,
					"n":62,
					"scriptPubKey":{
						"asm":"OP_HASH160 54d1d3982910165eddd607622cf2aa2518cf5405 OP_EQUAL",
						"hex":"a91454d1d3982910165eddd607622cf2aa2518cf540587",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"39RW3fJJXZHaikDoJHWaHSGPYeVGuCSngD"
						]
					}
				},
				{
					"value":0.3791111,
					"n":63,
					"scriptPubKey":{
						"asm":"OP_HASH160 0ac0973483473fc700352483d72211ef74b7f77a OP_EQUAL",
						"hex":"a9140ac0973483473fc700352483d72211ef74b7f77a87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"32fsPtquwdaAJXaWUhAHfmJZUmSo4iUkBT"
						]
					}
				},
				{
					"value":0.0344179,
					"n":64,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 05da50df6705f7528c0de919a87a02ca74b635fc OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a91405da50df6705f7528c0de919a87a02ca74b635fc88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1XwupEjUAd8frdAFMSUaRATgXUhQM2u1y"
						]
					}
				},
				{
					"value":0.07881161,
					"n":65,
					"scriptPubKey":{
						"asm":"OP_HASH160 69f37547d53a98c778289f01066ab23b41680905 OP_EQUAL",
						"hex":"a91469f37547d53a98c778289f01066ab23b4168090587",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3BMEXTEn8uR2utJUEAQxEG2mare7KeZKxt"
						]
					}
				},
				{
					"value":0.00247,
					"n":66,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 2889396473e1709927065dc363210386afd99407 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9142889396473e1709927065dc363210386afd9940788ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"14hLRBGW8gRCX8vUPLeBzE6T8cQ2A8zNhz"
						]
					}
				},
				{
					"value":1.0,
					"n":67,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 c625e5f34f3b2617326adbae2e73a1bb0a6be371 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914c625e5f34f3b2617326adbae2e73a1bb0a6be37188ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1K4iCCWzLgawbAJbYbDG9v81z6n6y72KNq"
						]
					}
				},
				{
					"value":2.08197368,
					"n":68,
					"scriptPubKey":{
						"asm":"OP_HASH160 2d717f7aa62e57ba6eaceca169cd7f63a54d679b OP_EQUAL",
						"hex":"a9142d717f7aa62e57ba6eaceca169cd7f63a54d679b87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"35qJJ9pwEoEwdnbfnsN1L1jdFRiyZKBEAK"
						]
					}
				},
				{
					"value":0.01,
					"n":69,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 f4f89313803d610fa472a5849d2389ca6df3b900 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914f4f89313803d610fa472a5849d2389ca6df3b90088ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1PLHf4siiNLC61LXZswQUznuMUCWcRa3e7"
						]
					}
				},
				{
					"value":0.1995,
					"n":70,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 f63784063000439d873f12041e8799d0252db89e OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914f63784063000439d873f12041e8799d0252db89e88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"1PSsjVRff6QpkJfDLyi3jVAePTSh4t4WdL"
						]
					}
				},
				{
					"value":0.03005473,
					"n":71,
					"scriptPubKey":{
						"asm":"OP_HASH160 4b098f67e04f711baa310758169f129cbda6385f OP_EQUAL",
						"hex":"a9144b098f67e04f711baa310758169f129cbda6385f87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"38Xn4Bcu6A6u9ShKDRhLoVTG8tkmC9aMSp"
						]
					}
				},
				{
					"value":0.00190118,
					"n":72,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 494ee9282fb208d60a6765c11310a09524280137 OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a914494ee9282fb208d60a6765c11310a0952428013788ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"17gcrpZVCmvdH3o3H1orsRaQ7wDAyNQ39j"
						]
					}
				},
				{
					"value":0.03079627,
					"n":73,
					"scriptPubKey":{
						"asm":"OP_HASH160 69d1a07e7d5fcb62a322c8fb24ae76ebe3c88374 OP_EQUAL",
						"hex":"a91469d1a07e7d5fcb62a322c8fb24ae76ebe3c8837487",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3BLXzpY4LN5mSXpBfkoMsAABNLRqdQ8EKM"
						]
					}
				},
				{
					"value":0.0995,
					"n":74,
					"scriptPubKey":{
						"asm":"OP_HASH160 ffe19b0c48d473db72ca2d243476183b64b4f5d4 OP_EQUAL",
						"hex":"a914ffe19b0c48d473db72ca2d243476183b64b4f5d487",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3R1zVpJKgvxKXuqL2bYm9xS8gwLWAiL9uC"
						]
					}
				},
				{
					"value":0.01924206,
					"n":75,
					"scriptPubKey":{
						"asm":"OP_HASH160 816b46471ee03653597995a2dfa65f0f39eaaf0a OP_EQUAL",
						"hex":"a914816b46471ee03653597995a2dfa65f0f39eaaf0a87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3DVKbrWQbiLMuaaWRJzsHAfMMHiC1uj5f5"
						]
					}
				},
				{
					"value":0.01392176,
					"n":76,
					"scriptPubKey":{
						"asm":"OP_HASH160 88d8d90979c558004f248cfcf0ae6efa3061100f OP_EQUAL",
						"hex":"a91488d8d90979c558004f248cfcf0ae6efa3061100f87",
						"reqSigs":1,
						"type":"scripthash",
						"addresses":[
							"3EAbbUup1nBgprztckGgKJRyB7rCiSWBPP"
						]
					}
				},
				{
					"value":0.0134,
					"n":77,
					"scriptPubKey":{
						"asm":"OP_DUP OP_HASH160 1302fb1cdc92135634e69f69feba89070a1c1b2f OP_EQUALVERIFY OP_CHECKSIG",
						"hex":"76a9141302fb1cdc92135634e69f69feba89070a1c1b2f88ac",
						"reqSigs":1,
						"type":"pubkeyhash",
						"addresses":[
							"12jXQmCHi93zKH1HQgc5fsn11beeKdMwL7"
						]
					}
				}
			],
			"hex":"020000000001050e37af28e0fb9394d38ed146ea81b4cec72b21b6e956cbbb9581e15413ac1cbc000000006b483045022100a8fdfac02ecba2cfa25d74f76dcfba41791563d9aac29063dab7f9865009212002200a79c035e48f675c0527f33926ebdbb8dbae89c0a77f1e7ba229126b9fa97cc6012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67feffffff2221d7671a04a3fdded4bdd174be3f1b9b0553bbdbd4c598d79f206f2eafbb40000000006b483045022100913d8dd7fc3e2114bec634886b0189cc400cba036c168228b9423f5526a9d361022008b3b02d3c0270911def718c1859aba34233e2e3c7327e2f5ac7d1a7fd65b9eb012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67feffffff6bcb7fe694092d3d0386df96e884c6edc7139922f85fb4a139bb900127e9abf5000000006a4730440220103bac3e985912b388f48cc979f82821cb637f690fdd497efe4fceb86e00122f022026173b0e6a5e5eef7483b94f7589e78810eae8f8249ff7b03876f6ae24faa19b012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67feffffffa8d8b433b92a6f95afb76c5071cd58a0b175f4a12851d2c952e94c5d0d76dd7c010000006b4830450221009f9188ef194366c3bb4cd520eb9d8a68c3f2fb6ea591f671a00039f05f67b9420220579874562e721bf8d07a34adc1ac587b6b48609100c43ba8e8bf180c86763adf012102679a681d9b5bf5c672e0413997762664a17009038674b806bf27dd6b368d9b67feffffffce861786ff0be984530fbf972b68a63412f985f6e46b7d324f654e4ca1faa7732d00000000feffffff4ef07e0e00000000001976a91466f8da41c6bb10975f565bde68b5df07003c59cb88ac525457030000000017a91446e14b4a4ff41785017080cd63aa5d17513e185487b0feea0b0000000017a91499bbebbdf7f2dc038b904103237765a77282b42b8725124900000000001976a914c78ac0df6b8241075d66f7f986653604a2c6a6fc88aca00bd100000000001976a91479f1db0274de574d49f9fc794b349ef81529fb1888acc03b47030000000017a914840f4d27071f400c5674b1a686235cb641ef34b887d1a320110000000017a914c2f1c77b4ab921d9a2b7a36b250e4ac5a29afe9287063e05000000000017a914d58ee5f1a2bc153ce58145676a679d7b31a1a5ae8790a434000000000017a914a61b218139c3cd63abbfc6d221f28019d86837d6875fd64a00000000001976a9148aeadc4ab5fbdf6fba1396405388868395cf4f1b88ac524036000000000017a914f9e4dab5529cda97fe7d0ea9c6dfd828c9160c828750f80c000000000017a914551343b34a385e392562ead50b2588ee97307c378752220f00000000001976a914633a3cd7a6ce04165619539a87ee5671d0537e4e88ac80841e00000000001976a9149f59e0163f592c3de094bc12ae338d8140c77c5488acf0b47c010000000017a9147b7f9a5fa10a45fc828d6a47ee6dbbbb2364cee2879835d402000000001976a914221a6189701ce0874c4ba6fc0f91579f68f0589588ac20402c000000000017a9141e5f0577643f2c17ecd5037034824e6b55f2f37f87f0874b000000000017a914a70f43b2b0bded27e58ba7997e15936d86b5b4cd8740771b00000000001976a9142388eb0f84b2ec9d0e35ceda9019e389aee2243f88ac6d1096010000000017a914e0a8d9fe6832f56524ad51e40c6b34cc212dad4c8770032d00000000001976a91497dfc57e73ab8a3b9bda027b79a28bc2e9fc193188ac20f12f000000000017a914be3d917f8b403b3e6b1cf900e29d686bddc8ce6487d0e90400000000001976a914f04d6dc750f0b2d3e648ab5afcc5b1c2cedb36f788acb0feea0b0000000017a914e366f89679d01a89599c9794a35872e5f3cb3d2987bfdbee000000000017a914af5c84f9b702a4c60611b6272c6670c4e961474187b49b2800000000001976a9142a80328a0c51051bf0e76eddbf5342178128096f88aceaca0a000000000017a914370628b7101a7ff461de2ab0a80a8703317c781187fa6f0d000000000017a914166c9a23dc39fbd57e58ff794069d083933cbc4c878c6fe109000000001976a9146f4bceafb26023db265d9abc763ab2ccbd0213ae88ac5b0f09000000000017a914fc8d98b2a4ea22f24e50261fd065afd99a8274a08703b404000000000017a914d1803af27bed138379b501e91f368d500b0b49e78724411700000000001976a914e39546887c31afee7a067432902239f44e64406788ac1fe504000000000017a91462830624a7d20d6c86ceeeac5a3e7bdea6773927876ea200010000000017a91466b967a217fc91d260025d46c9c9eacb746b5f9d87a7262400000000001976a9140f530ba894b185be3fd809e3992145f533e9953688acfcc40200000000001976a914df9443d2b7b497d1e7a950379f95be6ba9ea562888acf04902000000000017a914ccfccb33575cfe97d39b6d0d0fad8f09cce2fe1a8741536001000000001976a9141fec4449c7ba080cf0c85eb87ab0c855f0c3959d88acb9564800000000001976a914e5527898cbf243993a8b5b967120cc9a9a96d09288acf74e0900000000001976a91434f2329553b026ee1aa0c02dc0743ae0cf0062a788ac67aaa8000000000017a9149f2ad2868872be8c065cc9e2e20adf31e0cc44d58729493900000000001976a91450c4073088d9ecfa0791033d17a992e8b779f12788ac95746200000000001976a9142a27eb2171827358522c29a659aaea0f50b7757988acaa9ed6010000000017a914b8d0465ed10eac76fc86646ace6fa64b64cf357e8700802f00000000001976a914faecf0266209f760f5d5ec498f74a0ecca351a6288ac87881500000000001976a9142e30c6bb9396a24c4cdd56f20c74f7681d812d2b88ac50d127070000000017a91469f3751d9b18b84c15ddb3d1a5349657585c61a78730bf47000000000017a91465a11389f21ba13527b1c7629e999719f324125987b01df5050000000017a914dd005ce549e1a57453dfcb8fef3522d83f069432871ea703000000000017a9143ab72a89b9706691ac4de3871e0f63efaeed880b8768eeef000000000017a914b5022f11a874eea98b9f7e34c80d143f3b03678987105b6e08000000001976a91487db0ef6cde94004fabec6bb7dfb675fd691b67088acfefe4a00000000001976a914b827bbd222b251930da17d0a86ba0c5e19e3b27c88ac331c0f000000000017a914a0c63d441be7fd967ae9ef4af028092b446a43cb87f04902000000000017a914cf8df73caf54d7a8e54b1247c51b2566ae128fc187002738000000000017a914de796fa9d384058fcaab5b37c45803af4a73993187aac0c8000000000017a91404308a751559f8af188dc67a0dac238447e9141687bcc5ec000000000017a91428c28ea4ab911d65d2568fe2a2ade143f1804b158738cd2900000000001976a9146e9f5b3aefdd8b079e2d77a682a6276640b5a77988ac2ca024000000000017a914185f5481e1c5ab6d9926207fbfd86d85d51d7bdc8730d397000000000017a914f4761ebdd81b9b7e06a207a7a3d55332d016db3e87fea706020000000017a9149841711ba7b69aa821e5e4e78b07013789c0f1cf87f07e0e000000000017a91454d1d3982910165eddd607622cf2aa2518cf540587467a42020000000017a9140ac0973483473fc700352483d72211ef74b7f77a877e843400000000001976a91405da50df6705f7528c0de919a87a02ca74b635fc88acc94178000000000017a91469f37547d53a98c778289f01066ab23b4168090587d8c40300000000001976a9142889396473e1709927065dc363210386afd9940788ac00e1f505000000001976a914c625e5f34f3b2617326adbae2e73a1bb0a6be37188acf8d6680c0000000017a9142d717f7aa62e57ba6eaceca169cd7f63a54d679b8740420f00000000001976a914f4f89313803d610fa472a5849d2389ca6df3b90088acb0693001000000001976a914f63784063000439d873f12041e8799d0252db89e88ac21dc2d000000000017a9144b098f67e04f711baa310758169f129cbda6385f87a6e60200000000001976a914494ee9282fb208d60a6765c11310a0952428013788accbfd2e000000000017a91469d1a07e7d5fcb62a322c8fb24ae76ebe3c883748730d397000000000017a914ffe19b0c48d473db72ca2d243476183b64b4f5d4876e5c1d000000000017a914816b46471ee03653597995a2dfa65f0f39eaaf0a87303e15000000000017a91488d8d90979c558004f248cfcf0ae6efa3061100f8760721400000000001976a9141302fb1cdc92135634e69f69feba89070a1c1b2f88ac0000000002483045022100b7b6368e45383b2da463ba56397a1966b94be5ef860ac95f1067e62a4531e75a022077bc58f3ea606219fe086f291d39b805faec10c848b525f4997f32979bab5aca01210253a13bae39c5604dc4e9634c10e87e33d0c2d1a618efc0726af5a4a4ea81f7abb6e10800",
			"blockhash":"0000000000000000000ae5f893bc9156bf24938ff6ee1d5a1555a6b7d82ce176",
			"confirmations":13533,
			"time":1561311885,
			"blocktime":1561311885
		}"#;

        let _tx: Transaction = serde_json::from_str(tx_str).unwrap();
    }

    #[allow(dead_code)]
    fn test_kmd_raw_confirmations() {
        let json_str = r#"{
			"hex":"0400008085202f89010000000000000000000000000000000000000000000000000000000000000000ffffffff0603aed11a0101ffffffff0188b6e11100000000232103fff24efd5648870a23badf46e26510e96d9e79ce281b27cfe963993039dd1351ac3b5e4e5e000000000000000000000000000000",
			"txid":"1b1a413c7205dc07f23ef60ca04d29ca33d72e9f6c473ddd8b02aaac53fb8e7a",
			"overwintered":true,
			"version":4,
			"last_notarized_height":1757600,
			"versiongroupid":"892f2085",
			"locktime":1582194235,
			"expiryheight":0,
			"vin":[
				{
					"coinbase":"03aed11a0101",
					"sequence":4294967295
				}
			],
			"vout":[
				{
					"value":3.00005,
					"interest":0.0,
					"valueSat":300005000,
					"n":0,
					"scriptPubKey":{
					"asm":"03fff24efd5648870a23badf46e26510e96d9e79ce281b27cfe963993039dd1351 OP_CHECKSIG",
					"hex":"2103fff24efd5648870a23badf46e26510e96d9e79ce281b27cfe963993039dd1351ac",
					"reqSigs":1,
					"type":"pubkey",
					"addresses":[
						"RTPBi5hpdSUARnh9gGahv6tr4ppHDwAkxD"
					]
				}
				}
			],
			"vjoinsplit":[

			],
			"valueBalance":0.0,
			"vShieldedSpend":[

			],
			"vShieldedOutput":[

			],
			"blockhash":"059ad2e93f92de1ff80432ba1227c83739ed76bc78f41630dd6a773dc6595dc8",
			"height":1757614,
			"confirmations":1,
			"rawconfirmations":8,
			"time":1582194235,
			"blocktime":1582194235
		}"#;

        let tx: Transaction = serde_json::from_str(json_str).unwrap();
        assert_eq!(tx.rawconfirmations, Some(8));
    }

    #[test]
    fn test_qtum_call_script_pubkey() {
        let json_str = r#"{
			"blockhash":"b81f26a919bc9d792aeb056d6eea5340b7e334aa3f21144cd0f3c663286ff870",
			"blocktime":1589537936,
			"confirmations":2457,
			"hash":"fad39a18206633258a0e77cc59d4606553bab374d05bb3d56cf3f0a701bacfaf",
			"hex":"0100000003b6eec8104d23c90a0061b3b4aef7c16bab8fb11e2a3764540716e8207dc356eb0b0000006a4730440220409ebe0309a14cca0761b52594b3e4d5c52ad997f0e7f3b918416f9a1ebdb28b022005f2e391350ae36cce3a28b0ab393b38760918f5d9843e9f06126a011f2fc6c5012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff9561b9a318c5021e0ad76ea51a97ceb0a7ea8217ff5bea92c2249484ebc412380c0000006b483045022100f753ec48e19a84768905970689a9166dffa4ae4202e0d7aac61025b739f990d002201bbdf4226c969747600b802fa14c1480eb0667b7c7d9b76867b2fb9f2ab11e94012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffffdef6489dee6cd3c88f267176d4aa07cff600c034148b1f2726ebd785c8cee951010000006b483045022100e128b8380c02e76826cce1db03f9d661a68c5ef2198d34e4557455dd2ee3a035022015b66c6b8c44803b2c8c61f8a65ef57d727043582d431377edbbdf7466a38d8f012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff0200ca9a3b00000000625403a02526012844a9059cbb0000000000000000000000000240b898276ad2cc0d2fe6f527e8e31104e7fde3000000000000000000000000000000000000000000000000000000003b9aca0014d362e096e873eb7907e205fadc6175c6fec7bc44c215c80b80000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88acdb6bbe5e",
			"locktime":1589537755,
			"size":594,
			"time":1589537936,
			"txid":"fad39a18206633258a0e77cc59d4606553bab374d05bb3d56cf3f0a701bacfaf",
			"version":1,
			"vin":[
			  {
				 "scriptSig":{
					"asm":"30440220409ebe0309a14cca0761b52594b3e4d5c52ad997f0e7f3b918416f9a1ebdb28b022005f2e391350ae36cce3a28b0ab393b38760918f5d9843e9f06126a011f2fc6c5[ALL] 03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9",
					"hex":"4730440220409ebe0309a14cca0761b52594b3e4d5c52ad997f0e7f3b918416f9a1ebdb28b022005f2e391350ae36cce3a28b0ab393b38760918f5d9843e9f06126a011f2fc6c5012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9"
				 },
				 "sequence":4294967295,
				 "txid":"eb56c37d20e816075464372a1eb18fab6bc1f7aeb4b361000ac9234d10c8eeb6",
				 "vout":11
			  },
			  {
				 "scriptSig":{
					"asm":"3045022100f753ec48e19a84768905970689a9166dffa4ae4202e0d7aac61025b739f990d002201bbdf4226c969747600b802fa14c1480eb0667b7c7d9b76867b2fb9f2ab11e94[ALL] 03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9",
					"hex":"483045022100f753ec48e19a84768905970689a9166dffa4ae4202e0d7aac61025b739f990d002201bbdf4226c969747600b802fa14c1480eb0667b7c7d9b76867b2fb9f2ab11e94012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9"
				 },
				 "sequence":4294967295,
				 "txid":"3812c4eb849424c292ea5bff1782eaa7b0ce971aa56ed70a1e02c518a3b96195",
				 "vout":12
			  },
			  {
				 "scriptSig":{
					"asm":"3045022100e128b8380c02e76826cce1db03f9d661a68c5ef2198d34e4557455dd2ee3a035022015b66c6b8c44803b2c8c61f8a65ef57d727043582d431377edbbdf7466a38d8f[ALL] 03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9",
					"hex":"483045022100e128b8380c02e76826cce1db03f9d661a68c5ef2198d34e4557455dd2ee3a035022015b66c6b8c44803b2c8c61f8a65ef57d727043582d431377edbbdf7466a38d8f012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9"
				 },
				 "sequence":4294967295,
				 "txid":"51e9cec885d7eb26271f8b1434c000f6cf07aad47671268fc8d36cee9d48f6de",
				 "vout":1
			  }
			],
			"vout":[
			  {
				 "n":0,
				 "scriptPubKey":{
					"asm":"4 2500000 40 a9059cbb0000000000000000000000000240b898276ad2cc0d2fe6f527e8e31104e7fde3000000000000000000000000000000000000000000000000000000003b9aca00 d362e096e873eb7907e205fadc6175c6fec7bc44 OP_CALL",
					"hex":"5403a02526012844a9059cbb0000000000000000000000000240b898276ad2cc0d2fe6f527e8e31104e7fde3000000000000000000000000000000000000000000000000000000003b9aca0014d362e096e873eb7907e205fadc6175c6fec7bc44c2",
					"type":"call"
				 },
				 "value":10.0
			  },
			  {
				 "n":1,
				 "scriptPubKey":{
					"addresses":[
					   "qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG"
					],
					"asm":"OP_DUP OP_HASH160 9e032d4b0090a11dc40fe6c47601499a35d55fbb OP_EQUALVERIFY OP_CHECKSIG",
					"hex":"76a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac",
					"reqSigs":1,
					"type":"pubkeyhash"
				 },
				 "value":21.48255765
			  }
			],
			"vsize":594,
			"weight":2376
		}"#;

        let tx: Transaction = serde_json::from_str(json_str).unwrap();
        assert_eq!(tx.vout[0].script.script_type, ScriptType::Call);
    }

    #[test]
    fn test_firo_sigmaspend_input() {
        // https://explorer.firo.org/tx/d4b9f5a01a43b1d592999f9fd6fe64aa8f63ac42abab43090938321064c1ec1f
        let json_str = r#"{
		  "hex":"0100000006000000000000000000000000000000000000000000000000000000000000000001000000fd2805c4aed78fbc5b719b0887d2c96005214b5e6d82852e4cc189bcd5fa1860caee2b700100dc1b2cd57ba6a72246fd3935c2930e8b8465860b8fbb6ee55957433a31ba705f00005586b9add2ab0301cb65dfa154ddea1ae2cfd352a4a9959f8f0956050b167e7a00007c294578d92c7ab5dccd2379461618124af985b5c3c1890a762d3b9bdf562cc2000015bea1119f1177726d155ebb2f2f4fa7e1bbc42e7e063e6d8780f4fba17bf74aaa907e1e669bf60fb97f87fc45f6d432a39a01119717bd35f9c64d0430e8a5544026656243aff1f1fdbc491c0ce01d63969e488265bc938c60efd0214b8e40b231ae85d87a3e7ca99080b850ff5c0c2e8171534132a0ad32fe8892347cb920a7768f58a963e3604b6ddd02c282c20bdea658b7dea637494af9b3619ed2e11c2ad28cd870fc1bf9a9fecdee794fdc9001a6e06330ff834f87d5de64ef5c9aae0e26916946fcd6668f6c12ac7c705c38470ad572ea71fb596b5149e982bffd0c238865a693aab5c8dd0f91143eb3b314f283379e3c7a708ea4faf4430137fe71453995ecc3cad8a3816b28bda1c866821cc389748a62a8edb4f2a3493dcc5b166c37a0cd8390fb41f21e9464dedafe54d2b3b4a3bf5881584c96ed537f00cf5c6dee475c6b8a252c07fb5f6c822547c4299db862653e3ed20c867eef508043f5580b404bc111f14d5839789c774d71a5adb14efe8b97b6124f084f22fde1728d483ffa60365e517187e9891404cdcb41f3823e3c8fec84112d54ab9537db9d0c66ce681ed18a3025b0982d06ce00c1a0db670936ef7aa4ec3059296f7a56e5618da52e155ef91cf3ce952431c00a3c5fbc422133f09ee5a76d7df34081fee9e0e574bee1cdfe83e964341813fcb7b219024f2a2606b7306ea9c61330752915c459bb5d9b1969f64be1878814ad39b41eff50c60e495472ee628929d22d468343c162a7f033e1fa0ebf039c529739a5f071d86f1bda27fe9b4dee8ed7faf8e6e9ff1d78a6f74498ea3326c05cf3d3850e807269cb073776bb8dd34f6fce30307e8330265ec89112bf48861a122df7bdd203af1cf776ae4353ae09e176eeaaeaad98f82eb2a9277fa9795954590d29c8c0ba84aca16850800ba1950b16430af7cfa81add4b225de680e9c5690418f70851295161bd3c2c91ae366531c62991a73fcd9c3b31fa8261fe29761538de5ddf38f6163c9eb57609888bd6d177f395ea12593807acd32a8d9247ab5aa8db8af62d0a28c376c7d096c02ff9799d4e5818abda986a00002548ea29cc77d2aa5cb91850528fd7f1e209686a3cab3c8f8965818d0063e4f600003e12c6dd83593e2303cca9b712b69d121b2feeb19a6d070cb06e465cce92653b0000ab18e8decb34fb0cecd0600ca124786f133401ac197d9615e4dc646ae750b2db0100c40a4788e12bac6c26e3ca1d72925f7764f72dbde8d487d657707e880849b7bf0100ed6189ae534e3d8de39f2e4046a43486f348065a85d12860c39cb85fefa6fb4c01001049602903683e070a3fcbfe160bfcf5c21094185cb9fdba4262020860b12b2800004cd9d1ed019992afdb63ba2644017ac9938ff11f665ff57279d507fe90de5be6f56d5554349f7d637ce0d0365a0cc056de61cb9a6fb10cfdfbb83d927911018b1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd2103a2e831904e261074139899446af2298307f7c5a59c2ff9f7d691b66c5f78a3e840c2e23decf2e8f845e8a8e9d18cd81d9eb4a5d0e7d53624d34c0f1cbdbb2e05697b427bd7004c14ba20370d2384a5967a8bd5921202dfa382e99c1c49db9d3182ffffffff000000000000000000000000000000000000000000000000000000000000000001000000fd2805c42c0bb28d0e52f8f3d0a139446fe7e4da31cd195f7c3058d8e6cffea3b155dc32000092f7d2a8a1a4892c983da5a20fdd657957fcdc0ebd86485e7ac1c7aa62a92dfe010023353a1603f1d14720d48a80ad139a400862ecba29637b38f9f1162e0d556ee3010095defe584076981c1c6fc020d842a87711fec85b01aecf288282a564dcde2c2c01001597a0147c89ed6f0961dfa572909bccc58eeadaf646ce7ac29a6e0fffba2201e2450eb859d4560a7c1aaa18971cf9610f5b361c97207c97d947b808f08f8767f950b817d1e653b582638fc69da3725d78a0c0d41262288ef5c9e06a2c6412165b0818786e2d5d821f707a59226a2fa2cdd009a60b91742956d315601b8390537c636f5a0f523546f283f0860ee7c495020e2cce56c8b9814e3997a0d0ae224b17e8b464565f91a16fa722546ea707eb73f16613cd1a204491c6590652f2df7876253d3f4c315f6a2481bb70f42095dc54237e9d1cffb49072762af07ee6cdaa82daf8f85039756da027621e3b4dc67596450aeb638894c0cca4ab69a0bdd1fff7a2fb4abacf33351dd0046d0a29608be1fe6fc91f3a349b0b6940f33e1232af2099ad97a2e05442261c3f25c2e5d241203eb7f983df4714f016692748d1aea107b3a71013092143cb4f4085bbf4f23064af7250151b6f7fea5c08933fae00353dfe11966731377d839e89aef9dda76c46a2238af18a7932c247ee78bcf0400938428e1c6c368b9af88891b40f07b836a7abaa6c638a9fa6ad2f7add6f2ded013a3a838dbe5c81f4dad69c98df205dc507004471b9f7ed6ce5eefd20a58db720e2479f198654e385254e32dfbd0161f63b8a3d3c8e81b9660d78626190acdc6a43b70bcc32c8b7cf3420323c43cf415ca852eb529f931c428c8f3e7e42a70474d0690c6c24d4f5840672804b4208a1ad756772dd80e9e90379b7da72c8ec8729f75c152b1fa5705fb894f187b6d3d2c855ee9538e4c58f6d15cc39698a7075164de691658d865797faf88703cd59b51cea8b96a6027546ebfe3007a0356d14b8480f30481ac74fa635199cd87c2f597245de45b412d32e57a9d0c67f10e384da8e1b6f125ab62691788151432a7701dd31ba4b4c6d7276217f1ddcd26b42d2320d8e8b81c8c845d6333b009b29633ad2afccf37f4259ef5eef236a6b399134aee916327c44f591866e7392110597e63490fd43f743f2f227b7284d991a484a3bb307db65821fa9747af6cc277b465c5f524a650188d8421681c3a631c0e964cf9ac500004cdadf027a224a6746819329ac51c4c7a295ed9d4e75f081e8c590718e0a762c0100c448a2bba9efe5a4e1473a1ad9217bc28f7a628f64920d400b7b37097f7e7b1f00002304ea4f401f1c08e6d2dfad3e53de99062563c5232cbfb7cd97660177798df90000d315be7ae3d127a96f680db963d23c0f8759ebf0d7e1b5db573851ea7974d3d000009faa7df3c9c87ea2be077274952505173477e7c927fa5433e5a508ba2b8851a800004737d94352d548efc6479596ea413b523384f839715479295fba94928334dfdc000001bb5aa89712cb71b90006fd545ad4295ef4e0e8f0d9362f73fb53e32d1806d8102e4c4cbfdcce690e04dcad8f8354c90d6a65ac2cff03c5dd76c6f06f770ab01f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd2102d9cd799a2267321b8109204c87e638e5e040bb9a2101a0ea12e43af4ed7586754046a5fca18239564d3bed40328a7b1472e1d0941337b68b6a7d82d62d5023278b7f6e3573a22627c0a30269ddee7059968b0aa957dbd041e6d762af2fed06de3bffffffff000000000000000000000000000000000000000000000000000000000000000001000000fd2805c4e9fcfb78b81f1cdfb53c62417cc1b362f52532ccdf055190787c826a84de3e9b0100779985d4b1b1c5e701a008666aec2c65400a1d0fd684cebfc8e77439d5362c16000097bfb9c79218528bf2515a6c3edb70428999afd348fb1ae6d4a053cf85b3e28c0100cd6eaeec888a2f971fceddbff518a2cad03f66d28cdd084b7f406d5fef84eb150000151ca45f20ef7e7b95cae35c6b08af8c95965b156dac5eb748dbe9db067c63ebfed427fb0a30e422bfee913ac7106877ab2838db36e992e7ac9b6cfc746e3954d172708b64a6ed0c94a460dbba986ecf0592d0a289baa3369fa7d9d8181973d92a5a6f1615a5c4a27377739fc172af3ca83c8d2bfe1f4c4163851c0cf7776773f29a4fcad864e11eb7572e13fb02dfda4613035a2df00ae21aca80c6c5e345deac538a4b4a8c5c9998f35176a68de0ef38eb25b036810d440976d556e4e6acee2772bc0c8961dd140b857ecb53a029930600267aec1f7c2358b9a874ebb15fa3a2f14afa1645921409f50a6b3d9f38e841e580b460f90c3c77cf764a77a6b65c657af753c54e27ed849a03a2394c99548a46d9719dc563a2f6e19bf909845b38c028f70ca802d96ff47eb3540ded69554df3c0e75da124c737c97d36590762f6264f2b131301e997a05b6871f3797ea7e0e59db4cdea0688681e8df62341cee745d8ad9defcfafa8338ff0b7d477ddd93b9b7b1d805decb3329334f4a7990d1766887c346a10677a54ddaf57f7f81e004a972af7281173f521d6970b145294fb9a0d83281ded291a6aa00caefc2ffe6a66d7c77d5701037ed012192bb4423c4631fc08d89f4f8d25ce3be9b88dfe051e6653a18b7af859d83bc222bf413a4a024a8023040e11485b4a70e2d0e7763d601c9532d27e02483f640c0a5c3179a12d53d8200d00cb0fe4c1cbe2edc06f36c278af77a095eb8a11c9aa7f0f53d88832464262876af5a2b3f1b4ca357e8f7a8cddd9b4d3d6a698590560eb5dfe1209056fa8758145f419175aefe73b36296c2e3d762ca03337f180a879d7edadec45682f02f36dd464a6e049d05c05f6ead47a0da6fc62673891d3d01e08adc0c88a5fc029ef342384e051e69c451202340f8e3e72f78060c645413c8653c07c0d08ed3a337f02eb92677960181d43b58e0808136a846400bef3bc34e18e5e09c3fb2b0073bca169884bed4592740c61b3d5ffb49fee0df0bca7b52024df3dcaa00f4d28077364db9b252cc06aa85f66f996d19569dd8f961fc4289ff32088ace04d75c18b00001030f2a9882d3aca1cf5a84bda8ac7c92c5bdcaf492295e8cf6080104063ca180000925eda046bbc24b27d4651dd2809de2747885f63d9ebdac1143e574462d4b69801001d7422d3ebaa9401f97c489a85086a3b48d09c7a69208ee7d7e6a7f0cb8e32cc0000d2cfb0d15de941e0025f65f0161d55685f05e4ce8cabe6f5b660800a4efb15950000490929b68bf5e1bd01adcc522b71aa949f6e1393703b25124b82d5f568a2aeea00003d2af0ca26d08f44ed80c43281aed6dab6d491fa6b2b4739c06c86855e5cb8ea010015cd819b456a851192b8caf923e76daee4db3cb38b3cc488796a151d0ff5d9a1630423e3e068c1ad988794671918f63ffda1ed627801c22a72ca7be0085d7e9f1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd210305a48450bcb57d448832a95486c294d89834523760e74f90e682500b67c9064640fed06a1a2b65d15bdb4ef65c33c804d480b0217c505d59c71b7efdc9b189f4ac2e6ce945df1c37f2241a972e8da09ff119c14e27629d6a026f2316d427860831ffffffff000000000000000000000000000000000000000000000000000000000000000001000000fd2805c477c1f4013508aadcb047c5d4d073df0533956739522240935e1aed3339c761a1000071c754b9b34da3ee5b83ca6a0c3b834569ba5bdd3af42207cf2348e02b6f50860000812fc9b026abfc1975a96f0b09b4f16c2a07aea16563e0b2f80d4c3bf1403df50100293dc3b5bf1168aa661bf2e94e9a56862e1225f592c38c5c11042be03e92f83601001517ad2d049e8812e5d3dfae275ca406b224b6c71ee7746f69e2ebba478c0ebbf78f1a93d9ac05744c1c250b5163939c528e8cedb57371f12527caf67a184c99cb32b67f380ed6e91da76cdba1091e912250e3a2a7d6c3adc1066bb037ade7e3b3f61a6a2ba0330a9e4dee6dde7755be5a7474cc951949ac8a21090bc77f7289d0b1fc3e9c059732fe96083a1a73081d9557a17c4e1a6ddfdc72efe694680131bd3ca8ee9360b0fbfd45f3d60f516aaa22da398d1e4368f1928fdc366120ff01ff9d50257cab74a2ca473914827b2237b20eb07fd13afd0fbdcdef467e92558f93b202860d019976ab8d226ee1bd4e92a029e60c72f952b791d8addcfb11f2b66b06219cecb229d0d74d07e57e3dd9caacc0f0e57478e313e84149f4aaef47c8bc5d1a6efc51546696a5eb34b4b8f46f140dd84764b53d273286f5f49cb85988695526de83b387fafdab97ccad5377009fa94f11c5690f7a7750785ad0cc5125f7755e7569aa9d66e92b6f902fbfea559b79a666f2c8486a4b60a69ba497fe3f45976e2f6f62b496e7bc22615d43a201485e52b7647887fbc13522192aa6089a9673ce11dde3873b0dc4fe44536d7530e5324a3eac95f3eb05b27eaa5ef72e808aa6808c5e972b7cea71ab5cdaf281d7ddf7ce9053ba62c60d10c7f61aad56ad36728a2056da78183cc19ee9d8974b4c6a7421e6c18fca89bd5608aa1744be2710fe5224ce3a1f451884c74ce3146917ec81c8a798cabd890981f74c631d25dacb79190a283fdb5cc6373c828893411b3b547ac764702bb3b4244ed4c37f470de98ca67fff063060c302d65cce3ba3b7898766959d893017963adfa8c8381a3235e3a8fd00ec4ca603dcbae1eb8041eb3f8d0819cfcc746459331e62cb512ca7a4439ebf7af619febd05170947b176ed12391a0b5f8fa4b913e8782dbd31073450558c5be1a289fc636d6522953c0ea8a6e8c1f62ce1f1f80626154c7a4ddbfb3d4a5f71b20a3da47b350d275c5c6c5a67b6e78a5bc1169a67d5caec220b13e1ec076f60364dbef9636d1643cc02b6a758e77b58bfc1f435f33df5d22d57feb31125000009374f988af7ba8879cadb00e7c35a34acb87c2d59bb2fae4d4b5387bf27fb890100defc344c95c73ed397d059bcf49c8cea57055f4b7651f656658a7d3ef1f4f8b90000570b35d424e33c4d77784b2db0c056bc6e66fa3c60dc2e2398049efb8a41165d00003e3e88eed2cdc564914637bc2c37f4f53faf2ed1023003b947cad66b2fdc151901002dbe998be1456b6bedbf57cf1107dcc67fa3efd21def15c453ac8d08526b04fa000099b04830e653d1dc6b65011a5bcb50adee726c424f4e44d54bf1fd79b83315270100412f9b1ff15bdbaa0100586183f67ce6e0e89ae3404e212da84978e8341d8d789c0bb429717bfb484b50743743d9276cbec48c32fe82d14994257af48d85f5ec1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd210310931a8798da56262c681a28d09329bbc7862a91a3688fd90a90956c188bc9ad40fdfd16a2b5cbc85335acc017bad488f829e321e7ba27f33e51ce1118cd835086542751f4b85da667bb365b3f53fa5c84827482c077c550d32079cad2965d9a5effffffff000000000000000000000000000000000000000000000000000000000000000001000000fd2805c47252d557c36f45288f287d81c1f31f5d51b778f905b41b79b9ab15554a0e3b5f010036ea61b6cc59ffd61c2b7a6599c344a7e259e0f67b2a42940fef260fac7619df0000731f2649a28ec62446524273557bbd5a9377684d50288a244c1738a8587148ea00007cd2836bf9359994dc2cd9267ebc42414d8d807852c62ce2523551fb4f2f30b900001548c989ab140a3ace02c4cc4560cb7770593c3a1576262d7b3e6700098fbc5420748d3ce99f1ce057b928b280f85a93a0b08b5aa7eff9922bf79c5447d8ee6af172c99743ffdb4b5d21daf1014150c3f9014c242b3f6707d272089bfdce87c51b34e6d46bfb5d4452a7a9d16deaf15c11f44e112a885fe07770df9c959cc77ca53a6b7ca2a883fb1fb040e423a06e79bc534e21f243f92a25d59094b195f1a02bf3348105e720edd1bfbad05ed97845c9f346340c1aa06d4983d14d5a5b08f19be2dea8ac9425d0d2a69ee9918ba5eb0273287b83f97360833180805987703ab123ebc6e5f099e2a4857f34913f06cd609fa717b2d3047a148189046484b88460ae7f572a3f213f375a3d7d2177e231f0a3c0145dfa60be0cc761785d290e69d154407019e2429d69729374638c5ae06147e7499dbae31fe3a1684c901ee12b27398201a7916397b6cacb6b6972db15117288122ba01ec9c68d78bfe0e8515525f2534d8401a92faffbe44f886406f2e2ae077c779803d197de1bca5a94abdfe85862f685c7158e1fb1cdc1ce6abd7384c1443b93e9984779c8d0d7167d350fd4c1b9abe1be6ae136612ff23a4c7830df1e034f544fcbeb54c7a1d0885ebb53d0123ac0e30b93ee4f3533da7ba808b848c677bda4c86542c360dad96fb6829e2197a07c108ee339436ef26f352703d6353c803522e57beeecd79abebd7d6bab9c4e8c4d6cc92421a93f977cc353e23fca5eb91dc87958e14e69b52afb8dae1f70972456c8be70ae86c28ca1798cb09d93732559a259e858fa157b39fc022215d829aca9841f07a99f4f40f66b55d0b96f7f1cb87467ae6003c8b258cbe9af7b2d2765bd420b905f9b0e5096b750bf156526174532aa43f6e7304521d44e330488f99fb124280cf863020332bc8cea92cc71eca67855d42da5d6ce58875139d23d1317dd04085bd331a383542993fb8cd83f9e73c9becebb7c7b840f097461b0386459992d4077bfe3118f8be2e47c1539f088a97457b7013ba7d7e68f398eb55c075997c8c67d9087aa8d9c9f44c42eff98f67c4f45f9c4d7d13315aebc67fd0ebc00003363076abf579ecf927734e9bcb843037f684278c659e183c0f6235d22dab8180000d36084ae243828756d54f3b6908891e5fdd8deb6368e9d4941407dcaf019ff0e000092c21af2d0bb0c506454fb6f6cfd789649facef1238c8e6a137fe9570cd2562701007ea89f8a2c2d587a3cd445de70303e65444e4924031726690c4a8f8bba3683c400002596a772df3afc96d64955a1cbdc1e8c4b67c85d9ac010b83c2f8141f6ba63410000103b07d9089f0aee1d25631385a0d00b30f10ac7ecc991e5910afee6bc6c7fbc000082c08c24863ec43bef2fabe1ff8aa5b6acd37dc2bacfaf2e819078820b78e80fc03a7226372e5074f2b83ccf4aaa92e89225888f4e35bd79760881376f34e4a21f00000000f9029500000000e244028be6278500ede0782231c1664f6a40f5b01d82c83b21bd50248a717a0c21034bc9bc3f623ba555a6abceb0d5a491abb9d067ad76993f6b3544a21b34a2237a40e4e3733e1191bb6731897c6b691a00dbf86c94a163047b61bd4b24e6250c3f1c6828ab5a1d6266f09323e2838e304fdfb636eaee1ec3e90a535debde88e31ba1ffffffff000000000000000000000000000000000000000000000000000000000000000001000000fd2805c451f9b4d6931827686e1c5ed60d09fbd7050c093e7424aaab9c84e3d11ec3349d00005a582a32d0fedb32b3f7fa0ef118bf7325a95fd46ef846a89c155fc6ac4271130000c6b4a8a988c610dda27c92102db5575f4b6f42d8f32d8ec253b25427f784a2490100f5e5e8e590818e013b67f5c52b5cebf59cf73b2a03912bcd0f94d3c4380be27d000015e48f758a84da02e0a57fdae68e0341bfda81dc42461ed7789cc05c2a47f2c1750b980e17e873bc8ef4b9b07e05118f4d9fdfff6577e65540b9a2a1bd687e6e8ff0364281cbfc246f7700f7ee22eefc3525ccb2e77f64d63a92273649ffb0072430b4cfdd301aa900271f3da4375165fcd6a86445a20b93846fc0642bc6c268e8f9b3bd40d6768cdf11aa6cfe07c6c1c5cdc01edffa97f36b0130b4d359f33572cfaedbd7fd310d30bab86e429e8aecf8d196fc2715c12548962c8ed14ef097d5c92092d6727a0a5c12d64bc60c65764cd7463c7c5e40f35cbcfd8848e33b4e23c2c9dbf8378954a54361e8ad3c133ca4cdf8ecc519d6c45e4a0bd7bf9b99303478f9ae9c66de8a412f06011d34f536687940e9c547c89be9adce8985c395f4c72b1141d2ae124f1413ad0d067e572a3d711ab7eead482154ce01eb36cf9d8f62b60c75920df93b8921b14fe5b870a06829976e246b2db5fcd104574261dae094b86830b697ced626000abcb58ae6d4bd3dea0717fe6dd7cddec1c67ae4c0e246cf150d4ad5922f91d40f5b02b27376d46e8b9380f0d0b58d7b7eeea1a21e722bfd3295ca469e1f256eb97e9716a0371a4dd4cf0dd0e2ec38692a228b4378fc9a41b449a2b9d6c32cd47918bd47fc24be8566e43227f7ed1f870110bef20443ccb434e46cc3b6d8feee8e31b8da2cdccdcd74bd656cd94d59c26fabed0b85e6f15cacd09ae91b64fb8fc061e0ebe4dc4d270962120caa24e5c84290d994fe2410840149b4921d41b76f837d1a29da04ecaeb01a695df8e633f91105af7f6fe5a26a05fee03a19c5033453c11a710cfc84ce413457be9775c1957533fdfd817f2a99590d10ddceec7fded39a09ef9e9ef08d8f38d15dccee88f219e15301fa6eac574f9ddbc812f5a0a3a629750e48a3e868a90f39e63755e1b46c4eb5c94f4059572f0c05fae3e9d0e3271f0989f013fc5184c236f8c9b9ab05146683156c9a2b11bb0f7a0da3627c0bac58c194498a673801870a3958d3a0582a5ccbeb9ca554071ea446325603e67d876eeb1a05e1342649309ff58f618ea0a21f0cb02a766e3c01003eb745c4933d0f52ee02aaef2a16cb6c52bd164928673edb0f51c7214fa7bf2b010064c7d3b6a7aa59a310eda829e7ac9075ff1e96e01ea1719ee7fc4a5bdb0e9b9b01002763566e427b69e5646cf25048eda2e5806dcc6b7a5e5cdb59702d556840bf7201003c2ee321c25bfdea2ec3df0886c8b165dbf5b6902de7c39682e872ec9cd7b2a500001b3c159cef5dae965c86db605792ace3d987c9559a1be5388acaecb2c753da8f010075cf5f030c222e7dbd81b07096a20a1dc1958211f5327d60e7a499ae7c549188010014de0bbc9bf0798fbbf3255270b7b93ab1e70d76761fb74226653c47b3fd5e6c9f2d757029c6f2a8aa2e73bbebe51287c5a34d6f129a69e0ef7bbd2c13f477e51f00000000f9029500000000e244028be6278500ede0782231c1664f6a40f5b01d82c83b21bd50248a717a0c2102eb2aa581203121be475e55e67299a1aa8715a7602c538421240a2b91b5475e624018b406db4445bf718f2ddb175f83c73f6a1a17dbeefcfaf281fac6750a090a0e7a5ee40dd360279af0f85056341de416b426b79b48bea5bad6b06a2be6277703ffffffff01a8ff327a0a0000001976a914d8ad1e3e2a57c3fe88d241337b0cbabed117bd6d88acd7000500",
		  "txid":"d4b9f5a01a43b1d592999f9fd6fe64aa8f63ac42abab43090938321064c1ec1f",
		  "hash":"d4b9f5a01a43b1d592999f9fd6fe64aa8f63ac42abab43090938321064c1ec1f",
		  "size":8222,
		  "vsize":8222,
		  "version":1,
		  "locktime":327895,
		  "type":0,
		  "vin":[
			{
			  "anonymityGroup":1,
			  "scriptSig":{
				"asm":"OP_SIGMASPEND aed78fbc5b719b0887d2c96005214b5e6d82852e4cc189bcd5fa1860caee2b700100dc1b2cd57ba6a72246fd3935c2930e8b8465860b8fbb6ee55957433a31ba705f00005586b9add2ab0301cb65dfa154ddea1ae2cfd352a4a9959f8f0956050b167e7a00007c294578d92c7ab5dccd2379461618124af985b5c3c1890a762d3b9bdf562cc2000015bea1119f1177726d155ebb2f2f4fa7e1bbc42e7e063e6d8780f4fba17bf74aaa907e1e669bf60fb97f87fc45f6d432a39a01119717bd35f9c64d0430e8a5544026656243aff1f1fdbc491c0ce01d63969e488265bc938c60efd0214b8e40b231ae85d87a3e7ca99080b850ff5c0c2e8171534132a0ad32fe8892347cb920a7768f58a963e3604b6ddd02c282c20bdea658b7dea637494af9b3619ed2e11c2ad28cd870fc1bf9a9fecdee794fdc9001a6e06330ff834f87d5de64ef5c9aae0e26916946fcd6668f6c12ac7c705c38470ad572ea71fb596b5149e982bffd0c238865a693aab5c8dd0f91143eb3b314f283379e3c7a708ea4faf4430137fe71453995ecc3cad8a3816b28bda1c866821cc389748a62a8edb4f2a3493dcc5b166c37a0cd8390fb41f21e9464dedafe54d2b3b4a3bf5881584c96ed537f00cf5c6dee475c6b8a252c07fb5f6c822547c4299db862653e3ed20c867eef508043f5580b404bc111f14d5839789c774d71a5adb14efe8b97b6124f084f22fde1728d483ffa60365e517187e9891404cdcb41f3823e3c8fec84112d54ab9537db9d0c66ce681ed18a3025b0982d06ce00c1a0db670936ef7aa4ec3059296f7a56e5618da52e155ef91cf3ce952431c00a3c5fbc422133f09ee5a76d7df34081fee9e0e574bee1cdfe83e964341813fcb7b219024f2a2606b7306ea9c61330752915c459bb5d9b1969f64be1878814ad39b41eff50c60e495472ee628929d22d468343c162a7f033e1fa0ebf039c529739a5f071d86f1bda27fe9b4dee8ed7faf8e6e9ff1d78a6f74498ea3326c05cf3d3850e807269cb073776bb8dd34f6fce30307e8330265ec89112bf48861a122df7bdd203af1cf776ae4353ae09e176eeaaeaad98f82eb2a9277fa9795954590d29c8c0ba84aca16850800ba1950b16430af7cfa81add4b225de680e9c5690418f70851295161bd3c2c91ae366531c62991a73fcd9c3b31fa8261fe29761538de5ddf38f6163c9eb57609888bd6d177f395ea12593807acd32a8d9247ab5aa8db8af62d0a28c376c7d096c02ff9799d4e5818abda986a00002548ea29cc77d2aa5cb91850528fd7f1e209686a3cab3c8f8965818d0063e4f600003e12c6dd83593e2303cca9b712b69d121b2feeb19a6d070cb06e465cce92653b0000ab18e8decb34fb0cecd0600ca124786f133401ac197d9615e4dc646ae750b2db0100c40a4788e12bac6c26e3ca1d72925f7764f72dbde8d487d657707e880849b7bf0100ed6189ae534e3d8de39f2e4046a43486f348065a85d12860c39cb85fefa6fb4c01001049602903683e070a3fcbfe160bfcf5c21094185cb9fdba4262020860b12b2800004cd9d1ed019992afdb63ba2644017ac9938ff11f665ff57279d507fe90de5be6f56d5554349f7d637ce0d0365a0cc056de61cb9a6fb10cfdfbb83d927911018b1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd2103a2e831904e261074139899446af2298307f7c5a59c2ff9f7d691b66c5f78a3e840c2e23decf2e8f845e8a8e9d18cd81d9eb4a5d0e7d53624d34c0f1cbdbb2e05697b427bd7004c14ba20370d2384a5967a8bd5921202dfa382e99c1c49db9d3182",
				"hex":"c4aed78fbc5b719b0887d2c96005214b5e6d82852e4cc189bcd5fa1860caee2b700100dc1b2cd57ba6a72246fd3935c2930e8b8465860b8fbb6ee55957433a31ba705f00005586b9add2ab0301cb65dfa154ddea1ae2cfd352a4a9959f8f0956050b167e7a00007c294578d92c7ab5dccd2379461618124af985b5c3c1890a762d3b9bdf562cc2000015bea1119f1177726d155ebb2f2f4fa7e1bbc42e7e063e6d8780f4fba17bf74aaa907e1e669bf60fb97f87fc45f6d432a39a01119717bd35f9c64d0430e8a5544026656243aff1f1fdbc491c0ce01d63969e488265bc938c60efd0214b8e40b231ae85d87a3e7ca99080b850ff5c0c2e8171534132a0ad32fe8892347cb920a7768f58a963e3604b6ddd02c282c20bdea658b7dea637494af9b3619ed2e11c2ad28cd870fc1bf9a9fecdee794fdc9001a6e06330ff834f87d5de64ef5c9aae0e26916946fcd6668f6c12ac7c705c38470ad572ea71fb596b5149e982bffd0c238865a693aab5c8dd0f91143eb3b314f283379e3c7a708ea4faf4430137fe71453995ecc3cad8a3816b28bda1c866821cc389748a62a8edb4f2a3493dcc5b166c37a0cd8390fb41f21e9464dedafe54d2b3b4a3bf5881584c96ed537f00cf5c6dee475c6b8a252c07fb5f6c822547c4299db862653e3ed20c867eef508043f5580b404bc111f14d5839789c774d71a5adb14efe8b97b6124f084f22fde1728d483ffa60365e517187e9891404cdcb41f3823e3c8fec84112d54ab9537db9d0c66ce681ed18a3025b0982d06ce00c1a0db670936ef7aa4ec3059296f7a56e5618da52e155ef91cf3ce952431c00a3c5fbc422133f09ee5a76d7df34081fee9e0e574bee1cdfe83e964341813fcb7b219024f2a2606b7306ea9c61330752915c459bb5d9b1969f64be1878814ad39b41eff50c60e495472ee628929d22d468343c162a7f033e1fa0ebf039c529739a5f071d86f1bda27fe9b4dee8ed7faf8e6e9ff1d78a6f74498ea3326c05cf3d3850e807269cb073776bb8dd34f6fce30307e8330265ec89112bf48861a122df7bdd203af1cf776ae4353ae09e176eeaaeaad98f82eb2a9277fa9795954590d29c8c0ba84aca16850800ba1950b16430af7cfa81add4b225de680e9c5690418f70851295161bd3c2c91ae366531c62991a73fcd9c3b31fa8261fe29761538de5ddf38f6163c9eb57609888bd6d177f395ea12593807acd32a8d9247ab5aa8db8af62d0a28c376c7d096c02ff9799d4e5818abda986a00002548ea29cc77d2aa5cb91850528fd7f1e209686a3cab3c8f8965818d0063e4f600003e12c6dd83593e2303cca9b712b69d121b2feeb19a6d070cb06e465cce92653b0000ab18e8decb34fb0cecd0600ca124786f133401ac197d9615e4dc646ae750b2db0100c40a4788e12bac6c26e3ca1d72925f7764f72dbde8d487d657707e880849b7bf0100ed6189ae534e3d8de39f2e4046a43486f348065a85d12860c39cb85fefa6fb4c01001049602903683e070a3fcbfe160bfcf5c21094185cb9fdba4262020860b12b2800004cd9d1ed019992afdb63ba2644017ac9938ff11f665ff57279d507fe90de5be6f56d5554349f7d637ce0d0365a0cc056de61cb9a6fb10cfdfbb83d927911018b1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd2103a2e831904e261074139899446af2298307f7c5a59c2ff9f7d691b66c5f78a3e840c2e23decf2e8f845e8a8e9d18cd81d9eb4a5d0e7d53624d34c0f1cbdbb2e05697b427bd7004c14ba20370d2384a5967a8bd5921202dfa382e99c1c49db9d3182"
			  },
			  "value":100.0,
			  "valueSat":10000000000,
			  "sequence":4294967295
			},
			{
			  "anonymityGroup":1,
			  "scriptSig":{
				"asm":"OP_SIGMASPEND 2c0bb28d0e52f8f3d0a139446fe7e4da31cd195f7c3058d8e6cffea3b155dc32000092f7d2a8a1a4892c983da5a20fdd657957fcdc0ebd86485e7ac1c7aa62a92dfe010023353a1603f1d14720d48a80ad139a400862ecba29637b38f9f1162e0d556ee3010095defe584076981c1c6fc020d842a87711fec85b01aecf288282a564dcde2c2c01001597a0147c89ed6f0961dfa572909bccc58eeadaf646ce7ac29a6e0fffba2201e2450eb859d4560a7c1aaa18971cf9610f5b361c97207c97d947b808f08f8767f950b817d1e653b582638fc69da3725d78a0c0d41262288ef5c9e06a2c6412165b0818786e2d5d821f707a59226a2fa2cdd009a60b91742956d315601b8390537c636f5a0f523546f283f0860ee7c495020e2cce56c8b9814e3997a0d0ae224b17e8b464565f91a16fa722546ea707eb73f16613cd1a204491c6590652f2df7876253d3f4c315f6a2481bb70f42095dc54237e9d1cffb49072762af07ee6cdaa82daf8f85039756da027621e3b4dc67596450aeb638894c0cca4ab69a0bdd1fff7a2fb4abacf33351dd0046d0a29608be1fe6fc91f3a349b0b6940f33e1232af2099ad97a2e05442261c3f25c2e5d241203eb7f983df4714f016692748d1aea107b3a71013092143cb4f4085bbf4f23064af7250151b6f7fea5c08933fae00353dfe11966731377d839e89aef9dda76c46a2238af18a7932c247ee78bcf0400938428e1c6c368b9af88891b40f07b836a7abaa6c638a9fa6ad2f7add6f2ded013a3a838dbe5c81f4dad69c98df205dc507004471b9f7ed6ce5eefd20a58db720e2479f198654e385254e32dfbd0161f63b8a3d3c8e81b9660d78626190acdc6a43b70bcc32c8b7cf3420323c43cf415ca852eb529f931c428c8f3e7e42a70474d0690c6c24d4f5840672804b4208a1ad756772dd80e9e90379b7da72c8ec8729f75c152b1fa5705fb894f187b6d3d2c855ee9538e4c58f6d15cc39698a7075164de691658d865797faf88703cd59b51cea8b96a6027546ebfe3007a0356d14b8480f30481ac74fa635199cd87c2f597245de45b412d32e57a9d0c67f10e384da8e1b6f125ab62691788151432a7701dd31ba4b4c6d7276217f1ddcd26b42d2320d8e8b81c8c845d6333b009b29633ad2afccf37f4259ef5eef236a6b399134aee916327c44f591866e7392110597e63490fd43f743f2f227b7284d991a484a3bb307db65821fa9747af6cc277b465c5f524a650188d8421681c3a631c0e964cf9ac500004cdadf027a224a6746819329ac51c4c7a295ed9d4e75f081e8c590718e0a762c0100c448a2bba9efe5a4e1473a1ad9217bc28f7a628f64920d400b7b37097f7e7b1f00002304ea4f401f1c08e6d2dfad3e53de99062563c5232cbfb7cd97660177798df90000d315be7ae3d127a96f680db963d23c0f8759ebf0d7e1b5db573851ea7974d3d000009faa7df3c9c87ea2be077274952505173477e7c927fa5433e5a508ba2b8851a800004737d94352d548efc6479596ea413b523384f839715479295fba94928334dfdc000001bb5aa89712cb71b90006fd545ad4295ef4e0e8f0d9362f73fb53e32d1806d8102e4c4cbfdcce690e04dcad8f8354c90d6a65ac2cff03c5dd76c6f06f770ab01f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd2102d9cd799a2267321b8109204c87e638e5e040bb9a2101a0ea12e43af4ed7586754046a5fca18239564d3bed40328a7b1472e1d0941337b68b6a7d82d62d5023278b7f6e3573a22627c0a30269ddee7059968b0aa957dbd041e6d762af2fed06de3b",
				"hex":"c42c0bb28d0e52f8f3d0a139446fe7e4da31cd195f7c3058d8e6cffea3b155dc32000092f7d2a8a1a4892c983da5a20fdd657957fcdc0ebd86485e7ac1c7aa62a92dfe010023353a1603f1d14720d48a80ad139a400862ecba29637b38f9f1162e0d556ee3010095defe584076981c1c6fc020d842a87711fec85b01aecf288282a564dcde2c2c01001597a0147c89ed6f0961dfa572909bccc58eeadaf646ce7ac29a6e0fffba2201e2450eb859d4560a7c1aaa18971cf9610f5b361c97207c97d947b808f08f8767f950b817d1e653b582638fc69da3725d78a0c0d41262288ef5c9e06a2c6412165b0818786e2d5d821f707a59226a2fa2cdd009a60b91742956d315601b8390537c636f5a0f523546f283f0860ee7c495020e2cce56c8b9814e3997a0d0ae224b17e8b464565f91a16fa722546ea707eb73f16613cd1a204491c6590652f2df7876253d3f4c315f6a2481bb70f42095dc54237e9d1cffb49072762af07ee6cdaa82daf8f85039756da027621e3b4dc67596450aeb638894c0cca4ab69a0bdd1fff7a2fb4abacf33351dd0046d0a29608be1fe6fc91f3a349b0b6940f33e1232af2099ad97a2e05442261c3f25c2e5d241203eb7f983df4714f016692748d1aea107b3a71013092143cb4f4085bbf4f23064af7250151b6f7fea5c08933fae00353dfe11966731377d839e89aef9dda76c46a2238af18a7932c247ee78bcf0400938428e1c6c368b9af88891b40f07b836a7abaa6c638a9fa6ad2f7add6f2ded013a3a838dbe5c81f4dad69c98df205dc507004471b9f7ed6ce5eefd20a58db720e2479f198654e385254e32dfbd0161f63b8a3d3c8e81b9660d78626190acdc6a43b70bcc32c8b7cf3420323c43cf415ca852eb529f931c428c8f3e7e42a70474d0690c6c24d4f5840672804b4208a1ad756772dd80e9e90379b7da72c8ec8729f75c152b1fa5705fb894f187b6d3d2c855ee9538e4c58f6d15cc39698a7075164de691658d865797faf88703cd59b51cea8b96a6027546ebfe3007a0356d14b8480f30481ac74fa635199cd87c2f597245de45b412d32e57a9d0c67f10e384da8e1b6f125ab62691788151432a7701dd31ba4b4c6d7276217f1ddcd26b42d2320d8e8b81c8c845d6333b009b29633ad2afccf37f4259ef5eef236a6b399134aee916327c44f591866e7392110597e63490fd43f743f2f227b7284d991a484a3bb307db65821fa9747af6cc277b465c5f524a650188d8421681c3a631c0e964cf9ac500004cdadf027a224a6746819329ac51c4c7a295ed9d4e75f081e8c590718e0a762c0100c448a2bba9efe5a4e1473a1ad9217bc28f7a628f64920d400b7b37097f7e7b1f00002304ea4f401f1c08e6d2dfad3e53de99062563c5232cbfb7cd97660177798df90000d315be7ae3d127a96f680db963d23c0f8759ebf0d7e1b5db573851ea7974d3d000009faa7df3c9c87ea2be077274952505173477e7c927fa5433e5a508ba2b8851a800004737d94352d548efc6479596ea413b523384f839715479295fba94928334dfdc000001bb5aa89712cb71b90006fd545ad4295ef4e0e8f0d9362f73fb53e32d1806d8102e4c4cbfdcce690e04dcad8f8354c90d6a65ac2cff03c5dd76c6f06f770ab01f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd2102d9cd799a2267321b8109204c87e638e5e040bb9a2101a0ea12e43af4ed7586754046a5fca18239564d3bed40328a7b1472e1d0941337b68b6a7d82d62d5023278b7f6e3573a22627c0a30269ddee7059968b0aa957dbd041e6d762af2fed06de3b"
			  },
			  "value":100.0,
			  "valueSat":10000000000,
			  "sequence":4294967295
			},
			{
			  "anonymityGroup":1,
			  "scriptSig":{
				"asm":"OP_SIGMASPEND e9fcfb78b81f1cdfb53c62417cc1b362f52532ccdf055190787c826a84de3e9b0100779985d4b1b1c5e701a008666aec2c65400a1d0fd684cebfc8e77439d5362c16000097bfb9c79218528bf2515a6c3edb70428999afd348fb1ae6d4a053cf85b3e28c0100cd6eaeec888a2f971fceddbff518a2cad03f66d28cdd084b7f406d5fef84eb150000151ca45f20ef7e7b95cae35c6b08af8c95965b156dac5eb748dbe9db067c63ebfed427fb0a30e422bfee913ac7106877ab2838db36e992e7ac9b6cfc746e3954d172708b64a6ed0c94a460dbba986ecf0592d0a289baa3369fa7d9d8181973d92a5a6f1615a5c4a27377739fc172af3ca83c8d2bfe1f4c4163851c0cf7776773f29a4fcad864e11eb7572e13fb02dfda4613035a2df00ae21aca80c6c5e345deac538a4b4a8c5c9998f35176a68de0ef38eb25b036810d440976d556e4e6acee2772bc0c8961dd140b857ecb53a029930600267aec1f7c2358b9a874ebb15fa3a2f14afa1645921409f50a6b3d9f38e841e580b460f90c3c77cf764a77a6b65c657af753c54e27ed849a03a2394c99548a46d9719dc563a2f6e19bf909845b38c028f70ca802d96ff47eb3540ded69554df3c0e75da124c737c97d36590762f6264f2b131301e997a05b6871f3797ea7e0e59db4cdea0688681e8df62341cee745d8ad9defcfafa8338ff0b7d477ddd93b9b7b1d805decb3329334f4a7990d1766887c346a10677a54ddaf57f7f81e004a972af7281173f521d6970b145294fb9a0d83281ded291a6aa00caefc2ffe6a66d7c77d5701037ed012192bb4423c4631fc08d89f4f8d25ce3be9b88dfe051e6653a18b7af859d83bc222bf413a4a024a8023040e11485b4a70e2d0e7763d601c9532d27e02483f640c0a5c3179a12d53d8200d00cb0fe4c1cbe2edc06f36c278af77a095eb8a11c9aa7f0f53d88832464262876af5a2b3f1b4ca357e8f7a8cddd9b4d3d6a698590560eb5dfe1209056fa8758145f419175aefe73b36296c2e3d762ca03337f180a879d7edadec45682f02f36dd464a6e049d05c05f6ead47a0da6fc62673891d3d01e08adc0c88a5fc029ef342384e051e69c451202340f8e3e72f78060c645413c8653c07c0d08ed3a337f02eb92677960181d43b58e0808136a846400bef3bc34e18e5e09c3fb2b0073bca169884bed4592740c61b3d5ffb49fee0df0bca7b52024df3dcaa00f4d28077364db9b252cc06aa85f66f996d19569dd8f961fc4289ff32088ace04d75c18b00001030f2a9882d3aca1cf5a84bda8ac7c92c5bdcaf492295e8cf6080104063ca180000925eda046bbc24b27d4651dd2809de2747885f63d9ebdac1143e574462d4b69801001d7422d3ebaa9401f97c489a85086a3b48d09c7a69208ee7d7e6a7f0cb8e32cc0000d2cfb0d15de941e0025f65f0161d55685f05e4ce8cabe6f5b660800a4efb15950000490929b68bf5e1bd01adcc522b71aa949f6e1393703b25124b82d5f568a2aeea00003d2af0ca26d08f44ed80c43281aed6dab6d491fa6b2b4739c06c86855e5cb8ea010015cd819b456a851192b8caf923e76daee4db3cb38b3cc488796a151d0ff5d9a1630423e3e068c1ad988794671918f63ffda1ed627801c22a72ca7be0085d7e9f1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd210305a48450bcb57d448832a95486c294d89834523760e74f90e682500b67c9064640fed06a1a2b65d15bdb4ef65c33c804d480b0217c505d59c71b7efdc9b189f4ac2e6ce945df1c37f2241a972e8da09ff119c14e27629d6a026f2316d427860831",
				"hex":"c4e9fcfb78b81f1cdfb53c62417cc1b362f52532ccdf055190787c826a84de3e9b0100779985d4b1b1c5e701a008666aec2c65400a1d0fd684cebfc8e77439d5362c16000097bfb9c79218528bf2515a6c3edb70428999afd348fb1ae6d4a053cf85b3e28c0100cd6eaeec888a2f971fceddbff518a2cad03f66d28cdd084b7f406d5fef84eb150000151ca45f20ef7e7b95cae35c6b08af8c95965b156dac5eb748dbe9db067c63ebfed427fb0a30e422bfee913ac7106877ab2838db36e992e7ac9b6cfc746e3954d172708b64a6ed0c94a460dbba986ecf0592d0a289baa3369fa7d9d8181973d92a5a6f1615a5c4a27377739fc172af3ca83c8d2bfe1f4c4163851c0cf7776773f29a4fcad864e11eb7572e13fb02dfda4613035a2df00ae21aca80c6c5e345deac538a4b4a8c5c9998f35176a68de0ef38eb25b036810d440976d556e4e6acee2772bc0c8961dd140b857ecb53a029930600267aec1f7c2358b9a874ebb15fa3a2f14afa1645921409f50a6b3d9f38e841e580b460f90c3c77cf764a77a6b65c657af753c54e27ed849a03a2394c99548a46d9719dc563a2f6e19bf909845b38c028f70ca802d96ff47eb3540ded69554df3c0e75da124c737c97d36590762f6264f2b131301e997a05b6871f3797ea7e0e59db4cdea0688681e8df62341cee745d8ad9defcfafa8338ff0b7d477ddd93b9b7b1d805decb3329334f4a7990d1766887c346a10677a54ddaf57f7f81e004a972af7281173f521d6970b145294fb9a0d83281ded291a6aa00caefc2ffe6a66d7c77d5701037ed012192bb4423c4631fc08d89f4f8d25ce3be9b88dfe051e6653a18b7af859d83bc222bf413a4a024a8023040e11485b4a70e2d0e7763d601c9532d27e02483f640c0a5c3179a12d53d8200d00cb0fe4c1cbe2edc06f36c278af77a095eb8a11c9aa7f0f53d88832464262876af5a2b3f1b4ca357e8f7a8cddd9b4d3d6a698590560eb5dfe1209056fa8758145f419175aefe73b36296c2e3d762ca03337f180a879d7edadec45682f02f36dd464a6e049d05c05f6ead47a0da6fc62673891d3d01e08adc0c88a5fc029ef342384e051e69c451202340f8e3e72f78060c645413c8653c07c0d08ed3a337f02eb92677960181d43b58e0808136a846400bef3bc34e18e5e09c3fb2b0073bca169884bed4592740c61b3d5ffb49fee0df0bca7b52024df3dcaa00f4d28077364db9b252cc06aa85f66f996d19569dd8f961fc4289ff32088ace04d75c18b00001030f2a9882d3aca1cf5a84bda8ac7c92c5bdcaf492295e8cf6080104063ca180000925eda046bbc24b27d4651dd2809de2747885f63d9ebdac1143e574462d4b69801001d7422d3ebaa9401f97c489a85086a3b48d09c7a69208ee7d7e6a7f0cb8e32cc0000d2cfb0d15de941e0025f65f0161d55685f05e4ce8cabe6f5b660800a4efb15950000490929b68bf5e1bd01adcc522b71aa949f6e1393703b25124b82d5f568a2aeea00003d2af0ca26d08f44ed80c43281aed6dab6d491fa6b2b4739c06c86855e5cb8ea010015cd819b456a851192b8caf923e76daee4db3cb38b3cc488796a151d0ff5d9a1630423e3e068c1ad988794671918f63ffda1ed627801c22a72ca7be0085d7e9f1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd210305a48450bcb57d448832a95486c294d89834523760e74f90e682500b67c9064640fed06a1a2b65d15bdb4ef65c33c804d480b0217c505d59c71b7efdc9b189f4ac2e6ce945df1c37f2241a972e8da09ff119c14e27629d6a026f2316d427860831"
			  },
			  "value":100.0,
			  "valueSat":10000000000,
			  "sequence":4294967295
			},
			{
			  "anonymityGroup":1,
			  "scriptSig":{
				"asm":"OP_SIGMASPEND 77c1f4013508aadcb047c5d4d073df0533956739522240935e1aed3339c761a1000071c754b9b34da3ee5b83ca6a0c3b834569ba5bdd3af42207cf2348e02b6f50860000812fc9b026abfc1975a96f0b09b4f16c2a07aea16563e0b2f80d4c3bf1403df50100293dc3b5bf1168aa661bf2e94e9a56862e1225f592c38c5c11042be03e92f83601001517ad2d049e8812e5d3dfae275ca406b224b6c71ee7746f69e2ebba478c0ebbf78f1a93d9ac05744c1c250b5163939c528e8cedb57371f12527caf67a184c99cb32b67f380ed6e91da76cdba1091e912250e3a2a7d6c3adc1066bb037ade7e3b3f61a6a2ba0330a9e4dee6dde7755be5a7474cc951949ac8a21090bc77f7289d0b1fc3e9c059732fe96083a1a73081d9557a17c4e1a6ddfdc72efe694680131bd3ca8ee9360b0fbfd45f3d60f516aaa22da398d1e4368f1928fdc366120ff01ff9d50257cab74a2ca473914827b2237b20eb07fd13afd0fbdcdef467e92558f93b202860d019976ab8d226ee1bd4e92a029e60c72f952b791d8addcfb11f2b66b06219cecb229d0d74d07e57e3dd9caacc0f0e57478e313e84149f4aaef47c8bc5d1a6efc51546696a5eb34b4b8f46f140dd84764b53d273286f5f49cb85988695526de83b387fafdab97ccad5377009fa94f11c5690f7a7750785ad0cc5125f7755e7569aa9d66e92b6f902fbfea559b79a666f2c8486a4b60a69ba497fe3f45976e2f6f62b496e7bc22615d43a201485e52b7647887fbc13522192aa6089a9673ce11dde3873b0dc4fe44536d7530e5324a3eac95f3eb05b27eaa5ef72e808aa6808c5e972b7cea71ab5cdaf281d7ddf7ce9053ba62c60d10c7f61aad56ad36728a2056da78183cc19ee9d8974b4c6a7421e6c18fca89bd5608aa1744be2710fe5224ce3a1f451884c74ce3146917ec81c8a798cabd890981f74c631d25dacb79190a283fdb5cc6373c828893411b3b547ac764702bb3b4244ed4c37f470de98ca67fff063060c302d65cce3ba3b7898766959d893017963adfa8c8381a3235e3a8fd00ec4ca603dcbae1eb8041eb3f8d0819cfcc746459331e62cb512ca7a4439ebf7af619febd05170947b176ed12391a0b5f8fa4b913e8782dbd31073450558c5be1a289fc636d6522953c0ea8a6e8c1f62ce1f1f80626154c7a4ddbfb3d4a5f71b20a3da47b350d275c5c6c5a67b6e78a5bc1169a67d5caec220b13e1ec076f60364dbef9636d1643cc02b6a758e77b58bfc1f435f33df5d22d57feb31125000009374f988af7ba8879cadb00e7c35a34acb87c2d59bb2fae4d4b5387bf27fb890100defc344c95c73ed397d059bcf49c8cea57055f4b7651f656658a7d3ef1f4f8b90000570b35d424e33c4d77784b2db0c056bc6e66fa3c60dc2e2398049efb8a41165d00003e3e88eed2cdc564914637bc2c37f4f53faf2ed1023003b947cad66b2fdc151901002dbe998be1456b6bedbf57cf1107dcc67fa3efd21def15c453ac8d08526b04fa000099b04830e653d1dc6b65011a5bcb50adee726c424f4e44d54bf1fd79b83315270100412f9b1ff15bdbaa0100586183f67ce6e0e89ae3404e212da84978e8341d8d789c0bb429717bfb484b50743743d9276cbec48c32fe82d14994257af48d85f5ec1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd210310931a8798da56262c681a28d09329bbc7862a91a3688fd90a90956c188bc9ad40fdfd16a2b5cbc85335acc017bad488f829e321e7ba27f33e51ce1118cd835086542751f4b85da667bb365b3f53fa5c84827482c077c550d32079cad2965d9a5e",
				"hex":"c477c1f4013508aadcb047c5d4d073df0533956739522240935e1aed3339c761a1000071c754b9b34da3ee5b83ca6a0c3b834569ba5bdd3af42207cf2348e02b6f50860000812fc9b026abfc1975a96f0b09b4f16c2a07aea16563e0b2f80d4c3bf1403df50100293dc3b5bf1168aa661bf2e94e9a56862e1225f592c38c5c11042be03e92f83601001517ad2d049e8812e5d3dfae275ca406b224b6c71ee7746f69e2ebba478c0ebbf78f1a93d9ac05744c1c250b5163939c528e8cedb57371f12527caf67a184c99cb32b67f380ed6e91da76cdba1091e912250e3a2a7d6c3adc1066bb037ade7e3b3f61a6a2ba0330a9e4dee6dde7755be5a7474cc951949ac8a21090bc77f7289d0b1fc3e9c059732fe96083a1a73081d9557a17c4e1a6ddfdc72efe694680131bd3ca8ee9360b0fbfd45f3d60f516aaa22da398d1e4368f1928fdc366120ff01ff9d50257cab74a2ca473914827b2237b20eb07fd13afd0fbdcdef467e92558f93b202860d019976ab8d226ee1bd4e92a029e60c72f952b791d8addcfb11f2b66b06219cecb229d0d74d07e57e3dd9caacc0f0e57478e313e84149f4aaef47c8bc5d1a6efc51546696a5eb34b4b8f46f140dd84764b53d273286f5f49cb85988695526de83b387fafdab97ccad5377009fa94f11c5690f7a7750785ad0cc5125f7755e7569aa9d66e92b6f902fbfea559b79a666f2c8486a4b60a69ba497fe3f45976e2f6f62b496e7bc22615d43a201485e52b7647887fbc13522192aa6089a9673ce11dde3873b0dc4fe44536d7530e5324a3eac95f3eb05b27eaa5ef72e808aa6808c5e972b7cea71ab5cdaf281d7ddf7ce9053ba62c60d10c7f61aad56ad36728a2056da78183cc19ee9d8974b4c6a7421e6c18fca89bd5608aa1744be2710fe5224ce3a1f451884c74ce3146917ec81c8a798cabd890981f74c631d25dacb79190a283fdb5cc6373c828893411b3b547ac764702bb3b4244ed4c37f470de98ca67fff063060c302d65cce3ba3b7898766959d893017963adfa8c8381a3235e3a8fd00ec4ca603dcbae1eb8041eb3f8d0819cfcc746459331e62cb512ca7a4439ebf7af619febd05170947b176ed12391a0b5f8fa4b913e8782dbd31073450558c5be1a289fc636d6522953c0ea8a6e8c1f62ce1f1f80626154c7a4ddbfb3d4a5f71b20a3da47b350d275c5c6c5a67b6e78a5bc1169a67d5caec220b13e1ec076f60364dbef9636d1643cc02b6a758e77b58bfc1f435f33df5d22d57feb31125000009374f988af7ba8879cadb00e7c35a34acb87c2d59bb2fae4d4b5387bf27fb890100defc344c95c73ed397d059bcf49c8cea57055f4b7651f656658a7d3ef1f4f8b90000570b35d424e33c4d77784b2db0c056bc6e66fa3c60dc2e2398049efb8a41165d00003e3e88eed2cdc564914637bc2c37f4f53faf2ed1023003b947cad66b2fdc151901002dbe998be1456b6bedbf57cf1107dcc67fa3efd21def15c453ac8d08526b04fa000099b04830e653d1dc6b65011a5bcb50adee726c424f4e44d54bf1fd79b83315270100412f9b1ff15bdbaa0100586183f67ce6e0e89ae3404e212da84978e8341d8d789c0bb429717bfb484b50743743d9276cbec48c32fe82d14994257af48d85f5ec1f00000000e40b540200000080590526dd52f8c321fa49589ca1611724fc9572336b3983c8fb4e83e55863dd210310931a8798da56262c681a28d09329bbc7862a91a3688fd90a90956c188bc9ad40fdfd16a2b5cbc85335acc017bad488f829e321e7ba27f33e51ce1118cd835086542751f4b85da667bb365b3f53fa5c84827482c077c550d32079cad2965d9a5e"
			  },
			  "value":100.0,
			  "valueSat":10000000000,
			  "sequence":4294967295
			},
			{
			  "anonymityGroup":1,
			  "scriptSig":{
				"asm":"OP_SIGMASPEND 7252d557c36f45288f287d81c1f31f5d51b778f905b41b79b9ab15554a0e3b5f010036ea61b6cc59ffd61c2b7a6599c344a7e259e0f67b2a42940fef260fac7619df0000731f2649a28ec62446524273557bbd5a9377684d50288a244c1738a8587148ea00007cd2836bf9359994dc2cd9267ebc42414d8d807852c62ce2523551fb4f2f30b900001548c989ab140a3ace02c4cc4560cb7770593c3a1576262d7b3e6700098fbc5420748d3ce99f1ce057b928b280f85a93a0b08b5aa7eff9922bf79c5447d8ee6af172c99743ffdb4b5d21daf1014150c3f9014c242b3f6707d272089bfdce87c51b34e6d46bfb5d4452a7a9d16deaf15c11f44e112a885fe07770df9c959cc77ca53a6b7ca2a883fb1fb040e423a06e79bc534e21f243f92a25d59094b195f1a02bf3348105e720edd1bfbad05ed97845c9f346340c1aa06d4983d14d5a5b08f19be2dea8ac9425d0d2a69ee9918ba5eb0273287b83f97360833180805987703ab123ebc6e5f099e2a4857f34913f06cd609fa717b2d3047a148189046484b88460ae7f572a3f213f375a3d7d2177e231f0a3c0145dfa60be0cc761785d290e69d154407019e2429d69729374638c5ae06147e7499dbae31fe3a1684c901ee12b27398201a7916397b6cacb6b6972db15117288122ba01ec9c68d78bfe0e8515525f2534d8401a92faffbe44f886406f2e2ae077c779803d197de1bca5a94abdfe85862f685c7158e1fb1cdc1ce6abd7384c1443b93e9984779c8d0d7167d350fd4c1b9abe1be6ae136612ff23a4c7830df1e034f544fcbeb54c7a1d0885ebb53d0123ac0e30b93ee4f3533da7ba808b848c677bda4c86542c360dad96fb6829e2197a07c108ee339436ef26f352703d6353c803522e57beeecd79abebd7d6bab9c4e8c4d6cc92421a93f977cc353e23fca5eb91dc87958e14e69b52afb8dae1f70972456c8be70ae86c28ca1798cb09d93732559a259e858fa157b39fc022215d829aca9841f07a99f4f40f66b55d0b96f7f1cb87467ae6003c8b258cbe9af7b2d2765bd420b905f9b0e5096b750bf156526174532aa43f6e7304521d44e330488f99fb124280cf863020332bc8cea92cc71eca67855d42da5d6ce58875139d23d1317dd04085bd331a383542993fb8cd83f9e73c9becebb7c7b840f097461b0386459992d4077bfe3118f8be2e47c1539f088a97457b7013ba7d7e68f398eb55c075997c8c67d9087aa8d9c9f44c42eff98f67c4f45f9c4d7d13315aebc67fd0ebc00003363076abf579ecf927734e9bcb843037f684278c659e183c0f6235d22dab8180000d36084ae243828756d54f3b6908891e5fdd8deb6368e9d4941407dcaf019ff0e000092c21af2d0bb0c506454fb6f6cfd789649facef1238c8e6a137fe9570cd2562701007ea89f8a2c2d587a3cd445de70303e65444e4924031726690c4a8f8bba3683c400002596a772df3afc96d64955a1cbdc1e8c4b67c85d9ac010b83c2f8141f6ba63410000103b07d9089f0aee1d25631385a0d00b30f10ac7ecc991e5910afee6bc6c7fbc000082c08c24863ec43bef2fabe1ff8aa5b6acd37dc2bacfaf2e819078820b78e80fc03a7226372e5074f2b83ccf4aaa92e89225888f4e35bd79760881376f34e4a21f00000000f9029500000000e244028be6278500ede0782231c1664f6a40f5b01d82c83b21bd50248a717a0c21034bc9bc3f623ba555a6abceb0d5a491abb9d067ad76993f6b3544a21b34a2237a40e4e3733e1191bb6731897c6b691a00dbf86c94a163047b61bd4b24e6250c3f1c6828ab5a1d6266f09323e2838e304fdfb636eaee1ec3e90a535debde88e31ba1",
				"hex":"c47252d557c36f45288f287d81c1f31f5d51b778f905b41b79b9ab15554a0e3b5f010036ea61b6cc59ffd61c2b7a6599c344a7e259e0f67b2a42940fef260fac7619df0000731f2649a28ec62446524273557bbd5a9377684d50288a244c1738a8587148ea00007cd2836bf9359994dc2cd9267ebc42414d8d807852c62ce2523551fb4f2f30b900001548c989ab140a3ace02c4cc4560cb7770593c3a1576262d7b3e6700098fbc5420748d3ce99f1ce057b928b280f85a93a0b08b5aa7eff9922bf79c5447d8ee6af172c99743ffdb4b5d21daf1014150c3f9014c242b3f6707d272089bfdce87c51b34e6d46bfb5d4452a7a9d16deaf15c11f44e112a885fe07770df9c959cc77ca53a6b7ca2a883fb1fb040e423a06e79bc534e21f243f92a25d59094b195f1a02bf3348105e720edd1bfbad05ed97845c9f346340c1aa06d4983d14d5a5b08f19be2dea8ac9425d0d2a69ee9918ba5eb0273287b83f97360833180805987703ab123ebc6e5f099e2a4857f34913f06cd609fa717b2d3047a148189046484b88460ae7f572a3f213f375a3d7d2177e231f0a3c0145dfa60be0cc761785d290e69d154407019e2429d69729374638c5ae06147e7499dbae31fe3a1684c901ee12b27398201a7916397b6cacb6b6972db15117288122ba01ec9c68d78bfe0e8515525f2534d8401a92faffbe44f886406f2e2ae077c779803d197de1bca5a94abdfe85862f685c7158e1fb1cdc1ce6abd7384c1443b93e9984779c8d0d7167d350fd4c1b9abe1be6ae136612ff23a4c7830df1e034f544fcbeb54c7a1d0885ebb53d0123ac0e30b93ee4f3533da7ba808b848c677bda4c86542c360dad96fb6829e2197a07c108ee339436ef26f352703d6353c803522e57beeecd79abebd7d6bab9c4e8c4d6cc92421a93f977cc353e23fca5eb91dc87958e14e69b52afb8dae1f70972456c8be70ae86c28ca1798cb09d93732559a259e858fa157b39fc022215d829aca9841f07a99f4f40f66b55d0b96f7f1cb87467ae6003c8b258cbe9af7b2d2765bd420b905f9b0e5096b750bf156526174532aa43f6e7304521d44e330488f99fb124280cf863020332bc8cea92cc71eca67855d42da5d6ce58875139d23d1317dd04085bd331a383542993fb8cd83f9e73c9becebb7c7b840f097461b0386459992d4077bfe3118f8be2e47c1539f088a97457b7013ba7d7e68f398eb55c075997c8c67d9087aa8d9c9f44c42eff98f67c4f45f9c4d7d13315aebc67fd0ebc00003363076abf579ecf927734e9bcb843037f684278c659e183c0f6235d22dab8180000d36084ae243828756d54f3b6908891e5fdd8deb6368e9d4941407dcaf019ff0e000092c21af2d0bb0c506454fb6f6cfd789649facef1238c8e6a137fe9570cd2562701007ea89f8a2c2d587a3cd445de70303e65444e4924031726690c4a8f8bba3683c400002596a772df3afc96d64955a1cbdc1e8c4b67c85d9ac010b83c2f8141f6ba63410000103b07d9089f0aee1d25631385a0d00b30f10ac7ecc991e5910afee6bc6c7fbc000082c08c24863ec43bef2fabe1ff8aa5b6acd37dc2bacfaf2e819078820b78e80fc03a7226372e5074f2b83ccf4aaa92e89225888f4e35bd79760881376f34e4a21f00000000f9029500000000e244028be6278500ede0782231c1664f6a40f5b01d82c83b21bd50248a717a0c21034bc9bc3f623ba555a6abceb0d5a491abb9d067ad76993f6b3544a21b34a2237a40e4e3733e1191bb6731897c6b691a00dbf86c94a163047b61bd4b24e6250c3f1c6828ab5a1d6266f09323e2838e304fdfb636eaee1ec3e90a535debde88e31ba1"
			  },
			  "value":25.0,
			  "valueSat":2500000000,
			  "sequence":4294967295
			},
			{
			  "anonymityGroup":1,
			  "scriptSig":{
				"asm":"OP_SIGMASPEND 51f9b4d6931827686e1c5ed60d09fbd7050c093e7424aaab9c84e3d11ec3349d00005a582a32d0fedb32b3f7fa0ef118bf7325a95fd46ef846a89c155fc6ac4271130000c6b4a8a988c610dda27c92102db5575f4b6f42d8f32d8ec253b25427f784a2490100f5e5e8e590818e013b67f5c52b5cebf59cf73b2a03912bcd0f94d3c4380be27d000015e48f758a84da02e0a57fdae68e0341bfda81dc42461ed7789cc05c2a47f2c1750b980e17e873bc8ef4b9b07e05118f4d9fdfff6577e65540b9a2a1bd687e6e8ff0364281cbfc246f7700f7ee22eefc3525ccb2e77f64d63a92273649ffb0072430b4cfdd301aa900271f3da4375165fcd6a86445a20b93846fc0642bc6c268e8f9b3bd40d6768cdf11aa6cfe07c6c1c5cdc01edffa97f36b0130b4d359f33572cfaedbd7fd310d30bab86e429e8aecf8d196fc2715c12548962c8ed14ef097d5c92092d6727a0a5c12d64bc60c65764cd7463c7c5e40f35cbcfd8848e33b4e23c2c9dbf8378954a54361e8ad3c133ca4cdf8ecc519d6c45e4a0bd7bf9b99303478f9ae9c66de8a412f06011d34f536687940e9c547c89be9adce8985c395f4c72b1141d2ae124f1413ad0d067e572a3d711ab7eead482154ce01eb36cf9d8f62b60c75920df93b8921b14fe5b870a06829976e246b2db5fcd104574261dae094b86830b697ced626000abcb58ae6d4bd3dea0717fe6dd7cddec1c67ae4c0e246cf150d4ad5922f91d40f5b02b27376d46e8b9380f0d0b58d7b7eeea1a21e722bfd3295ca469e1f256eb97e9716a0371a4dd4cf0dd0e2ec38692a228b4378fc9a41b449a2b9d6c32cd47918bd47fc24be8566e43227f7ed1f870110bef20443ccb434e46cc3b6d8feee8e31b8da2cdccdcd74bd656cd94d59c26fabed0b85e6f15cacd09ae91b64fb8fc061e0ebe4dc4d270962120caa24e5c84290d994fe2410840149b4921d41b76f837d1a29da04ecaeb01a695df8e633f91105af7f6fe5a26a05fee03a19c5033453c11a710cfc84ce413457be9775c1957533fdfd817f2a99590d10ddceec7fded39a09ef9e9ef08d8f38d15dccee88f219e15301fa6eac574f9ddbc812f5a0a3a629750e48a3e868a90f39e63755e1b46c4eb5c94f4059572f0c05fae3e9d0e3271f0989f013fc5184c236f8c9b9ab05146683156c9a2b11bb0f7a0da3627c0bac58c194498a673801870a3958d3a0582a5ccbeb9ca554071ea446325603e67d876eeb1a05e1342649309ff58f618ea0a21f0cb02a766e3c01003eb745c4933d0f52ee02aaef2a16cb6c52bd164928673edb0f51c7214fa7bf2b010064c7d3b6a7aa59a310eda829e7ac9075ff1e96e01ea1719ee7fc4a5bdb0e9b9b01002763566e427b69e5646cf25048eda2e5806dcc6b7a5e5cdb59702d556840bf7201003c2ee321c25bfdea2ec3df0886c8b165dbf5b6902de7c39682e872ec9cd7b2a500001b3c159cef5dae965c86db605792ace3d987c9559a1be5388acaecb2c753da8f010075cf5f030c222e7dbd81b07096a20a1dc1958211f5327d60e7a499ae7c549188010014de0bbc9bf0798fbbf3255270b7b93ab1e70d76761fb74226653c47b3fd5e6c9f2d757029c6f2a8aa2e73bbebe51287c5a34d6f129a69e0ef7bbd2c13f477e51f00000000f9029500000000e244028be6278500ede0782231c1664f6a40f5b01d82c83b21bd50248a717a0c2102eb2aa581203121be475e55e67299a1aa8715a7602c538421240a2b91b5475e624018b406db4445bf718f2ddb175f83c73f6a1a17dbeefcfaf281fac6750a090a0e7a5ee40dd360279af0f85056341de416b426b79b48bea5bad6b06a2be6277703",
				"hex":"c451f9b4d6931827686e1c5ed60d09fbd7050c093e7424aaab9c84e3d11ec3349d00005a582a32d0fedb32b3f7fa0ef118bf7325a95fd46ef846a89c155fc6ac4271130000c6b4a8a988c610dda27c92102db5575f4b6f42d8f32d8ec253b25427f784a2490100f5e5e8e590818e013b67f5c52b5cebf59cf73b2a03912bcd0f94d3c4380be27d000015e48f758a84da02e0a57fdae68e0341bfda81dc42461ed7789cc05c2a47f2c1750b980e17e873bc8ef4b9b07e05118f4d9fdfff6577e65540b9a2a1bd687e6e8ff0364281cbfc246f7700f7ee22eefc3525ccb2e77f64d63a92273649ffb0072430b4cfdd301aa900271f3da4375165fcd6a86445a20b93846fc0642bc6c268e8f9b3bd40d6768cdf11aa6cfe07c6c1c5cdc01edffa97f36b0130b4d359f33572cfaedbd7fd310d30bab86e429e8aecf8d196fc2715c12548962c8ed14ef097d5c92092d6727a0a5c12d64bc60c65764cd7463c7c5e40f35cbcfd8848e33b4e23c2c9dbf8378954a54361e8ad3c133ca4cdf8ecc519d6c45e4a0bd7bf9b99303478f9ae9c66de8a412f06011d34f536687940e9c547c89be9adce8985c395f4c72b1141d2ae124f1413ad0d067e572a3d711ab7eead482154ce01eb36cf9d8f62b60c75920df93b8921b14fe5b870a06829976e246b2db5fcd104574261dae094b86830b697ced626000abcb58ae6d4bd3dea0717fe6dd7cddec1c67ae4c0e246cf150d4ad5922f91d40f5b02b27376d46e8b9380f0d0b58d7b7eeea1a21e722bfd3295ca469e1f256eb97e9716a0371a4dd4cf0dd0e2ec38692a228b4378fc9a41b449a2b9d6c32cd47918bd47fc24be8566e43227f7ed1f870110bef20443ccb434e46cc3b6d8feee8e31b8da2cdccdcd74bd656cd94d59c26fabed0b85e6f15cacd09ae91b64fb8fc061e0ebe4dc4d270962120caa24e5c84290d994fe2410840149b4921d41b76f837d1a29da04ecaeb01a695df8e633f91105af7f6fe5a26a05fee03a19c5033453c11a710cfc84ce413457be9775c1957533fdfd817f2a99590d10ddceec7fded39a09ef9e9ef08d8f38d15dccee88f219e15301fa6eac574f9ddbc812f5a0a3a629750e48a3e868a90f39e63755e1b46c4eb5c94f4059572f0c05fae3e9d0e3271f0989f013fc5184c236f8c9b9ab05146683156c9a2b11bb0f7a0da3627c0bac58c194498a673801870a3958d3a0582a5ccbeb9ca554071ea446325603e67d876eeb1a05e1342649309ff58f618ea0a21f0cb02a766e3c01003eb745c4933d0f52ee02aaef2a16cb6c52bd164928673edb0f51c7214fa7bf2b010064c7d3b6a7aa59a310eda829e7ac9075ff1e96e01ea1719ee7fc4a5bdb0e9b9b01002763566e427b69e5646cf25048eda2e5806dcc6b7a5e5cdb59702d556840bf7201003c2ee321c25bfdea2ec3df0886c8b165dbf5b6902de7c39682e872ec9cd7b2a500001b3c159cef5dae965c86db605792ace3d987c9559a1be5388acaecb2c753da8f010075cf5f030c222e7dbd81b07096a20a1dc1958211f5327d60e7a499ae7c549188010014de0bbc9bf0798fbbf3255270b7b93ab1e70d76761fb74226653c47b3fd5e6c9f2d757029c6f2a8aa2e73bbebe51287c5a34d6f129a69e0ef7bbd2c13f477e51f00000000f9029500000000e244028be6278500ede0782231c1664f6a40f5b01d82c83b21bd50248a717a0c2102eb2aa581203121be475e55e67299a1aa8715a7602c538421240a2b91b5475e624018b406db4445bf718f2ddb175f83c73f6a1a17dbeefcfaf281fac6750a090a0e7a5ee40dd360279af0f85056341de416b426b79b48bea5bad6b06a2be6277703"
			  },
			  "value":25.0,
			  "valueSat":2500000000,
			  "sequence":4294967295
			}
		  ],
		  "vout":[
			{
			  "value":449.9983556,
			  "n":0,
			  "scriptPubKey":{
				"asm":"OP_DUP OP_HASH160 d8ad1e3e2a57c3fe88d241337b0cbabed117bd6d OP_EQUALVERIFY OP_CHECKSIG",
				"hex":"76a914d8ad1e3e2a57c3fe88d241337b0cbabed117bd6d88ac",
				"reqSigs":1,
				"type":"pubkeyhash",
				"addresses":[
				  "aLU95oKBFMSc7RWwtsGEBuK3ihFkcgKbfL"
				]
			  },
			  "spentTxId":"7a3f13c9e5088f64abbff0bfd8b95886b17659018972f643e3ee25c69b852928",
			  "spentIndex":0,
			  "spentHeight":328253
			}
		  ],
		  "blockhash":"c06c3a20c46614e13d0c6b449b4abc42ebfab26172fafc4e00105dd68dce9305",
		  "height":327896,
		  "confirmations":13521,
		  "time":1607820997,
		  "blocktime":1607820997
		}"#;

        let _tx: Transaction = serde_json::from_str(json_str).unwrap();
    }
}
