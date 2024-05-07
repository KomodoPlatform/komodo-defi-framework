use std::io::Write;

use super::formatters::{writeln_field, ZERO_INDENT};
use crate::rpc_data::message_signing::{SignatureError, SignatureResponse, VerificationError, VerificationResponse};

pub(super) fn on_sign_message(writer: &mut dyn Write, response: SignatureResponse) {
    writeln_field(writer, "signature", response.signature, ZERO_INDENT);
}

pub(super) fn on_signature_error(writer: &mut dyn Write, error: SignatureError) {
    writeln_field(writer, "signature error", error, ZERO_INDENT);
}

pub(super) fn on_verify_message(writer: &mut dyn Write, response: VerificationResponse) {
    writeln_field(
        writer,
        "is valid",
        if response.is_valid { "valid" } else { "invalid" },
        ZERO_INDENT,
    );
}

pub(super) fn on_verification_error(writer: &mut dyn Write, error: VerificationError) {
    writeln_field(writer, "verification error", error, ZERO_INDENT);
}
