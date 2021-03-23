export function host_ensure_dir_is_writable(ptr, len) {
}

export function host_electrum_connect(ptr, len) {
    return 0;
}

export function host_electrum_is_connected(ri) {
    return 0;
}

export function host_electrum_request(ri, ptr, len) {
    return 0;
}

export function host_electrum_reply(ri, id, rbuf, rcap) {
    return 0;
}

export function host_env(name, name2, rbuf, rcap) {
    return 0;
}

// pub fn date_now() -> f64;
export function date_now() {
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

export function host_read_dir(path_p, path_l, rbuf, rcap) {
    return 0;
}

export function http_helper_if(helper, helper_len, payload, payload_len, timeout_ms) {
}

export function http_helper_check(helper_request_id, rbuf, rcap) {
    return 0;
}

export function call_back(cb_id, ptr, len) {
}

export function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
