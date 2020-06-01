use num_bigint::BigInt;
use serde::{
    de,
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::fmt;

/// BigInt wrapper de/serializable from/to string representation
#[derive(Clone, Debug)]
pub struct BigIntStr(BigInt);

impl BigIntStr {
    pub fn inner(&self) -> &BigInt {
        &self.0
    }
}

impl From<BigInt> for BigIntStr {
    fn from(num: BigInt) -> BigIntStr {
        BigIntStr(num)
    }
}

impl Into<BigInt> for BigIntStr {
    fn into(self) -> BigInt {
        self.0
    }
}

impl Serialize for BigIntStr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for BigIntStr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BigIntStrVisitor;

        impl<'de> de::Visitor<'de> for BigIntStrVisitor {
            type Value = BigIntStr;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a string representing integer number")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let num: BigInt = v.parse().map_err(|e| {
                    let err = format!("Could not parse BigInt from str {}, err {}", v, e);
                    de::Error::custom(err)
                })?;
                Ok(BigIntStr(num))
            }
        }

        deserializer.deserialize_str(BigIntStrVisitor)
    }
}

#[cfg(test)]
mod big_int_str_tests {
    use super::*;
    use serde_json::{self as json};

    #[test]
    fn test_bigint_str_serialize() {
        let num = BigIntStr(1023.into());
        let expected = r#""1023""#;
        let actual = json::to_string(&num).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_bigint_str_deserialize() {
        let num = r#""1023""#;
        let expected: BigInt = 1023.into();
        let actual: BigIntStr = json::from_str(&num).unwrap();
        assert_eq!(expected, actual.0);

        let err_num = "abc";
        let res = json::from_str::<BigIntStr>(&err_num);
        assert!(res.is_err());
    }
}
