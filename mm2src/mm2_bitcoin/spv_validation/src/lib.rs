extern crate chain;
extern crate primitives;
extern crate ripemd160;
extern crate rustc_hex as hex;
extern crate serialization;
extern crate sha2;
extern crate test_helpers;

/// `helpers_validation` Override function modules from bitcoin_spv and adapt for our mm2_bitcoin library
pub mod helpers_validation;

/// `spv_proof` Contains spv proof validation logic and data structure
pub mod spv_proof;

#[cfg(test)]
pub mod test_utils {
    extern crate serde;
    extern crate std;

    use self::serde::Deserialize;

    use std::{fs::File, io::Read, panic, string::String, vec, vec::Vec};

    #[derive(Deserialize, Debug)]
    pub struct TestCase {
        pub input: serde_json::Value,
        pub output: serde_json::Value,
        pub error_message: serde_json::Value,
    }

    pub fn setup() -> serde_json::Value {
        let mut file = File::open("./src/for_tests/spvTestVectors.json").unwrap();
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();

        serde_json::from_str(&data).unwrap()
    }

    pub fn to_test_case(val: &serde_json::Value) -> TestCase {
        let o = val.get("output");
        let output: &serde_json::Value;
        output = match o {
            Some(v) => v,
            None => &serde_json::Value::Null,
        };

        let e = val.get("rustError");
        let error_message: &serde_json::Value;
        error_message = match e {
            Some(v) => v,
            None => &serde_json::Value::Null,
        };

        TestCase {
            input: val.get("input").unwrap().clone(),
            output: output.clone(),
            error_message: error_message.clone(),
        }
    }

    pub fn get_test_cases(name: &str, fixtures: &serde_json::Value) -> Vec<TestCase> {
        let vals: &Vec<serde_json::Value> = fixtures.get(name).unwrap().as_array().unwrap();
        let mut cases = vec![];
        for i in vals {
            cases.push(to_test_case(&i));
        }
        cases
    }

    pub fn run_test<T>(test: T)
    where
        T: FnOnce(&serde_json::Value) -> () + panic::UnwindSafe,
    {
        let fixtures = setup();

        let result = panic::catch_unwind(|| test(&fixtures));

        assert!(result.is_ok())
    }
}
