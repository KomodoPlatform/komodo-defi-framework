use std::io;

use crypto::dhash256;
use hash::H256;
use ser::{Deserializable, Error as ReaderError, Reader};

pub struct HashedData<T> {
    pub size: usize,
    pub hash: H256,
    pub data: T,
}

pub trait ReadAndHash {
    fn read_and_hash<T>(&mut self) -> Result<HashedData<T>, ReaderError>
    where
        T: Deserializable;
}

impl<R> ReadAndHash for Reader<R>
where
    R: io::Read,
{
    fn read_and_hash<T>(&mut self) -> Result<HashedData<T>, ReaderError>
    where
        T: Deserializable,
    {
        let mut size = 0usize;
        let mut input = vec![];
        let data = self.read_with_proxy(|bytes| {
            size += bytes.len();
            input.extend_from_slice(bytes);
        })?;

        let result = HashedData {
            hash: dhash256(&input),
            data,
            size,
        };

        Ok(result)
    }
}
