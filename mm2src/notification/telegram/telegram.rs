use crate::mm2::message_service::telegram::get_updates::GetUpdates;
use crate::mm2::message_service::{MessageResult, MessageServiceTraits};
use async_trait::async_trait;
use common::mm_error::prelude::MapToMmResult;
use common::mm_error::MmError;
use common::{fetch_json, post_json};
use derive_more::Display;

pub const TELEGRAM_BOT_API_ENDPOINT: &str = "https://api.telegram.org/bot";

pub mod get_updates;

#[derive(Debug, Deserialize)]
pub struct SendMessageResponse {
    pub ok: bool,
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum TelegramError {
    #[display(fmt = "Telegram Chat Id not available: {}", _0)]
    ChatIdNotAvailable(String),
    #[display(fmt = "Telegram transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Telegram invalid response: {}", _0)]
    InvalidResponse(String),
}

impl From<serde_json::Error> for TelegramError {
    fn from(e: serde_json::Error) -> Self { TelegramError::InvalidResponse(e.to_string()) }
}

pub type TelegramResult<T> = Result<T, MmError<TelegramError>>;

#[derive(Clone)]
pub struct TgClient {
    url: String,
    api_key: String,
    chat_id: Option<String>,
}

impl TgClient {
    pub fn new(api_key: String, url: Option<String>, chat_id: Option<String>) -> Self {
        TgClient {
            url: url.unwrap_or_else(|| TELEGRAM_BOT_API_ENDPOINT.to_string()),
            api_key,
            chat_id,
        }
    }

    pub async fn get_updates(&self) -> TelegramResult<GetUpdates> {
        let uri = self.url.to_owned() + &self.api_key + "/getUpdates";
        let result = fetch_json::<GetUpdates>(uri.as_str())
            .await
            .map_to_mm(TelegramError::Transport)?;
        Ok(result)
    }
}

#[async_trait]
impl MessageServiceTraits for TgClient {
    async fn send_message(&self, message: String, disable_notification: bool) -> MessageResult<bool> {
        let uri = self.url.to_owned() + self.api_key.as_str() + "/sendMessage";
        let chat_id = self.chat_id.clone().unwrap_or(self.get_updates().await?.get_chat_id()?);
        let json = json!({ "chat_id": chat_id, "text": message, "disable_notification": disable_notification });
        let result = post_json::<SendMessageResponse>(uri.as_str(), json.to_string())
            .await
            .map_to_mm(TelegramError::Transport)?;
        Ok(result.ok)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod telegram_tests {
    use crate::mm2::message_service::telegram::TgClient;
    use crate::mm2::message_service::MessageServiceTraits;
    use common::block_on;
    use std::env::var;

    #[test]
    #[ignore]
    fn test_telegram_chat_id() {
        let api_key = var("TELEGRAM_API_KEY").ok().unwrap();
        let tg_client = TgClient::new(api_key, None, None);
        let res = block_on(tg_client.get_updates()).unwrap();
        let chat_id = res.get_chat_id().unwrap();
        assert_eq!(chat_id, "586216033");
    }

    #[test]
    fn test_send_message() {
        let api_key = var("TELEGRAM_API_KEY").ok().unwrap();
        let tg_client = TgClient::new(api_key, None, Some("586216033".to_string()));
        let resp = block_on(tg_client.send_message("Hello from rust".to_string(), true)).unwrap();
        assert_eq!(resp, true);
    }
}
