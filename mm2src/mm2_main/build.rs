fn main() {
    let mut prost = prost_build::Config::new();
    prost.out_dir("src/lp_swap");
    prost
        .compile_protos(&["src/lp_swap/swap_v2.proto"], &["src/lp_swap"])
        .unwrap();
}
