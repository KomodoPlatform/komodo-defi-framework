use std::collections::HashMap;
use std::hash::Hash;
use std::iter::FromIterator;

pub trait CollectInto {
    /// Collects `FromB` from an `IntoIterator<Item=A>` given the fact that `A: Into<B>`.
    ///
    /// # Usage
    ///
    /// ```rust
    /// let actual: Vec<String> = vec!["foo", "bar"].collect_into();
    /// let expected = vec!["foo".to_owned(), "bar".to_owned()];
    /// assert_eq!(actual, expected);
    /// ```
    #[inline]
    fn collect_into<A, B, FromB>(self) -> FromB
    where
        Self: IntoIterator<Item = A> + Sized,
        A: Into<B>,
        FromB: FromIterator<B>,
    {
        self.into_iter().map(A::into).collect()
    }
}

impl<T> CollectInto for T {}

pub trait TryIntoGroupMap {
    /// An iterator method that unwraps the given `Result<(Key, Value), Err>` items yielded by the input iterator
    /// and collects `(Key, Value)` tuple pairs into a `HashMap` of keys mapped to `Vec`s of values until an `Err` error is encountered.
    fn try_into_group_map<K, V, Err>(self) -> Result<HashMap<K, Vec<V>>, Err>
    where
        Self: Iterator<Item = Result<(K, V), Err>> + Sized,
        K: Hash + Eq,
    {
        let (lower, upper) = self.size_hint();
        let capacity = upper.unwrap_or(lower);

        let mut lookup = HashMap::with_capacity(capacity);
        for res in self {
            let (key, val) = res?;
            lookup.entry(key).or_insert_with(Vec::new).push(val);
        }
        Ok(lookup)
    }
}

impl<T> TryIntoGroupMap for T {}

pub trait TryUnzip<A, B, E>
where
    Self: Iterator<Item = Result<(A, B), E>> + Sized,
{
    /// An iterator method that unwraps the given `Result<(A, B), Err>` items yielded by the input iterator
    /// and collects `(A, B)` tuple pairs into the pair of `(FromA, FromB)` containers until a `E` error is encountered.
    fn try_unzip<FromA, FromB>(self) -> Result<(FromA, FromB), E>
    where
        FromA: Default + Extend<A>,
        FromB: Default + Extend<B>,
    {
        let (mut from_a, mut from_b) = (FromA::default(), FromB::default());
        for res in self {
            let (a, b) = res?;
            from_a.extend(Some(a));
            from_b.extend(Some(b));
        }
        Ok((from_a, from_b))
    }
}

impl<T, A, B, E> TryUnzip<A, B, E> for T where T: Iterator<Item = Result<(A, B), E>> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_into() {
        let actual: Vec<String> = vec!["foo", "bar"].collect_into();
        let expected = vec!["foo".to_owned(), "bar".to_owned()];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_try_into_group_map() {
        let actual: Result<_, &'static str> = vec![Ok(("foo", 1)), Ok(("bar", 2)), Ok(("foo", 3))]
            .into_iter()
            .try_into_group_map();
        let expected: HashMap<_, _> = vec![("foo", vec![1, 3]), ("bar", vec![2])].into_iter().collect();
        assert_eq!(actual, Ok(expected));

        let err = vec![Ok(("foo", 1)), Ok(("bar", 2)), Err("Error"), Ok(("foo", 3))]
            .into_iter()
            .try_into_group_map()
            .unwrap_err();
        assert_eq!(err, "Error");
    }

    #[test]
    fn test_try_unzip() {
        let actual: Result<(Vec<_>, Vec<_>), &'static str> = vec![Ok(("foo", 1)), Ok(("bar", 2)), Ok(("foo", 3))]
            .into_iter()
            .try_unzip();
        assert_eq!(actual, Ok((vec!["foo", "bar", "foo"], vec![1, 2, 3])));

        let err = vec![Ok(("foo", 1)), Ok(("bar", 2)), Err("Error"), Ok(("foo", 3))]
            .into_iter()
            .try_unzip::<Vec<_>, Vec<_>>()
            .unwrap_err();
        assert_eq!(err, "Error");
    }
}
