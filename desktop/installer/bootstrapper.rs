#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

static MSI: &[u8] = include_bytes!(env!("CURSORCUE_MSI_PATH"));

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[link(name = "ole32")]
extern "system" {
    fn CoCreateGuid(guid: *mut Guid) -> i32;
}

#[link(name = "msi")]
extern "system" {
    fn MsiSetInternalUI(level: u32, window: *mut isize) -> u32;
    fn MsiInstallProductW(package: *const u16, command: *const u16) -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(window: isize, text: *const u16, title: *const u16, flags: u32) -> i32;
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn message(text: &str, flags: u32) -> i32 {
    let text = wide(OsStr::new(text));
    let title = wide(OsStr::new("CursorCue setup"));
    unsafe { MessageBoxW(0, text.as_ptr(), title.as_ptr(), flags) }
}

fn install(quiet: bool, arguments: &[String]) -> Result<u32, &'static str> {
    let mut command = String::from("REBOOT=ReallySuppress");
    for argument in arguments {
        if argument.eq_ignore_ascii_case("/quiet") {
            continue;
        }
        let (key, value) = argument.split_once('=').ok_or("Unknown setup option.")?;
        if !matches!(key, "INSTALLDIR" | "SHORTCUTDIR")
            || !PathBuf::from(value).is_absolute()
            || value.contains(['"', '\0', '\r', '\n'])
        {
            return Err("Setup accepts only absolute INSTALLDIR and SHORTCUTDIR paths.");
        }
        command.push_str(&format!(" {key}=\"{value}\""));
    }

    let mut guid = Guid {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0; 8],
    };
    if unsafe { CoCreateGuid(&mut guid) } < 0 {
        return Err("Setup could not create a temporary package name.");
    }
    let suffix = format!(
        "{:08X}-{:04X}-{:04X}-{}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>()
    );
    let directory = std::env::temp_dir().join(format!("CursorCueSetup-{suffix}"));
    fs::create_dir(&directory).map_err(|_| "Setup could not create its temporary directory.")?;
    let package = directory.join("CursorCue.msi");
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&package)
            .map_err(|_| "Setup could not create its temporary package.")?;
        file.write_all(MSI)
            .map_err(|_| "Setup could not write its temporary package.")?;
        drop(file);
        let package_wide = wide(package.as_os_str());
        let command_wide = wide(OsStr::new(&command));
        unsafe {
            MsiSetInternalUI(if quiet { 2 } else { 5 }, ptr::null_mut());
            Ok(MsiInstallProductW(
                package_wide.as_ptr(),
                command_wide.as_ptr(),
            ))
        }
    })();
    let _ = fs::remove_file(&package);
    let _ = fs::remove_dir(&directory);
    result
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let quiet = arguments
        .iter()
        .any(|arg| arg.eq_ignore_ascii_case("/quiet"));
    let code = match install(quiet, &arguments) {
        Ok(code) => {
            if !quiet && code != 0 && code != 3010 && code != 1602 {
                message(
                    &format!("Windows Installer could not install CursorCue. Error code: {code}."),
                    0x10,
                );
            }
            code
        }
        Err(error) => {
            if !quiet {
                message(error, 0x10);
            }
            1603
        }
    };
    std::process::exit(code as i32);
}
