use libc::c_char;
use std::ffi::CStr;
use std::net::IpAddr;

// Example of a function forward:
//lazy_static! {pub static ref PEERS_SEND_COMPAT: Mutex<Option<fn (u32, i32, *const u8, i32) -> i32>> = Mutex::new (None);}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn log_stacktrace(desc: *const c_char) {
    let desc = if desc.is_null() {
        ""
    } else {
        match CStr::from_ptr(desc).to_str() {
            Ok(s) => s,
            Err(err) => {
                log!("log_stacktrace] Bad trace description: {}", err);
                ""
            },
        }
    };
    let mut trace = String::with_capacity(4096);
    super::stack_trace(&mut super::stack_trace_frame, &mut |l| trace.push_str(l));
    log!("Stacktrace. {}\n{}", desc, trace);
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn is_loopback_ip(ip: *mut c_char) -> u8 {
    if ip.is_null() {
        log!("received null ip");
        return 0;
    }

    let ip_str = match CStr::from_ptr(ip).to_str() {
        Ok(s) => s,
        Err(e) => {
            log!("Error creating CStr {:?}", e);
            return 0;
        },
    };

    let ip: IpAddr = match ip_str.parse() {
        Ok(ip) => ip,
        Err(e) => {
            log!("Error {:?} parsing ip from str {}", e, ip_str);
            return 0;
        },
    };

    ip.is_loopback() as u8
}
