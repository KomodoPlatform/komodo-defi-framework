mod big_int_str;
pub mod bigdecimal_custom;
mod fraction;
mod mm_number;
mod mm_number_multi_repr;

pub use bigdecimal;
pub use bigdecimal::BigDecimal;
pub use fraction::Fraction;
pub use mm_number::MmNumber;
pub use mm_number_multi_repr::MmNumberMultiRepr;
pub use num_bigint;
pub use num_bigint::{BigInt, BigUint};
pub use num_rational;
pub use num_rational::BigRational;
pub use paste::paste;

pub(crate) fn from_dec_to_ratio(d: &BigDecimal) -> BigRational {
    let (num, scale) = d.as_bigint_and_exponent();
    let ten = BigInt::from(10i32);
    if scale >= 0 {
        BigRational::new(num, ten.pow(scale as u32))
    } else {
        BigRational::new(num * ten.pow((-scale) as u32), 1.into())
    }
}

pub(crate) fn from_ratio_to_dec(r: &BigRational) -> BigDecimal {
    BigDecimal::from(r.numer().clone()) / BigDecimal::from(r.denom().clone())
}
