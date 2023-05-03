use common::log::{error, info};
use std::env;
use std::path::PathBuf;

#[cfg(not(target_os = "macos"))]
pub use sysinfo::{PidExt, ProcessExt, System, SystemExt};

#[cfg(windows)]
mod reexport {
    pub use std::ffi::CString;
    pub use std::mem;
    pub use std::mem::size_of;
    pub use std::ptr::null;
    pub use std::u32;
    pub use winapi::um::processthreadsapi::{CreateProcessA, OpenProcess, TerminateProcess, PROCESS_INFORMATION,
                                            STARTUPINFOA};
    pub use winapi::um::winnt::{PROCESS_TERMINATE, SYNCHRONIZE};

    pub const MM2_BINARY: &str = "mm2.exe";
}

#[cfg(windows)] use reexport::*;

#[cfg(all(unix, not(target_os = "macos")))]
mod unix_not_macos_reexport {
    pub use std::process::{Command, Stdio};

    pub const KILL_CMD: &str = "kill";
}

#[cfg(all(unix, not(target_os = "macos")))]
use unix_not_macos_reexport::*;

#[cfg(unix)]
mod unix_reexport {
    pub const MM2_BINARY: &str = "mm2";
}

#[cfg(unix)] use unix_reexport::*;

#[cfg(target_os = "macos")]
mod macos_reexport {
    pub use std::fs;
    pub const LAUNCH_CTL_COOL_DOWN_TIMEOUT_MS: u64 = 500;
    pub use common::log::debug;
    pub use std::process::{Command, Stdio};
    pub use std::thread::sleep;
    pub use std::time::Duration;
    pub const LAUNCHCTL_MM2_ID: &str = "com.mm2.daemon";
}

#[cfg(target_os = "macos")] use macos_reexport::*;

#[cfg(not(target_os = "macos"))]
pub fn get_status() {
    let pids = find_proc_by_name(MM2_BINARY);
    if pids.is_empty() {
        info!("Process not found: {MM2_BINARY}");
    }
    pids.iter().map(u32::to_string).for_each(|pid| {
        info!("Found {MM2_BINARY} is running, pid: {pid}");
    });
}

#[cfg(not(target_os = "macos"))]
fn find_proc_by_name(pname: &'_ str) -> Vec<u32> {
    let s = System::new_all();

    s.processes()
        .iter()
        .filter(|(_, process)| process.name() == pname)
        .map(|(pid, _)| pid.as_u32())
        .collect()
}

fn get_mm2_binary_path() -> Result<PathBuf, ()> {
    let mut dir = env::current_exe().map_err(|error| {
        error!("Failed to get current binary dir: {error}");
    })?;
    dir.pop();
    dir.push(MM2_BINARY);
    Ok(dir)
}

#[cfg(not(target_os = "macos"))]
pub fn start_process(mm2_cfg_file: &Option<String>, coins_file: &Option<String>, log_file: &Option<String>) {
    if let Some(mm2_cfg_file) = mm2_cfg_file {
        info!("Set env MM_CONF_PATH as: {mm2_cfg_file}");
        env::set_var("MM_CONF_PATH", mm2_cfg_file);
    }
    if let Some(coins_file) = coins_file {
        info!("Set env MM_COINS_PATH as: {coins_file}");
        env::set_var("MM_COINS_PATH", coins_file);
    }
    if let Some(log_file) = log_file {
        info!("Set env MM_LOG as: {log_file}");
        env::set_var("MM_LOG", log_file);
    }

    let Ok(mm2_binary) = get_mm2_binary_path() else { return; };
    if !mm2_binary.exists() {
        error!("Failed to start mm2, no file: {mm2_binary:?}");
        return;
    }
    start_process_impl(mm2_binary);
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn start_process_impl(mm2_binary: PathBuf) {
    let mut command = Command::new(&mm2_binary);
    let file_name = mm2_binary.file_name().expect("No file_name in mm2_binary");
    let process = match command.stdout(Stdio::null()).stdout(Stdio::null()).spawn() {
        Ok(process) => process,
        Err(error) => {
            error!("Failed to start process: {mm2_binary:?}, error: {error}");
            return;
        },
    };
    let pid = process.id();
    std::mem::forget(process);
    info!("Started child process: {file_name:?}, pid: {pid}");
}

#[cfg(windows)]
pub fn start_process_impl(mm2_binary: PathBuf) {
    let Some(program) = mm2_binary.to_str() else {
        error!("Failed to cast mm2_binary to &str");
        return;
    };
    let program = match CString::new(program) {
        Ok(program) => program,
        Err(error) => {
            error!("Failed to construct CString program path: {error}");
            return;
        },
    };

    let mut startup_info: STARTUPINFOA = unsafe { mem::zeroed() };
    startup_info.cb = size_of::<STARTUPINFOA>() as u32;
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

    let result = unsafe {
        CreateProcessA(
            null(),
            program.into_raw() as *mut i8,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut startup_info,
            &mut process_info,
        )
    };

    match result {
        0 => error!("Failed to start: {MM2_BINARY}"),
        _ => info!("Successfully started: {MM2_BINARY}"),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn stop_process() {
    let pids = find_proc_by_name(MM2_BINARY);
    if pids.is_empty() {
        info!("Process not found: {MM2_BINARY}");
    }
    pids.iter().map(u32::to_string).for_each(|pid| {
        match Command::new(KILL_CMD)
            .arg(&pid)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) => {
                if status.success() {
                    info!("Process killed: {MM2_BINARY}:{pid}")
                } else {
                    error!("Failed to kill process: {MM2_BINARY}:{pid}")
                }
            },
            Err(e) => error!("Failed to kill process: {MM2_BINARY}:{pid}. Error: {e}"),
        };
    });
}

#[cfg(windows)]
pub fn stop_process() {
    let processes = find_proc_by_name(MM2_BINARY);
    for pid in processes {
        info!("Terminate process: {}", pid);
        unsafe {
            let handy = OpenProcess(SYNCHRONIZE | PROCESS_TERMINATE, true as i32, pid);
            TerminateProcess(handy, 1);
        }
    }
}

#[cfg(target_os = "macos")]
pub fn start_process(mm2_cfg_file: &Option<String>, coins_file: &Option<String>, log_file: &Option<String>) {
    let Ok(mm2_binary) = get_mm2_binary_path() else { return; };

    let Ok(current_dir) = env::current_dir() else {
	error!("Failed to get current_dir");
	return
    };

    let Ok(plist_path)  = get_plist_path() else {return;};

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key>
            <string>{}</string>
            <key>ProgramArguments</key>
            <array>
                <string>{}</string>
            </array>
            <key>WorkingDirectory</key>
            <string>{}</string>
            <key>EnvironmentVariables</key>
            <dict>{}{}{}</dict>
            <key>RunAtLoad</key>
            <false/>
            <key>KeepAlive</key>
            <false/>
        </dict>
        </plist>"#,
        LAUNCHCTL_MM2_ID,
        mm2_binary.display(),
        current_dir.display(),
        log_file
            .as_deref()
            .map(|log_file| format!("<key>MM_LOG</key><string>{log_file}</string>"))
            .unwrap_or_default(),
        mm2_cfg_file
            .as_deref()
            .map(|cfg_file| format!("<key>MM_CONF_PATH</key><string>{cfg_file}</string>"))
            .unwrap_or_default(),
        coins_file
            .as_deref()
            .map(|coins_file| format!("<key>MM_COINS_PATH</key><string>{coins_file}</string>"))
            .unwrap_or_default(),
    );

    if let Err(error) = fs::write(&plist_path, plist) {
        error!("Failed to write plist file: {error}");
        return;
    }

    match Command::new("launchctl")
        .arg("enable")
        .arg(format!("system/{LAUNCHCTL_MM2_ID}").as_str())
        .spawn()
    {
        Ok(_) => debug!("Successfully enabled using launchctl, label: {LAUNCHCTL_MM2_ID}"),
        Err(error) => error!("Failed to enable process: {error}"),
    }

    match Command::new("launchctl").arg("load").arg(&plist_path).spawn() {
        Ok(_) => debug!("Successfully loaded using launchctl, label: {LAUNCHCTL_MM2_ID}"),
        Err(error) => error!("Failed to load process: {error}"),
    }

    match Command::new("launchctl").args(["start", LAUNCHCTL_MM2_ID]).spawn() {
        Ok(_) => info!("Successfully started using launchctl, label: {LAUNCHCTL_MM2_ID}"),
        Err(error) => error!("Failed to start process: {error}"),
    }
}

#[cfg(target_os = "macos")]
fn get_plist_path() -> Result<PathBuf, ()> {
    match env::current_dir() {
        Err(error) => {
            error!("Failed to get current_dir to construct plist_path: {error}");
            Err(())
        },
        Ok(mut current_dir) => {
            current_dir.push(&format!("{LAUNCHCTL_MM2_ID}.plist"));
            Ok(current_dir)
        },
    }
}

#[cfg(target_os = "macos")]
pub fn stop_process() {
    let Ok(plist_path) = get_plist_path() else { return; };

    if let Err(error) = Command::new("launchctl").arg("unload").arg(&plist_path).spawn() {
        error!("Failed to unload process using launchctl: {}", error);
    } else {
        info!("mm2 successfully stopped by launchctl");
    }
    sleep(Duration::from_millis(LAUNCH_CTL_COOL_DOWN_TIMEOUT_MS));
    if let Err(err) = fs::remove_file(&plist_path) {
        error!("Failed to remove plist file: {}", err);
    }
}

#[cfg(target_os = "macos")]
pub fn get_status() {
    let output = Command::new("launchctl")
        .args(["list", LAUNCHCTL_MM2_ID])
        .output()
        .unwrap();

    if !output.status.success() {
        info!("Service '{LAUNCHCTL_MM2_ID}' is not running");
        return;
    }

    if let Some(found) = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains("PID"))
        .last()
    {
        let pid = found
            .trim()
            .matches(char::is_numeric)
            .fold(String::default(), |mut pid, ch| {
                pid.push_str(ch);
                pid
            });
        info!("Service '{LAUNCHCTL_MM2_ID}' is running under launchctl, pid: {}", pid);
    } else {
        info!("Service '{LAUNCHCTL_MM2_ID}' is not running");
    };
}
