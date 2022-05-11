use crate::mm_error::{MmError, NotMmError};

pub trait MapMmError<T, E1, E2: NotMmError> {
    fn mm_err<F>(self, f: F) -> Result<T, MmError<E2>>
    where
        F: FnOnce(E1) -> E2;
}

/// Implement mapping from [`Result<T, MmError<E1>>`] into [`Result<T, MmError<E2>>`].
impl<T, E1, E2> MapMmError<T, E1, E2> for Result<T, MmError<E1>>
where
    E1: NotMmError,
    E2: NotMmError,
{
    /// Maps a [`Result<T, MmError<E1>`] to [`Result<T, MmError<E2>>`] by applying a function to a
    /// contained [`Err`] value, leaving an [`Ok`] value untouched.
    ///
    /// # Important
    ///
    /// Please consider using `?` operator if `E2: From<E1>`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let res: Result<(), MmError<String>> = MmError::err("An error".to_owned());
    /// let mapped_res: Result<(), MmError<usize>> = res.mm_err(|e: String| e.len());
    /// ```
    #[track_caller]
    fn mm_err<F>(self, f: F) -> Result<T, MmError<E2>>
    where
        F: FnOnce(E1) -> E2,
    {
        // do not use [`Result::map_err`], because we should keep the `track_caller` chain
        match self {
            Ok(x) => Ok(x),
            Err(e1) => Err(e1.map(f)),
        }
    }
}
