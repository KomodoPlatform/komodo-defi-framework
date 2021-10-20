use crate::mm2::telegram::get_updates::GetUpdates;
use crate::mm2::telegram::send_message::SendMessageResponse;
use common::mm_error::prelude::MapToMmResult;
use common::mm_error::MmError;
use common::{fetch_json, post_json};
use derive_more::Display;

pub const TELEGRAM_BOT_API_ENDPOINT: &str = "https://api.telegram.org/bot";

pub mod get_updates;
pub mod send_message;

#[derive(Debug, Display)]
pub enum TelegramError {
    ChatIdNotAvailable(String),
    Transport(String),
    InvalidResponse(String),
    InvalidRequest(String),
}

impl From<serde_json::Error> for TelegramError {
    fn from(e: serde_json::Error) -> Self { TelegramError::InvalidResponse(e.to_string()) }
}

pub type TelegramResult<T> = Result<T, MmError<TelegramError>>;

pub struct TgClient {
    url: String,
    api_key: String,
}

impl TgClient {
    pub fn new(api_key: String, url: Option<String>) -> Self {
        TgClient {
            url: url.unwrap_or(TELEGRAM_BOT_API_ENDPOINT.to_string()),
            api_key,
        }
    }

    pub async fn get_updates(&self) -> TelegramResult<GetUpdates> {
        let uri = self.url.to_owned() + &self.api_key + "/getUpdates";
        let result = fetch_json::<GetUpdates>(uri.as_str())
            .await
            .map_to_mm(TelegramError::Transport)?;
        Ok(result)
    }

    // This is a telegram client, later add a NotificationOps and a generic `send` function which here would call send_message
    pub async fn send_message(&self, message: String, disable_notification: bool) -> TelegramResult<bool> {
        let uri = TELEGRAM_BOT_API_ENDPOINT.to_owned() + self.api_key.as_str() + "/sendMessage";
        let chat_id = self.get_updates().await?.get_chat_id()?;
        let json = json!({ "chat_id": chat_id, "text": message, "disable_notification": disable_notification });
        let result = post_json::<SendMessageResponse>(uri.as_str(), json.to_string())
            .await
            .map_to_mm(TelegramError::Transport)?;
        Ok(result.ok)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod telegram_tests {
    use crate::mm2::telegram::TgClient;
    use common::block_on;
    use std::env::var;

    #[test]
    fn test_telegram_chat_id() {
        let api_key = var("TELEGRAM_API_KEY").ok().unwrap();
        let tg_client = TgClient::new(api_key, None);
        let res = block_on(tg_client.get_updates()).unwrap();
        let chat_id = res.get_chat_id().unwrap();
        assert_eq!(chat_id, "586216033");
    }

    #[test]
    fn test_send_message() {
        let api_key = var("TELEGRAM_API_KEY").ok().unwrap();
        let tg_client = TgClient::new(api_key, None);
        let resp = block_on(tg_client.send_message("Hello from rust".to_string(), true)).unwrap();
        assert_eq!(resp, true);
    }
}
