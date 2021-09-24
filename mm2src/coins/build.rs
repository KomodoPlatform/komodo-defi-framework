fn main() { tonic_build::compile_protos("utxo/bchrpc.proto").unwrap(); }
