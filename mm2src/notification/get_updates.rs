use crate::mm2::telegram::{TelegramError, TelegramResult, TELEGRAM_BOT_API_ENDPOINT};
use common::mm_error::prelude::MapToMmResult;
use common::mm_error::MmError;
use common::slurp_url;
use http::StatusCode;
use serde_json as json;

#[derive(Debug, Serialize, Deserialize)]
pub struct GetUpdates {
    ok: bool,
    result: Vec<GetUpdatesResult>,
}

impl GetUpdates {
    // https://stackoverflow.com/questions/31078710/how-to-obtain-telegram-chat-id-for-a-specific-user
    pub fn get_chat_id(&self) -> TelegramResult<String> {
        if self.result.is_empty() {
            return MmError::err(TelegramError::ChatIdNotAvailable(
                "No Chat ID available, please send a message to your bot first".to_string(),
            ));
        }
        Ok(self.result[0].message.chat.id.to_string())
    }

    pub async fn new(api_token: &str) -> TelegramResult<GetUpdates> {
        let uri = TELEGRAM_BOT_API_ENDPOINT.to_owned() + api_token + "/getUpdates";
        let resp = slurp_url(uri.as_str()).await.map_to_mm(TelegramError::Transport)?;
        if resp.0 != StatusCode::OK {
            let error = format!("Get updates request failed with status code {}", resp.0);
            return MmError::err(TelegramError::Transport(error));
        }
        let result: GetUpdates = json::from_slice(&resp.2)?;
        Ok(result)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetUpdatesResult {
    update_id: i64,
    message: Message,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    message_id: i64,
    chat: Chat,
    date: i64,
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Chat {
    id: i64,
    first_name: String,
    last_name: String,
    username: String,
    #[serde(rename = "type")]
    chat_type: String,
}
