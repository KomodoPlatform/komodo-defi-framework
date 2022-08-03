use lightning_invoice::Invoice;
use secp256k1v22::PublicKey;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json as json;

    #[test]
    fn test_invoice_for_rpc_serialize() {
        let invoice_for_rpc = InvoiceForRPC(str::parse::<Invoice>("lntb20u1p3zqmvrpp52hej7trefx6y633aujj6nltjs8cf7lzyp78tfn5y5wpa3udk5tvqdp8xys9xcmpd3sjqsmgd9czq3njv9c8qatrvd5kumcxqrrsscqp79qy9qsqsp5ccy2qgmptg8dthxsjvw2c43uyvqkg6cqey3jpks4xf0tv7xfrqrq3xfnuffau2h2k8defphv2xsktzn2qj5n2l8d9l9zx64fg6jcmdg9kmpevneyyhfnzrpspqdrky8u7l4c6qdnquh8lnevswwrtcd9ypcq89ga09").unwrap());
        let expected = r#""lntb20u1p3zqmvrpp52hej7trefx6y633aujj6nltjs8cf7lzyp78tfn5y5wpa3udk5tvqdp8xys9xcmpd3sjqsmgd9czq3njv9c8qatrvd5kumcxqrrsscqp79qy9qsqsp5ccy2qgmptg8dthxsjvw2c43uyvqkg6cqey3jpks4xf0tv7xfrqrq3xfnuffau2h2k8defphv2xsktzn2qj5n2l8d9l9zx64fg6jcmdg9kmpevneyyhfnzrpspqdrky8u7l4c6qdnquh8lnevswwrtcd9ypcq89ga09""#;
        let actual = json::to_string(&invoice_for_rpc).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_invoice_for_rpc_deserialize() {
        let invoice_for_rpc = r#""lntb20u1p3zqmvrpp52hej7trefx6y633aujj6nltjs8cf7lzyp78tfn5y5wpa3udk5tvqdp8xys9xcmpd3sjqsmgd9czq3njv9c8qatrvd5kumcxqrrsscqp79qy9qsqsp5ccy2qgmptg8dthxsjvw2c43uyvqkg6cqey3jpks4xf0tv7xfrqrq3xfnuffau2h2k8defphv2xsktzn2qj5n2l8d9l9zx64fg6jcmdg9kmpevneyyhfnzrpspqdrky8u7l4c6qdnquh8lnevswwrtcd9ypcq89ga09""#;
        let expected = InvoiceForRPC(str::parse::<Invoice>("lntb20u1p3zqmvrpp52hej7trefx6y633aujj6nltjs8cf7lzyp78tfn5y5wpa3udk5tvqdp8xys9xcmpd3sjqsmgd9czq3njv9c8qatrvd5kumcxqrrsscqp79qy9qsqsp5ccy2qgmptg8dthxsjvw2c43uyvqkg6cqey3jpks4xf0tv7xfrqrq3xfnuffau2h2k8defphv2xsktzn2qj5n2l8d9l9zx64fg6jcmdg9kmpevneyyhfnzrpspqdrky8u7l4c6qdnquh8lnevswwrtcd9ypcq89ga09").unwrap());
        let actual = json::from_str(invoice_for_rpc).unwrap();
        assert_eq!(expected, actual);
    }

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
