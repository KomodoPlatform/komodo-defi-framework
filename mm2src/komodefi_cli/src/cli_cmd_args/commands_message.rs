use clap::{Args, Subcommand};
use std::mem::take;

use crate::rpc_data::message_signing::{SignatureRequest, VerificationRequest};

#[derive(Subcommand)]
pub(crate) enum MessageCommands {
    #[command(
        short_flag = 's',
        about = "If your coins file contains the correct message prefix definitions, you can sign \
                 message to prove ownership of an address"
    )]
    Sign(SignMessageArgs),
    #[command(short_flag = 'v', about = "Verify message signature")]
    Verify(VerifyMessageArgs),
}

#[derive(Args)]
pub(crate) struct SignMessageArgs {
    #[arg(long, short, help = "The coin to sign a message with")]
    coin: String,
    #[arg(long, short, help = "The message you want to sign")]
    message: String,
}

impl From<&mut SignMessageArgs> for SignatureRequest {
    fn from(value: &mut SignMessageArgs) -> Self {
        SignatureRequest {
            coin: take(&mut value.coin),
            message: take(&mut value.message),
        }
    }
}

#[derive(Args)]
pub(crate) struct VerifyMessageArgs {
    #[arg(long, short, help = "The coin to sign a message with")]
    coin: String,
    #[arg(long, short, help = "The message input via the sign_message method sign")]
    message: String,
    #[arg(long, short, help = "The signature generated for the message")]
    signature: String,
    #[arg(long, short, help = "The address used to sign the message")]
    address: String,
}

impl From<&mut VerifyMessageArgs> for VerificationRequest {
    fn from(value: &mut VerifyMessageArgs) -> Self {
        VerificationRequest {
            coin: take(&mut value.coin),
            message: take(&mut value.message),
            signature: take(&mut value.signature),
            address: take(&mut value.address),
        }
    }
}
