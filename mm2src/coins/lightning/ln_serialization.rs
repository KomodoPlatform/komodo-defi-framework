use lightning_invoice::Invoice;
use secp256k1::PublicKey;
use serde::{de, Serialize, Serializer};
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq)]
pub struct InvoiceForRPC(Invoice);

impl From<Invoice> for InvoiceForRPC {
    fn from(i: Invoice) -> Self { InvoiceForRPC(i) }
}

impl From<InvoiceForRPC> for Invoice {
    fn from(i: InvoiceForRPC) -> Self { i.0 }
}

impl Serialize for InvoiceForRPC {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> de::Deserialize<'de> for InvoiceForRPC {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct InvoiceForRPCVisitor;

        impl<'de> de::Visitor<'de> for InvoiceForRPCVisitor {
            type Value = InvoiceForRPC;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a lightning invoice")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let invoice = Invoice::from_str(v).map_err(|e| {
                    let err = format!("Could not parse lightning invoice from str {}, err {}", v, e);
                    de::Error::custom(err)
                })?;
                Ok(InvoiceForRPC(invoice))
            }
        }

        deserializer.deserialize_str(InvoiceForRPCVisitor)
    }
}

// TODO: support connection to onion addresses
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
pub struct PublicKeyForRPC(PublicKey);

impl From<PublicKeyForRPC> for PublicKey {
    fn from(p: PublicKeyForRPC) -> Self { p.0 }
}

impl Serialize for PublicKeyForRPC {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0.serialize())
    }
}

impl<'de> de::Deserialize<'de> for PublicKeyForRPC {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let slice: &[u8] = de::Deserialize::deserialize(deserializer)?;
        let pubkey =
            PublicKey::from_slice(slice).map_err(|e| de::Error::custom(format!("Error {} parsing pubkey", e)))?;

        Ok(PublicKeyForRPC(pubkey))
    }
}
