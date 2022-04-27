fn main() {
    let mut prost = prost_build::Config::new();
    prost.out_dir("utxo");
    prost.compile_protos(&["utxo/bchrpc.proto"], &["utxo"]).unwrap();

    tonic_build::configure()
        .build_server(false)
        .compile(&["z_coin/service.proto"], &["z_coin"])
        .unwrap();
}
