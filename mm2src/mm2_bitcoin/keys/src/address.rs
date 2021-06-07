//! `AddressHash` with network identifier and format type
//!
//! A Bitcoin address, or simply address, is an identifier of 26-35 alphanumeric characters, beginning with the number 1
//! or 3, that represents a possible destination for a bitcoin payment.
//!
//! https://en.bitcoin.it/wiki/Address

use base58::{FromBase58, ToBase58};
use crypto::{checksum, dgroestl512, dhash256, keccak256, ChecksumType};
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use {AddressHash, CashAddrType, CashAddress, DisplayLayout, Error};

/// There are two address formats currently in use.
/// https://bitcoin.org/en/developer-reference#address-conversion
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Type {
    /// Pay to PubKey Hash
    /// Common P2PKH which begin with the number 1, eg: 1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2.
    /// https://bitcoin.org/en/glossary/p2pkh-address
    P2PKH,
    /// Pay to Script Hash
    /// Newer P2SH type starting with the number 3, eg: 3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy.
    /// https://bitcoin.org/en/glossary/p2sh-address
    P2SH,
}

/// `AddressHash` with prefix and t addr zcash prefix
#[derive(Debug, PartialEq, Clone)]
pub struct Address {
    /// The prefix of the address.
    pub prefix: u8,
    /// T addr prefix, additional prefix used by Zcash and some forks
    pub t_addr_prefix: u8,
    /// Public key hash.
    pub hash: AddressHash,
    /// Checksum type
    pub checksum_type: ChecksumType,
}

pub fn detect_checksum(data: &[u8], checksum: &[u8]) -> Result<ChecksumType, Error> {
    if checksum == &dhash256(data)[0..4] {
        return Ok(ChecksumType::DSHA256);
    }

    if checksum == &dgroestl512(data)[0..4] {
        return Ok(ChecksumType::DGROESTL512);
    }

    if checksum == &keccak256(data)[0..4] {
        return Ok(ChecksumType::KECCAK256);
    }
    Err(Error::InvalidChecksum)
}

pub struct AddressDisplayLayout(Vec<u8>);

impl Deref for AddressDisplayLayout {
    type Target = [u8];

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl DisplayLayout for Address {
    type Target = AddressDisplayLayout;

    fn layout(&self) -> Self::Target {
        let mut result = vec![];

        if self.t_addr_prefix > 0 {
            result.push(self.t_addr_prefix);
        }

        result.push(self.prefix);
        result.extend_from_slice(&*self.hash);
        let cs = checksum(&result, &self.checksum_type);
        result.extend_from_slice(&*cs);

        AddressDisplayLayout(result)
    }

    fn from_layout(data: &[u8]) -> Result<Self, Error>
    where
        Self: Sized,
    {
        match data.len() {
            25 => {
                let sum_type = detect_checksum(&data[0..21], &data[21..])?;

                let mut hash = AddressHash::default();
                hash.copy_from_slice(&data[1..21]);

                let address = Address {
                    t_addr_prefix: 0,
                    prefix: data[0],
                    hash,
                    checksum_type: sum_type,
                };

                Ok(address)
            },
            26 => {
                let sum_type = detect_checksum(&data[0..22], &data[22..])?;

                let mut hash = AddressHash::default();
                hash.copy_from_slice(&data[2..22]);

                let address = Address {
                    t_addr_prefix: data[0],
                    prefix: data[1],
                    hash,
                    checksum_type: sum_type,
                };

                Ok(address)
            },
            _ => Err(Error::InvalidAddress),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.layout().to_base58().fmt(f) }
}

impl FromStr for Address {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let hex = s.from_base58().map_err(|_| Error::InvalidAddress)?;
        Address::from_layout(&hex)
    }
}

impl From<&'static str> for Address {
    fn from(s: &'static str) -> Self { s.parse().unwrap() }
}

impl Address {
    pub fn from_cashaddress(
        cashaddr: &str,
        checksum_type: ChecksumType,
        p2pkh_prefix: u8,
        p2sh_prefix: u8,
    ) -> Result<Address, String> {
        let address = CashAddress::decode(cashaddr)?;

        if address.hash.len() != 20 {
            return Err("Expect 20 bytes long hash".into());
        }

        let mut hash: AddressHash = Default::default();
        hash.copy_from_slice(address.hash.as_slice());

        let prefix = match address.address_type {
            CashAddrType::P2PKH => p2pkh_prefix,
            CashAddrType::P2SH => p2sh_prefix,
        };

        // Simple UTXO hash specific
        let t_addr_prefix = 0;

        Ok(Address {
            prefix,
            t_addr_prefix,
            hash,
            checksum_type,
        })
    }

    pub fn to_cashaddress(
        &self,
        network_prefix: &str,
        p2pkh_prefix: u8,
        p2sh_prefix: u8,
    ) -> Result<CashAddress, String> {
        let address_type = if self.prefix == p2pkh_prefix {
            CashAddrType::P2PKH
        } else if self.prefix == p2sh_prefix {
            CashAddrType::P2SH
        } else {
            return Err(format!(
                "Unknown address prefix {}. Expect: {}, {}",
                self.prefix, p2pkh_prefix, p2sh_prefix
            ));
        };

        CashAddress::new(network_prefix, self.hash.clone().take().to_vec(), address_type)
    }
}

#[cfg(test)]
mod tests {
    use super::{Address, ChecksumType};

    #[test]
    fn test_address_to_string() {
        let address = Address {
            prefix: 0,
            t_addr_prefix: 0,
            hash: "3f4aa1fedf1f54eeb03b759deadb36676b184911".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!("16meyfSoQV6twkAAxPe51RtMVz7PGRmWna".to_owned(), address.to_string());
    }

    #[test]
    fn test_komodo_address_to_string() {
        let address = Address {
            prefix: 60,
            t_addr_prefix: 0,
            hash: "05aab5342166f8594baf17a7d9bef5d567443327".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!("R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW".to_owned(), address.to_string());
    }

    #[test]
    fn test_zec_t_address_to_string() {
        let address = Address {
            t_addr_prefix: 29,
            prefix: 37,
            hash: "05aab5342166f8594baf17a7d9bef5d567443327".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!("tmAEKD7psc1ajK76QMGEW8WGQSBBHf9SqCp".to_owned(), address.to_string());
    }

    #[test]
    fn test_komodo_p2sh_address_to_string() {
        let address = Address {
            prefix: 85,
            t_addr_prefix: 0,
            hash: "ca0c3786c96ff7dacd40fdb0f7c196528df35f85".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!("bX9bppqdGvmCCAujd76Tq76zs1suuPnB9A".to_owned(), address.to_string());
    }

    #[test]
    fn test_address_from_str() {
        let address = Address {
            prefix: 0,
            t_addr_prefix: 0,
            hash: "3f4aa1fedf1f54eeb03b759deadb36676b184911".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!(address, "16meyfSoQV6twkAAxPe51RtMVz7PGRmWna".into());
        assert_eq!(address.to_string(), "16meyfSoQV6twkAAxPe51RtMVz7PGRmWna".to_owned());
    }

    #[test]
    fn test_komodo_address_from_str() {
        let address = Address {
            prefix: 60,
            t_addr_prefix: 0,
            hash: "05aab5342166f8594baf17a7d9bef5d567443327".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!(address, "R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW".into());
        assert_eq!(address.to_string(), "R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW".to_owned());
    }

    #[test]
    fn test_zec_address_from_str() {
        let address = Address {
            t_addr_prefix: 29,
            prefix: 37,
            hash: "05aab5342166f8594baf17a7d9bef5d567443327".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!(address, "tmAEKD7psc1ajK76QMGEW8WGQSBBHf9SqCp".into());
        assert_eq!(address.to_string(), "tmAEKD7psc1ajK76QMGEW8WGQSBBHf9SqCp".to_owned());
    }

    #[test]
    fn test_komodo_p2sh_address_from_str() {
        let address = Address {
            prefix: 85,
            t_addr_prefix: 0,
            hash: "ca0c3786c96ff7dacd40fdb0f7c196528df35f85".into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!(address, "bX9bppqdGvmCCAujd76Tq76zs1suuPnB9A".into());
        assert_eq!(address.to_string(), "bX9bppqdGvmCCAujd76Tq76zs1suuPnB9A".to_owned());
    }

    #[test]
    fn test_grs_addr_from_str() {
        let address = Address {
            prefix: 36,
            t_addr_prefix: 0,
            hash: "c3f710deb7320b0efa6edb14e3ebeeb9155fa90d".into(),
            checksum_type: ChecksumType::DGROESTL512,
        };

        assert_eq!(address, "Fo2tBkpzaWQgtjFUkemsYnKyfvd2i8yTki".into());
        assert_eq!(address.to_string(), "Fo2tBkpzaWQgtjFUkemsYnKyfvd2i8yTki".to_owned());
    }

    #[test]
    fn test_smart_addr_from_str() {
        let address = Address {
            prefix: 63,
            t_addr_prefix: 0,
            hash: "56bb05aa20f5a80cf84e90e5dab05be331333e27".into(),
            checksum_type: ChecksumType::KECCAK256,
        };

        assert_eq!(address, "SVCbBs6FvPYxJrYoJc4TdCe47QNCgmTabv".into());
        assert_eq!(address.to_string(), "SVCbBs6FvPYxJrYoJc4TdCe47QNCgmTabv".to_owned());
    }

    #[test]
    fn test_from_to_cashaddress() {
        let cashaddresses = vec![
            "bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55",
            "bitcoincash:qr6m7j9njldwwzlg9v7v53unlr4jkmx6eylep8ekg2",
            "bitcoincash:pq4ql3ph6738xuv2cycduvkpu4rdwqge5q2uxdfg6f",
        ];
        let expected = vec![
            "1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM",
            "1PQPheJQSauxRPTxzNMUco1XmoCyPoEJCp",
            "35XRC5HRZjih1sML23UXv1Ry1SzTDKSmfQ",
        ];

        for i in 0..3 {
            let actual_address = Address::from_cashaddress(cashaddresses[i], ChecksumType::DSHA256, 0, 5).unwrap();
            let expected_address: Address = expected[i].into();
            assert_eq!(actual_address, expected_address);
            let actual_cashaddress = actual_address
                .to_cashaddress("bitcoincash", 0, 5)
                .unwrap()
                .encode()
                .unwrap();
            let expected_cashaddress = cashaddresses[i];
            assert_eq!(actual_cashaddress, expected_cashaddress);
        }
    }

    #[test]
    fn test_from_cashaddress_err() {
        assert_eq!(
            Address::from_cashaddress(
                "bitcoincash:qgagf7w02x4wnz3mkwnchut2vxphjzccwxgjvvjmlsxqwkcw59jxxuz",
                ChecksumType::DSHA256,
                0,
                5
            ),
            Err("Expect 20 bytes long hash".into())
        );
    }

    #[test]
    fn test_to_cashaddress_err() {
        let address = Address {
            prefix: 2,
            t_addr_prefix: 0,
            hash: [
                140, 0, 44, 191, 189, 83, 144, 173, 47, 216, 127, 59, 80, 232, 159, 100, 156, 132, 78, 192,
            ]
            .into(),
            checksum_type: ChecksumType::DSHA256,
        };

        assert_eq!(
            address.to_cashaddress("bitcoincash", 0, 5),
            Err("Unknown address prefix 2. Expect: 0, 5".into())
        );

        let address: Address = "1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM".into();
        assert_eq!(
            address.to_cashaddress("prefix", 0, 5),
            Err("Unexpected network prefix".into())
        );
    }
}
