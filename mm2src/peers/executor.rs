// "It is the executor’s job to call `poll` on the task until `Ready(())` is returned."
// -- https://tokio.rs/docs/internals/runtime-model/

// Invoked from HTTP server the helpers will enjoy full native support for futures and threads.
// The portable code, on the other hand, will need this module
// in order to work with futures without the native threads and I/O.

// TODO: The portable executor should be available under `CORE.spawn`.

fn spawn () {

}
