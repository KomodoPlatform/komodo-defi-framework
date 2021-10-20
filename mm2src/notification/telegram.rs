use common::mm_error::MmError;
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

#[cfg(test)]
mod telegram_tests {
    use crate::mm2::telegram::get_updates::GetUpdates;
    use crate::mm2::telegram::send_message::SendMessageRequest;
    use common::block_on;

    #[test]
    fn test_telegram_chat_id() {
        let res = block_on(GetUpdates::new("2031837575:AAEvqE-LWQLi901h5CHYIUSlIgWvKgt6V8A"));
        let chat_id = res.unwrap().get_chat_id().unwrap();
        assert_eq!(chat_id, "586216033");
    }

    #[test]
    fn test_create_send_message_request() {
        let res = block_on(SendMessageRequest::new(
            "Hello from rust".to_string(),
            "2031837575:AAEvqE-LWQLi901h5CHYIUSlIgWvKgt6V8A".to_string(),
            false,
        ));
        assert_eq!(res.is_err(), false);
    }

    #[test]
    fn test_send_message() {
        let send_request_obj = block_on(SendMessageRequest::new(
            "Hello from rust".to_string(),
            "2031837575:AAEvqE-LWQLi901h5CHYIUSlIgWvKgt6V8A".to_string(),
            true,
        ))
        .unwrap();
        let resp = block_on(send_request_obj.send()).unwrap();
        assert_eq!(resp, true);
    }
}
