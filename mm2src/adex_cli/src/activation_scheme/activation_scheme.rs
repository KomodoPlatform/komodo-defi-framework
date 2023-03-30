use log::{debug, error, warn};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::mem::swap;
// use serde::Serialize;
use crate::activation_scheme::{activation_scheme, get_activation_scheme_path};
use crate::helpers::read_json_file;
use serde_json::Value as Json;
use thiserror::Error;

// #[derive(Serialize)]
// pub struct EnableActivation {}
//
// #[derive(Serialize)]
// pub struct ElectrumActivation {}
//
// #[derive(Serialize)]
// pub enum ActivationMethod {
//     Enable(EnableActivation),
//     Electrum(ElectrumActivation),
// }
//

pub trait ActivationScheme {
    type ActivationCommand;
    fn get_activation_command(&self, coin: &str) -> Option<&Self::ActivationCommand>;
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
        let Json::Array(mut results) = Self::load_json_file().unwrap() else {
            return new;
        };

        let mut scheme: HashMap<String, Json> = results.iter_mut().map(Self::get_coin_pair).collect();

        swap(&mut new.scheme, &mut scheme);
        // for (k, v) in new.scheme.iter() {
        //     if *k == "KMD" {
        //         continue;
        //     }
        //     println!("{}:{}", *k, *v);
        // }
        new
    }

    fn get_coin_pair(element: &mut Json) -> (String, Json) {
        let Ok(result) = Self::get_coin_pair_impl(element) else {
            warn!("Failed to process: {element}");
            return ("".to_string(), Json::Null)
        };
        result
    }

    fn get_coin_pair_impl(element: &mut Json) -> Result<(String, Json), ()> {
        let mut temp = Json::Null;
        let command = element.get_mut("command").ok_or(())?;
        swap(&mut temp, command);
        let coin = element.get("coin").ok_or(())?.as_str().ok_or(())?.to_string();
        Ok((coin, temp))
    }

    fn load_json_file() -> Result<Json, ()> {
        let activation_scheme_path = get_activation_scheme_path()?;
        debug!("Start reading activation_scheme from: {activation_scheme_path:?}");

        let activation_scheme = read_json_file(&activation_scheme_path)?;

        let results = activation_scheme
            .get("results")
            .map(|results| results.clone())
            .ok_or_else(|| error!("Failed to get results section"))?;

        Ok(results)
    }
}

impl ActivationScheme for ActivationSchemeJson {
    type ActivationCommand = Json;
    fn get_activation_command(&self, coin: &str) -> Option<&Self::ActivationCommand> { self.scheme.get(coin) }

    fn get_coins_list(&self) -> Vec<String> { return vec!["".to_string()] }
}

pub fn get_activation_scheme() -> Box<dyn ActivationScheme<ActivationCommand = Json>> {
    let activation_scheme: Box<dyn ActivationScheme<ActivationCommand = Json>> = Box::new(ActivationSchemeJson::new());
    activation_scheme
}
