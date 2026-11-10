//! Send a file to the Recycle Bin via the shell (so Explorer can restore it).

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

#[repr(C)]
struct ShFileOpStructW {
    hwnd: isize,
    func: u32,
    from: *const u16,
    to: *const u16,
    flags: u16,
    any_aborted: i32,
    name_mappings: *mut core::ffi::c_void,
    title: *const u16,
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHFileOperationW(op: *mut ShFileOpStructW) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
}

const FO_DELETE: u32 = 3;
const FOF_SILENT: u16 = 0x0004;
const FOF_NOCONFIRMATION: u16 = 0x0010;
const FOF_ALLOWUNDO: u16 = 0x0040;
const FOF_NOERRORUI: u16 = 0x0400;
/// Ask before destroying a file the Recycle Bin can't take (e.g. too big for
/// it), instead of FOF_NOCONFIRMATION silently deleting it for good.
const FOF_WANTNUKEWARNING: u16 = 0x4000;
const DRIVE_FIXED: u32 = 3;

/// The drive root of an absolute path, e.g. `C:\` or `\\server\share\`.
fn drive_root(path: &Path) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let mut it = path.components();
    match (it.next(), it.next()) {
        (Some(Component::Prefix(p)), Some(Component::RootDir)) => {
            Some(std::path::PathBuf::from(p.as_os_str()).join("\\"))
        }
        _ => None,
    }
}

/// Only local fixed drives have a Recycle Bin. On network shares and
/// removable drives the shell would delete the file permanently instead.
fn has_recycle_bin(path: &Path) -> bool {
    let Some(root) = drive_root(path) else { return false };
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call.
    unsafe { GetDriveTypeW(wide.as_ptr()) == DRIVE_FIXED }
}

/// Never deletes permanently: refuses on drives without a Recycle Bin, and
/// asks (failing if declined) when the bin can't take a file.
pub fn send(path: &Path) -> Result<(), String> {
    if !has_recycle_bin(path) {
        return Err("This drive has no Recycle Bin, so the file was left in place.".into());
    }
    // pFrom is a list of paths, each NUL terminated, ended by an extra NUL.
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);
    let mut op = ShFileOpStructW {
        hwnd: 0,
        func: FO_DELETE,
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_SILENT | FOF_NOERRORUI,
        any_aborted: 0,
        name_mappings: std::ptr::null_mut(),
        title: std::ptr::null(),
    };
    // SAFETY: `op` is fully initialised and `from` outlives the call.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.any_aborted != 0 {
        return Err(format!("Recycle Bin refused the file (code {code:#x})"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    #[test]
    fn drive_roots() {
        assert_eq!(super::drive_root(Path::new(r"C:\Users\me\Downloads\a.zip")), Some(PathBuf::from(r"C:\")));
        assert_eq!(
            super::drive_root(Path::new(r"\\nas\share\Downloads\a.zip")),
            Some(PathBuf::from(r"\\nas\share\"))
        );
        assert_eq!(super::drive_root(Path::new(r"Downloads\a.zip")), None);
    }

    #[test]
    fn refuses_where_there_is_no_recycle_bin() {
        // A UNC path is never a local fixed drive; must refuse, not delete.
        let err = super::send(Path::new(r"\\nonexistent-host\share\file.txt")).unwrap_err();
        assert!(err.contains("no Recycle Bin"));
    }

    #[test]
    fn recycles_a_real_file() {
        let p = std::env::temp_dir().join(format!("dtidy-recycle-{}.txt", std::process::id()));
        std::fs::write(&p, b"bye").unwrap();
        super::send(&p).unwrap();
        assert!(!p.exists());
    }
}
