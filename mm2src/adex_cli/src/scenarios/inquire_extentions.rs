use std::str::FromStr;

use inquire::parser::DEFAULT_BOOL_PARSER;

#[derive(Clone)]
pub(super) enum InquireOption<T> {
    Some(T),
    None,
}

type OptionalConfirm = InquireOption<bool>;

impl<T> From<InquireOption<T>> for Option<T> {
    fn from(value: InquireOption<T>) -> Self {
        match value {
            InquireOption::None => None,
            InquireOption::Some(value) => Some(value),
        }
    }
}

impl<T: FromStr> FromStr for InquireOption<T>
where
    <T as FromStr>::Err: ToString,
{
    type Err = T::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.to_lowercase() == "none" {
            return Ok(InquireOption::None);
        }
        T::from_str(s).map(InquireOption::Some)
    }
}

impl<T: ToString> ToString for InquireOption<T> {
    fn to_string(&self) -> String {
        match self {
            InquireOption::Some(value) => value.to_string(),
            InquireOption::None => "None".to_string(),
        }
    }
}

type OptionBoolFormatter<'a> = &'a dyn Fn(OptionalConfirm) -> String;
pub(super) const DEFAULT_OPTION_BOOL_FORMATTER: OptionBoolFormatter = &|ans| -> String {
    match ans {
        InquireOption::None => String::new(),
        InquireOption::Some(true) => String::from("yes"),
        InquireOption::Some(false) => String::from("no"),
    }
};

type OptionBoolParser<'a> = &'a dyn Fn(&str) -> Result<InquireOption<bool>, ()>;
pub(super) const OPTION_BOOL_PARSER: OptionBoolParser = &|ans: &str| -> Result<InquireOption<bool>, ()> {
    if ans.is_empty() {
        return Ok(InquireOption::None);
    }
    DEFAULT_BOOL_PARSER(ans).map(InquireOption::Some)
};

pub(super) const DEFAULT_DEFAULT_OPTION_BOOL_FORMATTER: OptionBoolFormatter = &|ans: InquireOption<bool>| match ans {
    InquireOption::None => String::from("Tap enter to skip/yes/no"),
    InquireOption::Some(true) => String::from("none/Yes/no"),
    InquireOption::Some(false) => String::from("none/yes/No"),
};
