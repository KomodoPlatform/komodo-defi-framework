use crate::lightning::ln_platform::h256_json_from_txid;
use crate::H256Json;
use lightning::chain::channelmonitor::Balance;
use lightning::ln::channelmanager::ChannelDetails;
use secp256k1v22::PublicKey;
use serde::{de, Serialize, Serializer};
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::str::FromStr;

// TODO: support connection to onion addresses
#[derive(Debug, PartialEq)]
pub struct NodeAddress {
    pub pubkey: PublicKey,
    pub addr: SocketAddr,
}

impl Serialize for NodeAddress {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{}@{}", self.pubkey, self.addr))
    }
}

impl<'de> de::Deserialize<'de> for NodeAddress {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NodeAddressVisitor;

        impl<'de> de::Visitor<'de> for NodeAddressVisitor {
            type Value = NodeAddress;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result { write!(formatter, "pubkey@host:port") }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let mut pubkey_and_addr = v.split('@');
                let pubkey_str = pubkey_and_addr.next().ok_or_else(|| {
                    let err = format!("Could not parse node address from str {}", v);
                    de::Error::custom(err)
                })?;
                let addr_str = pubkey_and_addr.next().ok_or_else(|| {
                    let err = format!("Could not parse node address from str {}", v);
                    de::Error::custom(err)
                })?;
                let pubkey = PublicKey::from_str(pubkey_str).map_err(|e| {
                    let err = format!("Could not parse node pubkey from str {}, err {}", pubkey_str, e);
                    de::Error::custom(err)
                })?;
                let addr = addr_str
                    .to_socket_addrs()
                    .map(|mut r| r.next())
                    .map_err(|e| {
                        let err = format!("Could not parse socket address from str {}, err {}", addr_str, e);
                        de::Error::custom(err)
                    })?
                    .ok_or_else(|| {
                        let err = format!("Could not parse socket address from str {}", addr_str);
                        de::Error::custom(err)
                    })?;
                Ok(NodeAddress { pubkey, addr })
            }
        }

        deserializer.deserialize_str(NodeAddressVisitor)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublicKeyForRPC(pub PublicKey);

impl From<PublicKeyForRPC> for PublicKey {
    fn from(p: PublicKeyForRPC) -> Self { p.0 }
}

impl Serialize for PublicKeyForRPC {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> de::Deserialize<'de> for PublicKeyForRPC {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PublicKeyForRPCVisitor;

        impl<'de> de::Visitor<'de> for PublicKeyForRPCVisitor {
            type Value = PublicKeyForRPC;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result { write!(formatter, "a public key") }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let pubkey = PublicKey::from_str(v).map_err(|e| {
                    let err = format!("Could not parse public key from str {}, err {}", v, e);
                    de::Error::custom(err)
                })?;
                Ok(PublicKeyForRPC(pubkey))
            }
        }

        deserializer.deserialize_str(PublicKeyForRPCVisitor)
    }
}

#[derive(Clone, Serialize)]
pub struct ChannelDetailsForRPC {
    pub rpc_channel_id: u64,
    pub channel_id: H256Json,
    pub counterparty_node_id: PublicKeyForRPC,
    pub funding_tx: Option<H256Json>,
    pub funding_tx_output_index: Option<u16>,
    pub funding_tx_value_sats: u64,
    /// True if the channel was initiated (and thus funded) by us.
    pub is_outbound: bool,
    pub balance_msat: u64,
    pub outbound_capacity_msat: u64,
    pub inbound_capacity_msat: u64,
    // Channel is confirmed onchain, this means that funding_locked messages have been exchanged,
    // the channel is not currently being shut down, and the required confirmation count has been reached.
    pub is_ready: bool,
    // Channel is confirmed and channel_ready messages have been exchanged, the peer is connected,
    // and the channel is not currently negotiating a shutdown.
    pub is_usable: bool,
    // A publicly-announced channel.
    pub is_public: bool,
}

impl From<ChannelDetails> for ChannelDetailsForRPC {
    fn from(details: ChannelDetails) -> ChannelDetailsForRPC {
        ChannelDetailsForRPC {
            rpc_channel_id: details.user_channel_id,
            channel_id: details.channel_id.into(),
            counterparty_node_id: PublicKeyForRPC(details.counterparty.node_id),
            funding_tx: details.funding_txo.map(|tx| h256_json_from_txid(tx.txid)),
            funding_tx_output_index: details.funding_txo.map(|tx| tx.index),
            funding_tx_value_sats: details.channel_value_satoshis,
            is_outbound: details.is_outbound,
            balance_msat: details.balance_msat,
            outbound_capacity_msat: details.outbound_capacity_msat,
            inbound_capacity_msat: details.inbound_capacity_msat,
            is_ready: details.is_channel_ready,
            is_usable: details.is_usable,
            is_public: details.is_public,
        }
    }
}

/// Details about the balance(s) available for spending once the channel appears on chain.
#[derive(Serialize)]
pub enum ClaimableBalance {
    /// The channel is not yet closed (or the commitment or closing transaction has not yet
    /// appeared in a block). The given balance is claimable (less on-chain fees) if the channel is
    /// force-closed now.
    ClaimableOnChannelClose {
        /// The amount available to claim, in satoshis, excluding the on-chain fees which will be
        /// required to do so.
        claimable_amount_satoshis: u64,
    },
    /// The channel has been closed, and the given balance is ours but awaiting confirmations until
    /// we consider it spendable.
    ClaimableAwaitingConfirmations {
        /// The amount available to claim, in satoshis, possibly excluding the on-chain fees which
        /// were spent in broadcasting the transaction.
        claimable_amount_satoshis: u64,
        /// The height at which an [`Event::SpendableOutputs`] event will be generated for this
        /// amount.
        confirmation_height: u32,
    },
    /// The channel has been closed, and the given balance should be ours but awaiting spending
    /// transaction confirmation. If the spending transaction does not confirm in time, it is
    /// possible our counterparty can take the funds by broadcasting an HTLC timeout on-chain.
    ///
    /// Once the spending transaction confirms, before it has reached enough confirmations to be
    /// considered safe from chain reorganizations, the balance will instead be provided via
    /// [`Balance::ClaimableAwaitingConfirmations`].
    ContentiousClaimable {
        /// The amount available to claim, in satoshis, excluding the on-chain fees which will be
        /// required to do so.
        claimable_amount_satoshis: u64,
        /// The height at which the counterparty may be able to claim the balance if we have not
        /// done so.
        timeout_height: u32,
    },
    /// HTLCs which we sent to our counterparty which are claimable after a timeout (less on-chain
    /// fees) if the counterparty does not know the preimage for the HTLCs. These are somewhat
    /// likely to be claimed by our counterparty before we do.
    MaybeClaimableHTLCAwaitingTimeout {
        /// The amount available to claim, in satoshis, excluding the on-chain fees which will be
        /// required to do so.
        claimable_amount_satoshis: u64,
        /// The height at which we will be able to claim the balance if our counterparty has not
        /// done so.
        claimable_height: u32,
    },
}

impl From<Balance> for ClaimableBalance {
    fn from(balance: Balance) -> Self {
        match balance {
            Balance::ClaimableOnChannelClose {
                claimable_amount_satoshis,
            } => ClaimableBalance::ClaimableOnChannelClose {
                claimable_amount_satoshis,
            },
            Balance::ClaimableAwaitingConfirmations {
                claimable_amount_satoshis,
                confirmation_height,
            } => ClaimableBalance::ClaimableAwaitingConfirmations {
                claimable_amount_satoshis,
                confirmation_height,
            },
            Balance::ContentiousClaimable {
                claimable_amount_satoshis,
                timeout_height,
            } => ClaimableBalance::ContentiousClaimable {
                claimable_amount_satoshis,
                timeout_height,
            },
            Balance::MaybeClaimableHTLCAwaitingTimeout {
                claimable_amount_satoshis,
                claimable_height,
            } => ClaimableBalance::MaybeClaimableHTLCAwaitingTimeout {
                claimable_amount_satoshis,
                claimable_height,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json as json;

    #[test]
    fn test_node_address_serialize() {
        let node_address = NodeAddress {
            pubkey: PublicKey::from_str("038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9").unwrap(),
            addr: SocketAddr::new("203.132.94.196".parse().unwrap(), 9735),
        };
        let expected = r#""038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9@203.132.94.196:9735""#;
        let actual = json::to_string(&node_address).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_node_address_deserialize() {
        let node_address =
            r#""038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9@203.132.94.196:9735""#;
        let expected = NodeAddress {
            pubkey: PublicKey::from_str("038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9").unwrap(),
            addr: SocketAddr::new("203.132.94.196".parse().unwrap(), 9735),
        };
        let actual: NodeAddress = json::from_str(node_address).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_public_key_for_rpc_serialize() {
        let public_key_for_rpc = PublicKeyForRPC(
            PublicKey::from_str("038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9").unwrap(),
        );
        let expected = r#""038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9""#;
        let actual = json::to_string(&public_key_for_rpc).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_public_key_for_rpc_deserialize() {
        let public_key_for_rpc = r#""038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9""#;
        let expected = PublicKeyForRPC(
            PublicKey::from_str("038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9").unwrap(),
        );
        let actual = json::from_str(public_key_for_rpc).unwrap();
        assert_eq!(expected, actual);
    }
}
