//! This crate provides an abstraction layer on Git for doing query/parse
//! operations over the repositories.
//!
//! Implementation of generic `GitController` provides the flexibility of
//! adding any Git clients(like Gitlab, Bitbucket, etc) when needed.

use async_trait::async_trait;
use mm2_err_handle::prelude::MmError;
use serde::{de::DeserializeOwned, Deserialize};

pub mod github_client;
pub use github_client::*;

pub const GITHUB_API_URI: &str = "https://api.github.com";

#[derive(Clone, Debug, Deserialize)]
pub struct FileMetadata {
    pub name: String,
    pub download_url: String,
    pub size: usize,
}

pub trait GitCommons {
    fn new(api_address: String) -> Self;
}

#[async_trait]
pub trait RepositoryOperations {
    async fn deserialize_json_source<T: DeserializeOwned>(
        &self,
        file_metadata: FileMetadata,
    ) -> Result<T, MmError<GitControllerError>>;

    async fn get_file_metadata_list(
        &self,
        owner: &str,
        repository_name: &str,
        branch: &str,
        dir: &str,
    ) -> Result<Vec<FileMetadata>, MmError<GitControllerError>>;
}

pub struct GitController<T: RepositoryOperations> {
    pub client: T,
}

impl<T: GitCommons + RepositoryOperations> GitController<T> {
    pub fn new(api_address: &str) -> Self {
        Self {
            client: T::new(api_address.to_owned()),
        }
    }
}

#[derive(Debug)]
pub enum GitControllerError {
    DeserializationError(String),
    HttpError(String),
}
