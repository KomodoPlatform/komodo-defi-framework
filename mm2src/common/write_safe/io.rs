use std::fmt;
use std::io::Write;

#[macro_export]
macro_rules! write_safe_io {
    ($dst:expr, $($arg:tt)*) => {
        $dst.write_safe(format_args!($($arg)*))
    }
}
#[macro_export]
macro_rules! writeln_safe_io {
    ($dst:expr, $($arg:tt)*) => {{
        write_safe_io!($dst, $($arg)*);
        write_safe_io!($dst, "\n");
    }};
}

pub trait WriteSafeIO: Write {
    fn write_safe(&mut self, args: fmt::Arguments<'_>) {
        Write::write_fmt(self, args).expect("`write_fmt` should never fail for `WriteSafeIO` types")
    }
}

impl<'a> WriteSafeIO for dyn Write + 'a {}
