use std::fmt;

/// An alternative to the `write!` standard library macro that can never fail.
/// The macro takes a `WriteSafe` writer as a `$dst` destination.
#[macro_export]
macro_rules! write_safe {
    ($dst:expr, $($arg:tt)*) => {
        $dst.write_safe(format_args!($($arg)*))
    }
}

/// The trait is implemented for those types for those [`std::fmt::Write::write_fmt`] never fails.
pub trait WriteSafe: fmt::Write {
    fn write_safe(&mut self, args: fmt::Arguments<'_>) {
        fmt::Write::write_fmt(self, args).expect("`write_fmt` should never fail for `WriteSafe` types")
    }
}

impl WriteSafe for String {}

pub trait WriteJoin: Iterator + Sized
where
    Self::Item: fmt::Display,
{
    fn write_join<W>(self, writer: &mut W, sep: &str) -> fmt::Result
    where
        W: fmt::Write,
    {
        let mut first = true;
        for item in self {
            if first {
                write!(writer, "{}", item)?;
            } else {
                write!(writer, "{}{}", sep, item)?;
            }
            first = false;
        }
        Ok(())
    }
}

impl<I, T> WriteJoin for I
where
    I: Iterator<Item = T> + Sized,
    T: fmt::Display,
{
}

pub trait WriteSafeJoin: Iterator + Sized
where
    Self::Item: fmt::Display,
{
    fn write_safe_join<W>(self, writer: &mut W, sep: &str)
    where
        W: WriteSafe,
    {
        let mut first = true;
        for item in self {
            if first {
                write_safe!(writer, "{}", item);
            } else {
                write_safe!(writer, "{}{}", sep, item);
            }
            first = false;
        }
    }
}

impl<I, T> WriteSafeJoin for I
where
    I: Iterator<Item = T> + Sized,
    T: fmt::Display,
{
}
