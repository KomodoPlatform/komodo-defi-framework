fn main() {
    let mut prost = prost_build::Config::new();
    prost.out_dir("utxo");
    prost.compile_protos(&["utxo/bchrpc.proto"], &["utxo"]).unwrap();
}
