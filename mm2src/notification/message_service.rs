#[cfg(test)]
#[path = "telegram/telegram.rs"]
pub mod telegram;

use crate::mm2::message_service::telegram::TelegramError;
use async_trait::async_trait;
use common::mm_error::MmError;
use derive_more::Display;

pub type MessageResult<T> = Result<T, MmError<MessageError>>;

#[derive(Debug, Display)]
pub enum MessageError {
    #[display(fmt = "{}", _0)]
    TelegramError(TelegramError),
}

impl From<TelegramError> for MessageError {
    fn from(e: TelegramError) -> Self { MessageError::TelegramError(e) }
}

#[async_trait]
pub trait MessageServiceTraits {
    async fn send_message(&self, message: String, disable_notification: bool) -> MessageResult<bool>;
}

#[derive(Default)]
pub struct MessageService {
    services: Vec<Box<dyn MessageServiceTraits>>,
}

impl MessageService {
    async fn send_message(&self, message: String, disable_notification: bool) -> MessageResult<bool> {
        for service in self.services.iter() {
            service.send_message(message.clone(), disable_notification).await?;
        }
        Ok(true)
    }

    fn attach_service(&mut self, service: Box<dyn MessageServiceTraits>) -> &MessageService {
        self.services.push(service);
        self
    }

    fn nb_services(&self) -> usize { self.services.len() }

    fn new() -> Self { Default::default() }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod message_service_tests {
    use crate::mm2::message_service::telegram::TgClient;
    use crate::mm2::message_service::MessageService;
    use common::block_on;
    use std::env::var;

    #[test]
    fn test_attach_service() {
        let api_key = var("TELEGRAM_API_KEY").ok().unwrap();
        let tg_client = TgClient::new(api_key, None, None);
        let nb_services = MessageService::new().attach_service(Box::new(tg_client)).nb_services();
        assert_eq!(nb_services, 1);
    }

    #[test]
    fn test_send_message_service() {
        let api_key = var("TELEGRAM_API_KEY").ok().unwrap();
        let tg_client = TgClient::new(api_key, None, Some("586216033".to_string()));
        let mut message_service = MessageService::new();
        message_service.attach_service(Box::new(tg_client));
        let res =
            block_on(message_service.send_message("Hey it's the message service, do you hear me?".to_string(), true));
        assert_eq!(res.is_err(), false);
        assert_eq!(res.unwrap(), true);
    }
}
