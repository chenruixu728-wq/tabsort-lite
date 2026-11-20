use crate::protocol::{read_frame, write_frame, HOST};
use anyhow::{bail, Result};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::FromRawHandle;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{LocalFree, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES,
};

fn name() -> String {
    if let Ok(test) = std::env::var("TABSORT_TEST_PIPE") {
        return format!(r"\\.\pipe\tabsort-lite-test-{test}");
    }
    // Session isolation prevents one logged-in Windows user from blocking another.
    let user = std::env::var("USERNAME").unwrap_or_default();
    format!(r"\\.\pipe\tabsort-lite-v1-{user}")
}

pub fn accept() -> io::Result<File> {
    let pipe_name: Vec<u16> = name().encode_utf16().chain([0]).collect();
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)".encode_utf16().chain([0]).collect();
    let mut descriptor = null_mut();
    // The pipe is local-only, and its DACL grants access to its owner.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = CreateNamedPipeW(
            pipe_name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            65536,
            65536,
            0,
            &attributes,
        );
        let creation_error = io::Error::last_os_error();
        LocalFree(descriptor);
        if handle == INVALID_HANDLE_VALUE {
            return Err(creation_error);
        }
        let file = File::from_raw_handle(handle);
        if ConnectNamedPipe(handle, null_mut()) == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                return Err(error);
            }
        }
        Ok(file)
    }
}

pub fn extension_origin() -> &'static str {
    concat!(
        "chrome-extension://",
        include_str!("../extension/id.txt"),
        "/"
    )
}

pub fn host(origin: &str) -> Result<()> {
    if origin.trim_end_matches('/') != extension_origin().trim_end_matches('/') {
        bail!("扩展身份不匹配：{HOST}");
    }
    let mut desktop = OpenOptions::new().read(true).write(true).open(name())?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    write_frame(&mut desktop, &read_frame(&mut input)?)?;
    loop {
        let command = read_frame(&mut desktop)?;
        write_frame(&mut output, &command)?;
        let response = read_frame(&mut input)?;
        write_frame(&mut desktop, &response)?;
    }
}
