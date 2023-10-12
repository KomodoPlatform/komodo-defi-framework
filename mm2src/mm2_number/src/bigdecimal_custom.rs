use std::ops::Div;

use bigdecimal::BigDecimal;
use bigdecimal::Zero;

pub trait CheckedDivision {
    fn checked_div(self, other: BigDecimal) -> Option<BigDecimal>;
}

impl CheckedDivision for BigDecimal {
    #[inline]
    fn checked_div(self, other: BigDecimal) -> Option<Self> {
        if other.is_zero() {
            None
        } else {
            Some(self.div(other))
        }
    }
}
