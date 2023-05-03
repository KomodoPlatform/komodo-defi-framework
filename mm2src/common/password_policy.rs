use derive_more::Display;
use regex::Regex;

pub const PASSWORD_MAXIMUM_CONSECUTIVE_CHARACTERS: usize = 3;

#[derive(Debug, Display, PartialEq)]
pub enum PasswordPolicyError {
    #[display(fmt = "Password can't contain the word password")]
    ContainsTheWordPassword,
    #[display(fmt = "Password length should be at least 8 characters long")]
    PasswordLength,
    #[display(fmt = "Password should contain at least 1 digit")]
    PasswordMissDigit,
    #[display(fmt = "Password should contain at least 1 lowercase character")]
    PasswordMissLowercase,
    #[display(fmt = "Password should contain at least 1 uppercase character")]
    PasswordMissUppercase,
    #[display(fmt = "Password should contain at least 1 special character")]
    PasswordMissSpecialCharacter,
    #[display(fmt = "Password can't contain the same character 3 times in a row")]
    PasswordConsecutiveCharactersExceeded,
}

pub fn password_policy(password: &str) -> Result<(), PasswordPolicyError> {
    lazy_static! {
        static ref REGEX_NUMBER: Regex = Regex::new(".*[0-9].*").unwrap();
        static ref REGEX_LOWERCASE: Regex = Regex::new(".*[a-z].*").unwrap();
        static ref REGEX_UPPERCASE: Regex = Regex::new(".*[A-Z].*").unwrap();
        static ref REGEX_SPECIFIC_CHARS: Regex = Regex::new(".*[^A-Za-z0-9].*").unwrap();
    }
    if password.to_lowercase().contains("password") {
        return Err(PasswordPolicyError::ContainsTheWordPassword);
    }
    let password_len = password.chars().count();
    if (0..8).contains(&password_len) {
        return Err(PasswordPolicyError::PasswordLength);
    }
    if !REGEX_NUMBER.is_match(password) {
        return Err(PasswordPolicyError::PasswordMissDigit);
    }
    if !REGEX_LOWERCASE.is_match(password) {
        return Err(PasswordPolicyError::PasswordMissLowercase);
    }
    if !REGEX_UPPERCASE.is_match(password) {
        return Err(PasswordPolicyError::PasswordMissUppercase);
    }
    if !REGEX_SPECIFIC_CHARS.is_match(password) {
        return Err(PasswordPolicyError::PasswordMissSpecialCharacter);
    }
    if !super::is_acceptable_input_on_repeated_characters(password, PASSWORD_MAXIMUM_CONSECUTIVE_CHARACTERS) {
        return Err(PasswordPolicyError::PasswordConsecutiveCharactersExceeded);
    }
    Ok(())
}

#[test]
fn check_password_policy() {
    use crate::password_policy::PasswordPolicyError;
    // Length
    assert_eq!(
        password_policy("1234567").unwrap_err(),
        PasswordPolicyError::PasswordLength
    );

    // Miss special character
    assert_eq!(
        password_policy("pass123worD").unwrap_err(),
        PasswordPolicyError::PasswordMissSpecialCharacter
    );

    // Miss digit
    assert_eq!(
        password_policy("SecretPassSoStrong$*").unwrap_err(),
        PasswordPolicyError::PasswordMissDigit
    );

    // Miss lowercase
    assert_eq!(
        password_policy("SECRETPASS-SOSTRONG123*").unwrap_err(),
        PasswordPolicyError::PasswordMissLowercase
    );

    // Miss uppercase
    assert_eq!(
        password_policy("secretpass-sostrong123*").unwrap_err(),
        PasswordPolicyError::PasswordMissUppercase
    );

    // Contains the same character 3 times in a row
    assert_eq!(
        password_policy("SecretPassSoStrong123*aaa").unwrap_err(),
        PasswordPolicyError::PasswordConsecutiveCharactersExceeded
    );

    // Contains Password uppercase
    assert_eq!(
        password_policy("Password123*$").unwrap_err(),
        PasswordPolicyError::ContainsTheWordPassword
    );

    // Contains Password lowercase
    assert_eq!(
        password_policy("Foopassword123*$").unwrap_err(),
        PasswordPolicyError::ContainsTheWordPassword
    );

    // Check valid long password
    let long_pass = "SecretPassSoStrong*!1234567891012";
    assert!(long_pass.len() > 32);
    assert!(password_policy(long_pass).is_ok());

    // Valid passwords
    password_policy("StrongPass123*").unwrap();
    password_policy(r#"StrongPass123[]\± "#).unwrap();
    password_policy("StrongPass123£StrongPass123£Pass").unwrap();
}
