extern crate chain;
extern crate primitives;
extern crate ripemd160;
extern crate rustc_hex as hex;
extern crate serialization;
extern crate sha2;

/// `types` exposes simple types for on-chain evaluation of SPV proofs
pub mod types;

/// `helpers_validation` Override function modules from bitcoin_spv and adapt for our mm2_bitcoin library
pub mod helpers_validation;

/// `spv_proof` Contains spv proof validation logic and data structure
pub mod spv_proof;

#[cfg(test)]
pub mod test_utils {
    extern crate hex;
    extern crate serde;
    extern crate std;

    use self::serde::Deserialize;

    use std::{fs::File, io::Read, panic, string::String, vec, vec::Vec};

    /// Strips the '0x' prefix off of hex string so it can be deserialized.
    ///
    /// # Arguments
    ///
    /// * `s` - The hex str
    pub fn strip_0x_prefix(s: &str) -> &str {
        if &s[..2] == "0x" {
            &s[2..]
        } else {
            s
        }
    }

    /// Deserializes a hex string into a u8 array.
    ///
    /// # Arguments
    ///
    /// * `s` - The hex string
    pub fn deserialize_hex(s: &str) -> Result<Vec<u8>, hex::FromHexError> { hex::decode(&strip_0x_prefix(s)) }

    /// Deserialize a hex string into bytes.
    /// Panics if the string is malformatted.
    ///
    /// # Arguments
    ///
    /// * `s` - The hex string
    ///
    /// # Panics
    ///
    /// When the string is not validly formatted hex.
    pub fn force_deserialize_hex(s: &str) -> Vec<u8> { deserialize_hex(s).unwrap() }

    #[derive(Deserialize, Debug)]
    pub struct TestCase {
        pub input: serde_json::Value,
        pub output: serde_json::Value,
        pub error_message: serde_json::Value,
    }

    pub fn setup() -> serde_json::Value {
        let mut file = File::open("../for_tests/spvTestVectors.json").unwrap();
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

    #[test]
    fn it_strips_0x_prefixes() {
        let cases = [
            ("00", "00"),
            ("0x00", "00"),
            ("aa", "aa"),
            ("0xaa", "aa"),
            ("Quotidian", "Quotidian"),
            ("0xQuotidian", "Quotidian"),
        ];
        for case in cases.iter() {
            assert_eq!(strip_0x_prefix(case.0), case.1);
        }
    }
}
