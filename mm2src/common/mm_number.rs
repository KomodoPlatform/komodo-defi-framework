use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::Pow;
use core::ops::{Add, Div, Mul, Sub};
use serde::{Deserialize, Deserializer};

#[derive(Clone, Debug, Serialize)]
pub struct MmNumber(BigRational);

pub fn from_ratio_to_dec(r: &BigRational) -> BigDecimal {
    BigDecimal::from(r.numer().clone()) / BigDecimal::from(r.denom().clone())
}

pub fn from_dec_to_ratio(d: BigDecimal) -> BigRational {
    let (num, scale) = d.as_bigint_and_exponent();
    let ten = BigInt::from(10);
    if scale >= 0 {
        BigRational::new(num, ten.pow(scale as u64))
    } else {
        BigRational::new(num * ten.pow((-scale) as u64), 1.into())
    }
}

/// Handwritten deserialization method allows the MmNumber to be deserialized
/// not only from big rational representation, but from decimal string "0.1" too
impl<'de> Deserialize<'de> for MmNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error> where
        D: Deserializer<'de> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum MmNumberHelper {
            BigDecimal(BigDecimal),
            BigRational(BigRational),
        }

        let ratio = match Deserialize::deserialize(deserializer)? {
            MmNumberHelper::BigDecimal(x) => from_dec_to_ratio(x),
            MmNumberHelper::BigRational(x) => x
        };

        Ok(MmNumber(ratio))
    }
}

impl std::fmt::Display for MmNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", from_ratio_to_dec(&self.0))
    }
}

impl From<BigDecimal> for MmNumber {
    fn from(n: BigDecimal) -> MmNumber {
        from_dec_to_ratio(n).into()
    }
}

impl From<BigRational> for MmNumber {
    fn from(r: BigRational) -> MmNumber {
        MmNumber(r)
    }
}

impl From<MmNumber> for BigDecimal {
    fn from(n: MmNumber) -> BigDecimal {
        from_ratio_to_dec(&n.0)
    }
}

impl From<MmNumber> for BigRational {
    fn from(n: MmNumber) -> BigRational {
        n.0
    }
}

impl From<u64> for MmNumber {
    fn from(n: u64) -> MmNumber {
        BigRational::from_integer(n.into()).into()
    }
}

impl Mul for MmNumber {
    type Output = MmNumber;

    fn mul(self, rhs: Self) -> Self::Output {
        (self.0 * rhs.0).into()
    }
}

impl Mul for &MmNumber {
    type Output = MmNumber;

    fn mul(self, rhs: Self) -> Self::Output {
        let lhs = &self.0;
        let rhs = &rhs.0;
        MmNumber(lhs * rhs)
    }
}

impl Add for MmNumber {
    type Output = MmNumber;

    fn add(self, rhs: Self) -> Self::Output {
        (self.0 + rhs.0).into()
    }
}

impl Add for &MmNumber {
    type Output = MmNumber;

    fn add(self, rhs: Self) -> Self::Output {
        let lhs = &self.0;
        let rhs = &rhs.0;
        MmNumber(lhs + rhs)
    }
}

impl Sub for MmNumber {
    type Output = MmNumber;

    fn sub(self, rhs: Self) -> Self::Output {
        (self.0 - rhs.0).into()
    }
}

impl Div for MmNumber {
    type Output = MmNumber;

    fn div(self, rhs: MmNumber) -> MmNumber {
        (self.0 / rhs.0).into()
    }
}

impl Div for &MmNumber {
    type Output = MmNumber;

    fn div(self, rhs: &MmNumber) -> MmNumber {
        let lhs = &self.0;
        let rhs = &rhs.0;
        MmNumber(lhs / rhs)
    }
}

impl PartialOrd<MmNumber> for MmNumber {
    fn partial_cmp(&self, rhs: &MmNumber) -> Option<std::cmp::Ordering> {
        let lhs = from_ratio_to_dec(&self.0);
        let rhs = from_ratio_to_dec(&rhs.0);
        Some(lhs.cmp(&rhs))
    }
}

impl PartialOrd<BigDecimal> for MmNumber {
    fn partial_cmp(&self, other: &BigDecimal) -> Option<std::cmp::Ordering> {
        Some(from_ratio_to_dec(&self.0).cmp(other))
    }
}

impl PartialEq for MmNumber {
    fn eq(&self, rhs: &MmNumber) -> bool {
        let lhs = from_ratio_to_dec(&self.0);
        let rhs = from_ratio_to_dec(&rhs.0);
        lhs == rhs
    }
}

impl PartialEq<BigDecimal> for MmNumber {
    fn eq(&self, rhs: &BigDecimal) -> bool {
        let dec = from_ratio_to_dec(&self.0);
        &dec == rhs
    }
}

impl Default for MmNumber {
    fn default() -> MmNumber {
        BigRational::from_integer(0.into()).into()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use serde_json as json;
    use super::*;

    #[test]
    fn test_from_dec_to_ratio() {
        let number: BigDecimal = "11.00000000000000000000000000000000000000".parse().unwrap();
        let rational = from_dec_to_ratio(number);
        assert_eq!(*rational.numer(), 11.into());
        assert_eq!(*rational.denom(), 1.into());

        let number: BigDecimal = "0.00000001".parse().unwrap();
        let rational = from_dec_to_ratio(number);
        assert_eq!(*rational.numer(), 1.into());
        assert_eq!(*rational.denom(), 100000000.into());

        let number: BigDecimal = 1.into();
        let rational = from_dec_to_ratio(number);
        assert_eq!(*rational.numer(), 1.into());
        assert_eq!(*rational.denom(), 1.into());
    }

    #[test]
    fn test_mm_number_deserialize_from_dec() {
        let vals =
            vec!["1.0", "0.5", "50", "1e-3", "1e12", "0.3333333333333333", "3.141592653589793", "12.0010"];

        for num in vals {
            let decimal: BigDecimal = BigDecimal::from_str(num).unwrap();
            let expected: MmNumber = from_dec_to_ratio(decimal).into();
            let actual: MmNumber = json::from_str(&num).unwrap();
            assert_eq!(expected, actual);
        }
    }

    #[test]
    fn test_mm_number_deserialize_from_ratio() {
        let vals: Vec<BigRational> = vec![
            BigRational::from_integer(0.into()),
            BigRational::from_integer(81516161.into()),
            BigRational::new(370.into(), 5123.into()),
            BigRational::new(1742152.into(), 848841.into()),
        ];

        for num in vals {
            let serialized = json::to_string(&num).unwrap();
            let expected: MmNumber = num.into();
            let actual: MmNumber = json::from_str(&serialized).unwrap();
            assert_eq!(expected, actual);
        }
    }

    #[test]
    fn test_mm_number_deserialize() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Helper {
            num: MmNumber,
            nums: Vec<MmNumber>,
        }

        let data = Helper {
            num: BigRational::new(1.into(), 10.into()).into(),
            nums: vec![
                BigRational::from_integer(50.into()).into(),
                BigRational::new(1.into(), 1000.into()).into(),
                BigRational::from_integer(1000000000000i64.into()).into(),
                BigRational::new(33.into(), 100.into()).into(),
                BigRational::new(5.into(), 2.into()).into()
            ],
        };

        // A JSON input with plenty of whitespace.
        let json = json!({
            "num": "0.1",
            "nums": ["50", "1e-3", "1e12", "0.33", "2.5"]
        });

        assert_eq!(data, json::from_value(json).unwrap());
    }
}
