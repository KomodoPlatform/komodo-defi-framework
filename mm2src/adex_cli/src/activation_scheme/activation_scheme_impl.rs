use log::{debug, error, warn};
use serde_json::{json, Value as Json};
use std::collections::HashMap;

use super::get_activation_scheme_path;
use crate::helpers::read_json_file;

pub trait ActivationScheme {
    type ActivationCommand;
    fn get_activation_method(&self, coin: &str) -> Option<Self::ActivationCommand>;
    fn get_coins_list(&self) -> Vec<String>;
}

struct ActivationSchemeJson {
    scheme: HashMap<String, Json>,
}

impl ActivationSchemeJson {
    fn new() -> Self {
        let mut new = Self {
            scheme: HashMap::<String, Json>::new(),
        };

        let Ok(Json::Array(mut results)) = Self::load_json_file() else {
            return new;
        };
        new.scheme = results.iter_mut().map(Self::get_coin_pair).collect();
        new
    }

    fn get_coin_pair(element: &mut Json) -> (String, Json) {
        let presence = element.to_string();
        let Ok(result) = Self::get_coin_pair_impl(element) else {
            warn!("Failed to process: {presence}");
            return ("".to_string(), Json::Null)
        };
        result
    }

    fn get_coin_pair_impl(element: &mut Json) -> Result<(String, Json), ()> {
        let command = element.get_mut("command").ok_or(())?.take();
        let coin = element.get("coin").ok_or(())?.as_str().ok_or(())?.to_string();
        Ok((coin, command))
    }

    fn load_json_file() -> Result<Json, ()> {
        let activation_scheme_path = get_activation_scheme_path()?;
        debug!("Start reading activation_scheme from: {activation_scheme_path:?}");

        let mut activation_scheme: Json = read_json_file(&activation_scheme_path)?;

        let results = activation_scheme
            .get_mut("results")
            .ok_or_else(|| error!("Failed to get results section"))?
            .take();

        Ok(results)
    }
}

impl ActivationScheme for ActivationSchemeJson {
    type ActivationCommand = Json;
    fn get_activation_method(&self, coin: &str) -> Option<Self::ActivationCommand> {
        let Some(Json::Object(object)) = self.scheme.get(coin) else { return None };
        let mut copy = json!({});
        for (k, v) in object.iter() {
            // WORKAROUND: serde_json::Value does not support removing key
            if *k == "userpass" {
                continue;
            }
            copy[k] = v.clone();
        }
        Some(copy)
    }

    fn get_coins_list(&self) -> Vec<String> { vec!["".to_string()] }
}

pub fn get_activation_scheme() -> Box<dyn ActivationScheme<ActivationCommand = Json>> {
    let activation_scheme: Box<dyn ActivationScheme<ActivationCommand = Json>> = Box::new(ActivationSchemeJson::new());
    activation_scheme
}
