//! Owner-only files on Windows (spec §6.2): the grant store lives under
//! `%LOCALAPPDATA%\oab-imcp-winpoc` with a protected DACL granting only the current
//! user and SYSTEM, inheritance disabled — the Windows counterpart of the Linux
//! 0700/0600 handling, so reverse-attach grants (which hold the attach secret) are not
//! readable by other standard users.

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// `%LOCALAPPDATA%\oab-imcp-winpoc` as the state root (POC name, spec §13.2).
pub(crate) fn state_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("oab-imcp-winpoc"))
}

fn last_error() -> u32 {
    // SAFETY: no arguments.
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

/// The current user's SID as a string (e.g. `S-1-5-21-…`).
fn current_user_sid() -> Result<String, String> {
    // SAFETY: querying our own process token; handles are closed below.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(format!("OpenProcessToken failed ({})", last_error()));
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        windows_sys::Win32::Foundation::CloseHandle(token);
        if ok == 0 {
            return Err(format!("GetTokenInformation failed ({})", last_error()));
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut str_sid: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut str_sid) == 0 {
            return Err(format!("ConvertSidToStringSidW failed ({})", last_error()));
        }
        let mut n = 0;
        while *str_sid.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(str_sid, n));
        LocalFree(str_sid as HLOCAL);
        Ok(s)
    }
}

/// Apply a protected DACL granting full control to the current user and SYSTEM only,
/// with inheritance disabled (`P`). File (`FA`) rights; applied to files and dirs.
fn apply_owner_only_dacl(path: &Path) -> Result<(), String> {
    let sid = current_user_sid()?;
    // PAI/P = protected (no inheritance from the parent); two Allow ACEs: the user and
    // SYSTEM (S-1-5-18), each full access, inheritable onto dir children (OICI).
    let sddl = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)");
    let sddl_w: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let path_w: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: NUL-terminated strings; out-pointers to locals; LocalFree frees the SD; the
    // DACL pointer is owned by the SD for the duration of the Set call.
    unsafe {
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            1, // SDDL_REVISION_1
            &mut sd,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(format!("building DACL failed ({})", last_error()));
        }
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
        let rc = SetNamedSecurityInfoW(
            path_w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null_mut(),
        );
        LocalFree(sd as HLOCAL);
        if rc != 0 {
            return Err(format!("SetNamedSecurityInfo failed ({rc})"));
        }
    }
    Ok(())
}

/// Whether `path`'s DACL is protected (inheritance disabled). A rough check that it has
/// been locked down; we re-apply unconditionally on read anyway.
fn dacl_is_protected(path: &Path) -> bool {
    let path_w: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: NUL-terminated string; out-pointers to locals; the returned SD is freed.
    unsafe {
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let rc = GetNamedSecurityInfoW(
            path_w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        );
        if rc != 0 {
            return false;
        }
        // The control bits live in the SD; SE_DACL_PROTECTED = 0x1000. Read the control
        // word via GetSecurityDescriptorControl.
        let mut control: u16 = 0;
        let mut revision: u32 = 0;
        let got = windows_sys::Win32::Security::GetSecurityDescriptorControl(
            sd,
            &mut control,
            &mut revision,
        );
        LocalFree(sd as HLOCAL);
        got != 0 && (control & 0x1000) != 0
    }
}

pub(crate) fn create_dir_all(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    apply_owner_only_dacl(dir).map_err(std::io::Error::other)
}

/// A new file only the current user and SYSTEM can read or write; fails if it exists.
pub(crate) fn create_new(path: &Path) -> std::io::Result<fs::File> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    apply_owner_only_dacl(path).map_err(std::io::Error::other)?;
    Ok(file)
}

/// If `path`'s DACL is not protected, re-lock it before trusting the file (the Windows
/// counterpart of narrowing a widened 0600 file on Linux).
pub(crate) fn narrow(path: &Path, _meta: &fs::Metadata) -> Result<(), String> {
    if !dacl_is_protected(path) {
        eprintln!(
            "grants: {} had an open DACL; re-applying owner-only access",
            path.display()
        );
        apply_owner_only_dacl(path)?;
    }
    Ok(())
}
