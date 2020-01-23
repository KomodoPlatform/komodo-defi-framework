use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::Pow;
use core::ops::{Add, Div, Mul, Sub};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MmNumber {
    BigDecimal(BigDecimal),
    BigRational(BigRational),
}

impl std::fmt::Display for MmNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            MmNumber::BigDecimal(d) => write!(f, "{}", d),
            MmNumber::BigRational(r) => write!(f, "{}", from_ratio_to_dec(r)),
        }
    }
}

impl From<BigDecimal> for MmNumber {
    fn from(n: BigDecimal) -> MmNumber {
        MmNumber::BigDecimal(n)
    }
}

impl From<BigRational> for MmNumber {
    fn from(r: BigRational) -> MmNumber {
        MmNumber::BigRational(r)
    }
}

impl From<MmNumber> for BigDecimal {
    fn from(n: MmNumber) -> BigDecimal {
        match n {
            MmNumber::BigDecimal(d) => d,
            MmNumber::BigRational(r) => from_ratio_to_dec(&r),
        }
    }
}

impl From<MmNumber> for BigRational {
    fn from(n: MmNumber) -> BigRational {
        match n {
            MmNumber::BigDecimal(d) => from_dec_to_ratio(d),
            MmNumber::BigRational(r) => r,
        }
    }
}

impl From<u64> for MmNumber {
    fn from(n: u64) -> MmNumber {
        BigRational::from_integer(n.into()).into()
    }
}

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

impl Mul for MmNumber {
    type Output = MmNumber;

    fn mul(self, rhs: Self) -> Self::Output {
        let lhs: BigRational = self.into();
        let rhs: BigRational = rhs.into();
        MmNumber::from(lhs * rhs)
    }
}

impl Mul for &MmNumber {
    type Output = MmNumber;

    fn mul(self, rhs: Self) -> Self::Output {
        let lhs: BigRational = self.clone().into();
        let rhs: BigRational = rhs.clone().into();
        MmNumber::from(lhs * rhs)
    }
}

impl Add for MmNumber {
    type Output = MmNumber;

    fn add(self, rhs: Self) -> Self::Output {
        let lhs: BigRational = self.into();
        let rhs: BigRational = rhs.into();
        (lhs + rhs).into()
    }
}

impl Sub for MmNumber {
    type Output = MmNumber;

    fn sub(self, rhs: Self) -> Self::Output {
        let lhs: BigRational = self.into();
        let rhs: BigRational = rhs.into();
        (lhs - rhs).into()
    }
}

impl Add for &MmNumber {
    type Output = MmNumber;

    fn add(self, rhs: Self) -> Self::Output {
        let lhs: BigRational = self.clone().into();
        let rhs: BigRational = rhs.clone().into();
        (lhs + rhs).into()
    }
}

impl PartialOrd<BigDecimal> for MmNumber {
    fn partial_cmp(&self, other: &BigDecimal) -> Option<std::cmp::Ordering> {
        match self {
            MmNumber::BigDecimal(d) => Some(d.cmp(other)),
            MmNumber::BigRational(r) => Some(from_ratio_to_dec(&r).cmp(other)),
        }
    }
}

impl PartialOrd<MmNumber> for MmNumber {
    fn partial_cmp(&self, other: &MmNumber) -> Option<std::cmp::Ordering> {
        match self {
            MmNumber::BigDecimal(lhs) => match other {
                MmNumber::BigDecimal(rhs) => Some(lhs.cmp(rhs)),
                MmNumber::BigRational(rhs) => Some(lhs.cmp(&from_ratio_to_dec(rhs))),
            }
            MmNumber::BigRational(lhs) => match other {
                MmNumber::BigDecimal(rhs) => Some(from_ratio_to_dec(lhs).cmp(rhs)),
                MmNumber::BigRational(rhs) => Some(lhs.cmp(rhs)),
            },
        }
    }
}

impl PartialEq<BigDecimal> for MmNumber {
    fn eq(&self, rhs: &BigDecimal) -> bool {
        match self {
            MmNumber::BigDecimal(d) => d == rhs,
            MmNumber::BigRational(r) => {
                let dec = from_ratio_to_dec(&r);
                &dec == rhs
            },
        }
    }
}

impl Default for MmNumber {
    fn default() -> MmNumber {
        BigRational::from_integer(0.into()).into()
    }
}

impl Div for MmNumber {
    type Output = MmNumber;

    fn div(self, rhs: MmNumber) -> MmNumber {
        let lhs: BigRational = self.into();
        let rhs: BigRational = rhs.into();
        (lhs / rhs).into()
    }
}

impl Div for &MmNumber {
    type Output = MmNumber;

    fn div(self, rhs: &MmNumber) -> MmNumber {
        let lhs: BigRational = self.clone().into();
        let rhs: BigRational = rhs.clone().into();
        (lhs / rhs).into()
    }
}

impl PartialEq for MmNumber {
    fn eq(&self, rhs: &MmNumber) -> bool {
        match self {
            MmNumber::BigDecimal(lhs) => match rhs {
                MmNumber::BigDecimal(rhs) => lhs == rhs,
                MmNumber::BigRational(rhs) => lhs == &from_ratio_to_dec(rhs),
            },
            MmNumber::BigRational(lhs) => match rhs {
                MmNumber::BigDecimal(rhs) => &from_ratio_to_dec(lhs) == rhs,
                MmNumber::BigRational(rhs) => lhs == rhs,
            }
        }
    }
}

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
