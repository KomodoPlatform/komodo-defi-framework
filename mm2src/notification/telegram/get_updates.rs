use crate::mm2::message_service::telegram::{TelegramError, TelegramResult};
use common::mm_error::MmError;

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
