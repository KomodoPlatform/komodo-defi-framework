#[allow(dead_code)]
const PROTOS: [&str; 6] = [
    "proto/messages.proto",
    "proto/messages-common.proto",
    "proto/messages-management.proto",
    "proto/messages-bitcoin.proto",
    "proto/messages-ethereum-definitions.proto",
    "proto/messages-ethereum.proto",
];

/// Note this builder is not used and .proto files are just for info of message layouts.
/// Instead message structs are created manually and auto derivation macro prost::Message was added
fn main() {
    // prost_build::compile_protos(&PROTOS, &["proto"]).unwrap();
}
