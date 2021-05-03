use super::{MmError, NotMmError};

pub trait OrMmError<T, E: NotMmError> {
    fn or_mm_err<F>(self, f: F) -> Result<T, MmError<E>>
    where
        F: FnOnce() -> E;
}

impl<T, E: NotMmError> OrMmError<T, E> for Option<T> {
    /// Transforms the `Option<T>` into a [`Result<T, MmError<E>>`], mapping [`Some(v)`] to
    /// [`Ok(v)`] and [`None`] to [`Err(MmError<E>)`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// let res: Option<String> = None;
    /// let mapped_res: Result<(), MmError<usize>> = res.or_mm_err(|| 123usize);
    /// ```
    #[track_caller]
    fn or_mm_err<F>(self, f: F) -> Result<T, MmError<E>>
    where
        F: FnOnce() -> E,
    {
        match self {
            Some(x) => Ok(x),
            None => MmError::err(f()),
        }
    }
}
