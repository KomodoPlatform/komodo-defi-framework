#[cfg(not(target_arch = "wasm32"))] use mm2_main::mm2::mm2_main;

const MM_VERSION: &str = env!("MM_VERSION");
const MM_DATETIME: &str = env!("MM_DATETIME");

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        mm2_main(MM_VERSION.into(), MM_DATETIME.into())
    }
}
