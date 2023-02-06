use async_trait::async_trait;
use http::{header, Request};
use mm2_err_handle::prelude::MmError;
use mm2_net::transport::slurp_req;
use serde::de::DeserializeOwned;

use crate::{FileMetadata, GitCommons, GitControllerError, RepositoryOperations};

pub struct GithubClient {
    api_address: String,
}

impl GitCommons for GithubClient {
    fn new(api_address: String) -> Self { Self { api_address } }
}

#[async_trait]
impl RepositoryOperations for GithubClient {
    async fn deserialize_json_source<T: DeserializeOwned>(
        &self,
        file_metadata: FileMetadata,
    ) -> Result<T, MmError<GitControllerError>> {
        let req = Request::builder()
            .header(header::USER_AGENT, "mm2")
            .uri(&file_metadata.download_url)
            .body(Vec::new())
            .map_err(|e| GitControllerError::HttpError(e.to_string()))?;

        let (_status_code, _headers, data_buffer) = slurp_req(req)
            .await
            .map_err(|e| GitControllerError::HttpError(e.to_string()))?;

        Ok(
            serde_json::from_slice(&data_buffer)
                .map_err(|e| GitControllerError::DeserializationError(e.to_string()))?,
        )
    }

    async fn get_file_metadata_list(
        &self,
        owner: &str,
        repository_name: &str,
        branch: &str,
        dir: &str,
    ) -> Result<Vec<FileMetadata>, MmError<GitControllerError>> {
        let uri = format!(
            "{}/repos/{}/{}/contents/{}?ref={}",
            &self.api_address, owner, repository_name, dir, branch
        );

        let req = Request::builder()
            .header(header::USER_AGENT, "mm2")
            .uri(uri)
            .body(Vec::new())
            .map_err(|e| GitControllerError::HttpError(e.to_string()))?;

        let (_status_code, _headers, data_buffer) = slurp_req(req)
            .await
            .map_err(|e| GitControllerError::HttpError(e.to_string()))?;

        Ok(
            serde_json::from_slice(&data_buffer)
                .map_err(|e| GitControllerError::DeserializationError(e.to_string()))?,
        )
    }
}

#[cfg(test)]
#[allow(unused)]
mod tests {
    use crate::{GitController, GITHUB_API_URI};

    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct ChainRegistry {
        chain_1: ChainInfo,
        chain_2: ChainInfo,
        channels: Vec<IbcChannel>,
    }

    #[derive(Debug, Deserialize)]
    struct IbcChannel {
        chain_1: ChannelInfo,
        chain_2: ChannelInfo,
        ordering: String,
        version: String,
        tags: Option<ChannelTag>,
    }

    #[derive(Debug, Deserialize)]
    struct ChainInfo {
        chain_name: String,
        client_id: String,
        connection_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct ChannelInfo {
        channel_id: String,
        port_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct ChannelTag {
        status: String,
        preferred: bool,
        dex: Option<String>,
    }

    #[test]
    fn test_metadata_list_and_json_deserialization() {
        const REPO_OWNER: &str = "cosmos";
        const REPO_NAME: &str = "chain-registry";
        const BRANCH: &str = "master";
        const DIR_NAME: &str = "_IBC";

        let git_controller: GitController<GithubClient> = GitController::new(GITHUB_API_URI);

        let metadata_list = common::block_on(
            git_controller
                .client
                .get_file_metadata_list(REPO_OWNER, REPO_NAME, BRANCH, DIR_NAME),
        )
        .unwrap();

        assert!(!metadata_list.is_empty());

        common::block_on(
            git_controller
                .client
                .deserialize_json_source::<ChainRegistry>(metadata_list.first().unwrap().clone()),
        )
        .unwrap();
    }
}
