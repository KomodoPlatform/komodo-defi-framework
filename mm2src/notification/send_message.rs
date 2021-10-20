use crate::mm2::telegram::get_updates::GetUpdates;
use crate::mm2::telegram::{TelegramError, TelegramResult, TELEGRAM_BOT_API_ENDPOINT};
use common::mm_error::prelude::MapToMmResult;
use common::mm_error::MmError;
use common::slurp_post_json;
use http::StatusCode;
use serde_json as json;

pub struct SendMessageRequest {
    message: String,
    api_token: String,
    chat_id: String,
    disable_notification: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendMessageResponse {
    ok: bool,
}

impl SendMessageRequest {
    pub async fn new(message: String, api_token: String, disable_notification: bool) -> TelegramResult<Self> {
        let res = GetUpdates::new(api_token.as_str()).await?.get_chat_id()?;
        Ok(SendMessageRequest {
            message,
            api_token,
            chat_id: res,
            disable_notification,
        })
    }

    pub async fn send(&self) -> TelegramResult<bool> {
        let uri = TELEGRAM_BOT_API_ENDPOINT.to_owned() + self.api_token.as_str() + "/sendMessage";
        let json =
            json!({ "chat_id": self.chat_id, "text": self.message, "disable_notification": self.disable_notification });
        let resp = slurp_post_json(uri.as_str(), json.to_string())
            .await
            .map_to_mm(TelegramError::Transport)?;
        if resp.0 != StatusCode::OK {
            let error = format!("Send message request failed with status code {}", resp.0);
            return MmError::err(TelegramError::Transport(error));
        }
        let result: SendMessageResponse = json::from_slice(&resp.2)?;
        Ok(result.ok)
    }
}
