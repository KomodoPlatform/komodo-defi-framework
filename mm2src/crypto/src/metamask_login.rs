use lazy_static::lazy_static;
use mm2_metamask::{ObjectType, PropertyType};

pub(crate) const ADEX_LOGIN_TYPE: &str = "AtomicDEXLogin";

lazy_static! {
    static ref ADEX_TYPES: [ObjectType; 2] = adex_login_types();
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AtomicDEXDomain {
    pub(crate) name: String,
    url: String,
    version: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AtomicDEXLoginRequest {
    message: String,
}

impl AtomicDEXLoginRequest {
    pub fn new(name: String) -> AtomicDEXLoginRequest {
        AtomicDEXLoginRequest {
            message: format!("Login to {name}"),
        }
    }
}

fn adex_login_types() -> [ObjectType; 2] {
    let mut domain = ObjectType::domain();
    domain.property("name", PropertyType::String);
    domain.property("url", PropertyType::String);
    domain.property("version", PropertyType::String);

    let mut login_request = ObjectType::new(ADEX_LOGIN_TYPE);
    login_request.property("message", PropertyType::String);

    vec![domain, login_request]
}
