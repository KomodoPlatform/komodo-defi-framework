//! The wasm plug functions.

// pub fn host_ensure_dir_is_writable(ptr: *const c_char, len: i32) -> i32;
export function host_ensure_dir_is_writable(ptr, len) {
}

// pub fn host_env(name: *const c_char, nameˡ: i32, rbuf: *mut c_char, rcap: i32) -> i32;
export function host_env(name, name2, rbuf, rcap) {
    return 0;
}

// pub fn host_slurp(path_p: *const c_char, path_l: i32, rbuf: *mut c_char, rcap: i32) -> i32;
export function host_slurp(path_p, path_l, rbuf, rcap) {
    return 0;
}

// pub fn temp_dir(rbuf: *mut c_char, rcap: i32) -> i32;
export function temp_dir(rbuf, rcap) {
    return 0;
}

// pub fn host_rm(ptr: *const c_char, len: i32) -> i32;
export function host_rm(ptr, len) {
    return 0;
}

// pub fn host_write(path_p: *const c_char, path_l: i32, ptr: *const c_char, len: i32) -> i32;
export function host_write(path_p, path_l, ptr, len) {
    return 0;
}

// pub fn host_read_dir(path_p: *const c_char, path_l: i32, rbuf: *mut c_char, rcap: i32) -> i32;
export function host_read_dir(path_p, path_l, rbuf, rcap) {
    return 0;
}

// fn http_helper_if(helper: *const u8, helper_len: i32, payload: *const u8, payload_len: i32, timeout_ms: i32) -> i32;
export function http_helper_if(helper, helper_len, payload, payload_len, timeout_ms) {
}

// pub fn http_helper_check(helper_request_id: i32, rbuf: *mut u8, rcap: i32) -> i32;
export function http_helper_check(helper_request_id, rbuf, rcap) {
    return 0;
}

// pub fn call_back(cb_id: i32, ptr: *const c_char, len: i32);
export function call_back(cb_id, ptr, len) {
}

// fn sleep(ms: u32) -> Promise;
export function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
