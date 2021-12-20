#[allow(dead_code)]
const PROTOS: [&str; 4] = [
    "proto/messages.proto",
    "proto/messages-common.proto",
    "proto/messages-management.proto",
    "proto/messages-bitcoin.proto",
];

fn main() {
    // prost_build::compile_protos(&PROTOS, &["proto"]).unwrap();
}
