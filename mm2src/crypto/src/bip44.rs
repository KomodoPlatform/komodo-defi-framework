use crate::bip32_child::{Bip32Child, Bip32ChildValue, Bip32DerPathError, Bip44Tail, HardenedValue, NonHardenedValue};
use bip32::ChildNumber;
use derive_more::Display;
use enum_primitive_derive::Primitive;
use hw_common::primitives::Bip32Error;
use num_traits::FromPrimitive;
use std::convert::TryFrom;

pub const BIP44_PURPOSE: u32 = 44;

#[rustfmt::skip]
pub type Bip44DerivationPath =
    Bip32Child<Bip44PurposeValue, // `purpose`
    Bip32Child<HardenedValue, // `coin_type`
    Bip32Child<HardenedValue, // `account_id`
    Bip32Child<Bip44ChainValue, // `chain`
    Bip32Child<NonHardenedValue, // `address_id`
    Bip44Tail>>>>>;
#[rustfmt::skip]
pub type Bip44PathToCoin =
    Bip32Child<Bip44PurposeValue, // `purpose`
    Bip32Child<HardenedValue, // `coin_type`
    Bip44Tail>>;
#[rustfmt::skip]
pub type Bip44PathToAccount =
    Bip32Child<Bip44PurposeValue, // `purpose`
    Bip32Child<HardenedValue, // `coin_type`
    Bip32Child<HardenedValue, // `account_id`
    Bip44Tail>>>;

impl Bip44DerivationPath {
    pub fn coin_type(&self) -> u32 { self.child().value() }

    pub fn account_id(&self) -> u32 { self.child().child().value() }

    pub fn chain(&self) -> Bip44Chain { self.child().child().child().value() }

    pub fn address_id(&self) -> u32 { self.child().child().child().child().value() }
}

impl Bip44PathToCoin {
    pub fn coin_type(&self) -> u32 { self.child().value() }
}

impl Bip44PathToAccount {
    pub fn coin_type(&self) -> u32 { self.child().value() }

    pub fn account_id(&self) -> u32 { self.child().child().value() }
}

pub struct UnkownBip44ChainError {
    pub chain: u32,
}

#[derive(Debug, Display, Eq, PartialEq)]
pub enum Bip44DerPathError {
    #[display(fmt = "Invalid derivation path length '{}', expected '{}'", found, expected)]
    InvalidDerivationPathLength { expected: usize, found: usize },
    #[display(fmt = "Child '{}' is expected to be hardened", child)]
    ChildIsNotHardened { child: String },
    #[display(fmt = "Child '{}' is expected not to be hardened", child)]
    ChildIsHardened { child: String },
    #[display(fmt = "Unexpected '{}' child value '{}', expected: {}", child, value, expected)]
    UnexpectedChildValue {
        child: String,
        value: u32,
        expected: String,
    },
    #[display(fmt = "Unknown BIP32 error: {}", _0)]
    Bip32Error(Bip32Error),
}

impl From<Bip32DerPathError> for Bip44DerPathError {
    fn from(e: Bip32DerPathError) -> Self {
        fn display_child_at(child_at: usize) -> String {
            Bip44Index::from_usize(child_at)
                .map(|index| format!("{:?}", index))
                .unwrap_or_else(|| "UNKNOWN".to_owned())
        }

        match e {
            Bip32DerPathError::InvalidDerivationPathLength { expected, found } => {
                Bip44DerPathError::InvalidDerivationPathLength { expected, found }
            },
            Bip32DerPathError::ChildIsNotHardened { child_at } => Bip44DerPathError::ChildIsNotHardened {
                child: display_child_at(child_at),
            },
            Bip32DerPathError::ChildIsHardened { child_at } => Bip44DerPathError::ChildIsHardened {
                child: display_child_at(child_at),
            },
            Bip32DerPathError::UnexpectedChildValue {
                child_at,
                actual,
                expected,
            } => Bip44DerPathError::UnexpectedChildValue {
                child: display_child_at(child_at),
                value: actual,
                expected,
            },
            Bip32DerPathError::Bip32Error(bip32) => Bip44DerPathError::Bip32Error(bip32),
        }
    }
}

impl From<UnkownBip44ChainError> for Bip32DerPathError {
    fn from(e: UnkownBip44ChainError) -> Self {
        Bip32DerPathError::UnexpectedChildValue {
            child_at: Bip44Index::Chain as usize,
            actual: e.chain,
            expected: "0 or 1 chain".to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Primitive)]
pub enum Bip44Index {
    Purpose = 0,
    CoinType = 1,
    AccountId = 2,
    Chain = 3,
    AddressId = 4,
}

#[derive(Debug, Copy, Clone, Deserialize, PartialEq, Serialize)]
pub enum Bip44Chain {
    External = 0,
    Internal = 1,
}

impl TryFrom<u32> for Bip44Chain {
    type Error = UnkownBip44ChainError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Bip44Chain::External),
            1 => Ok(Bip44Chain::Internal),
            chain => Err(UnkownBip44ChainError { chain }),
        }
    }
}

impl Bip44Chain {
    pub fn to_child_number(&self) -> ChildNumber { ChildNumber::from(*self as u32) }
}

#[derive(Clone, PartialEq)]
pub struct Bip44ChainValue {
    chain: Bip44Chain,
}

impl Bip32ChildValue for Bip44ChainValue {
    type Value = Bip44Chain;

    /// `chain` is a non-hardened child as it's described in the BIP44 standard.
    fn hardened() -> bool { false }

    fn number(&self) -> u32 { self.chain as u32 }

    fn value(&self) -> Self::Value { self.chain }

    fn from_bip32_number(child_number: ChildNumber, child_at: usize) -> Result<Self, Bip32DerPathError> {
        if child_number.is_hardened() {
            return Err(Bip32DerPathError::ChildIsHardened { child_at });
        }
        Ok(Bip44ChainValue {
            chain: Bip44Chain::try_from(child_number.index())?,
        })
    }
}

#[derive(Clone, PartialEq)]
pub struct Bip44PurposeValue;

impl Bip32ChildValue for Bip44PurposeValue {
    type Value = u32;

    /// `purpose` is always a hardened child as it's described in the BIP44 standard.
    fn hardened() -> bool { true }

    fn number(&self) -> u32 { BIP44_PURPOSE }

    fn value(&self) -> u32 { BIP44_PURPOSE }

    fn from_bip32_number(child_number: ChildNumber, child_at: usize) -> Result<Self, Bip32DerPathError> {
        let purpose_child_hardened = true;
        let expected_purpose = ChildNumber::new(BIP44_PURPOSE, purpose_child_hardened)
            .expect("'BIP44_PURPOSE' is expected to be a valid index");

        if child_number != expected_purpose {
            return Err(Bip32DerPathError::UnexpectedChildValue {
                child_at,
                actual: child_number.0,
                expected: format!("'{}' BIP44 purpose", expected_purpose),
            });
        }
        Ok(Bip44PurposeValue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bip32_child::Bip32DerPathOps;
    use bip32::DerivationPath;
    use std::str::FromStr;

    #[test]
    fn test_from_str() {
        let der_path = Bip44DerivationPath::from_str("m/44'/141'/1'/0/10").unwrap();
        assert_eq!(der_path.coin_type(), 141);
        assert_eq!(der_path.account_id(), 1);
        assert_eq!(der_path.chain(), Bip44Chain::External);
        assert_eq!(der_path.address_id(), 10);
    }

    #[test]
    fn test_display() {
        let der_path = Bip44PathToAccount::from_str("m/44'/141'/1'").unwrap();
        let actual = format!("{}", der_path);
        assert_eq!(actual, "m/44'/141'/1'");
    }

    #[test]
    fn test_derive() {
        let der_path_to_coin = Bip44PathToCoin::from_str("m/44'/141'").unwrap();
        let der_path_to_account: Bip44PathToAccount =
            der_path_to_coin.derive(ChildNumber::new(10, true).unwrap()).unwrap();
        assert_eq!(
            der_path_to_account.to_derivation_path(),
            DerivationPath::from_str("m/44'/141'/10'").unwrap()
        );
    }

    #[test]
    fn test_from_invalid_length() {
        let error = Bip44DerivationPath::from_str("m/44'/141'/0'").expect_err("derivation path is too short");
        assert_eq!(error, Bip32DerPathError::InvalidDerivationPathLength {
            expected: 5,
            found: 3
        });

        let error = Bip44DerivationPath::from_str("m/44'/141'/0'/1/2/3")
            .expect_err("max number of children is 5, but 6 passes");
        assert_eq!(error, Bip32DerPathError::InvalidDerivationPathLength {
            expected: 5,
            found: 6
        });
    }

    #[test]
    fn test_from_unexpected_child_value() {
        let error = Bip44PathToAccount::from_str("m/44'/141'/0").expect_err("'account_id' is not hardened");
        assert_eq!(error, Bip32DerPathError::ChildIsNotHardened { child_at: 2 });
        let error = Bip44DerPathError::from(error);
        assert_eq!(error, Bip44DerPathError::ChildIsNotHardened {
            child: "AccountId".to_owned()
        });
    }
}
