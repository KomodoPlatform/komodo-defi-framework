use bigdecimal::BigDecimal;
use bigdecimal::Zero;
use std::ops::Div;

pub trait CheckedDiv {
    fn checked_div(self, other: BigDecimal) -> Option<BigDecimal>;
}

impl CheckedDiv for BigDecimal {
    #[inline]
    fn checked_div(self, other: BigDecimal) -> Option<Self> {
        if other.is_zero() {
            None
        } else {
            Some(self.div(other))
        }
    }
}
