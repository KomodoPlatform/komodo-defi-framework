/// This module contains type casting functions for numbers in a
/// safe way.
///
/// The problem comes with when casting a type that supports higher value
/// than the target type's highest value.
///
/// eg:
/// ```rs
/// let x: u64 = 4294967295 + 10;
/// assert_eq!(x as u32, std::u32::MAX, "{} is not {}", x as u32, std::u32::MAX);
/// ```

pub trait SafeTypeCastingNumbers<T>: Sized {
    fn into_or(self, or: T) -> T;
    fn into_or_max(self) -> T;
}

macro_rules! impl_safe_number_type_cast {
    ($from: ident, $to: ident) => {
        impl SafeTypeCastingNumbers<$to> for $from {
            fn into_or(self, or: $to) -> $to { std::convert::TryFrom::try_from(self).unwrap_or(or) }
            fn into_or_max(self) -> $to { std::convert::TryFrom::try_from(self).unwrap_or(std::$to::MAX) }
        }
    };
}

// USIZE
impl_safe_number_type_cast!(usize, i64);
impl_safe_number_type_cast!(usize, i32);
impl_safe_number_type_cast!(usize, i16);
impl_safe_number_type_cast!(usize, i8);
impl_safe_number_type_cast!(usize, isize);

impl_safe_number_type_cast!(usize, u32);
impl_safe_number_type_cast!(usize, u16);
impl_safe_number_type_cast!(usize, u8);

// ISIZE
impl_safe_number_type_cast!(isize, i32);
impl_safe_number_type_cast!(isize, i16);
impl_safe_number_type_cast!(isize, i8);

impl_safe_number_type_cast!(isize, u32);
impl_safe_number_type_cast!(isize, u16);
impl_safe_number_type_cast!(isize, u8);

// U128
impl_safe_number_type_cast!(u128, isize);
impl_safe_number_type_cast!(u128, i128);
impl_safe_number_type_cast!(u128, i64);
impl_safe_number_type_cast!(u128, i32);
impl_safe_number_type_cast!(u128, i16);
impl_safe_number_type_cast!(u128, i8);
impl_safe_number_type_cast!(u128, isize);

impl_safe_number_type_cast!(u128, u64);
impl_safe_number_type_cast!(u128, u32);
impl_safe_number_type_cast!(u128, u16);
impl_safe_number_type_cast!(u128, u8);
impl_safe_number_type_cast!(u128, usize);

// I128
impl_safe_number_type_cast!(i128, i64);
impl_safe_number_type_cast!(i128, i32);
impl_safe_number_type_cast!(i128, i16);
impl_safe_number_type_cast!(i128, i8);
impl_safe_number_type_cast!(i128, isize);

impl_safe_number_type_cast!(i128, u64);
impl_safe_number_type_cast!(i128, u32);
impl_safe_number_type_cast!(i128, u16);
impl_safe_number_type_cast!(i128, u8);
impl_safe_number_type_cast!(i128, usize);

// U64
impl_safe_number_type_cast!(u64, i64);
impl_safe_number_type_cast!(u64, i32);
impl_safe_number_type_cast!(u64, i16);
impl_safe_number_type_cast!(u64, i8);
impl_safe_number_type_cast!(u64, isize);

impl_safe_number_type_cast!(u64, u32);
impl_safe_number_type_cast!(u64, u16);
impl_safe_number_type_cast!(u64, u8);
impl_safe_number_type_cast!(u64, usize);

// I64
impl_safe_number_type_cast!(i64, i32);
impl_safe_number_type_cast!(i64, i16);
impl_safe_number_type_cast!(i64, i8);
impl_safe_number_type_cast!(i64, isize);

impl_safe_number_type_cast!(i64, u32);
impl_safe_number_type_cast!(i64, u16);
impl_safe_number_type_cast!(i64, u8);
impl_safe_number_type_cast!(i64, usize);

// U32
impl_safe_number_type_cast!(u32, i32);
impl_safe_number_type_cast!(u32, i16);
impl_safe_number_type_cast!(u32, i8);

impl_safe_number_type_cast!(u32, u16);
impl_safe_number_type_cast!(u32, u8);

// I32
impl_safe_number_type_cast!(i32, i16);
impl_safe_number_type_cast!(i32, i8);

impl_safe_number_type_cast!(i32, u16);
impl_safe_number_type_cast!(i32, u8);

// U16
impl_safe_number_type_cast!(u16, i16);
impl_safe_number_type_cast!(u16, i8);

impl_safe_number_type_cast!(u16, u8);

// I16
impl_safe_number_type_cast!(i16, i8);

impl_safe_number_type_cast!(i16, u8);

// U8
impl_safe_number_type_cast!(u8, i8);
