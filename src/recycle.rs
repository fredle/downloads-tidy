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

const FO_DELETE: u32 = 3;
const FOF_SILENT: u16 = 0x0004;
const FOF_NOCONFIRMATION: u16 = 0x0010;
const FOF_ALLOWUNDO: u16 = 0x0040;
const FOF_NOERRORUI: u16 = 0x0400;

pub fn send(path: &Path) -> Result<(), String> {
    // pFrom is a list of paths, each NUL terminated, ended by an extra NUL.
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);
    let mut op = ShFileOpStructW {
        hwnd: 0,
        func: FO_DELETE,
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI,
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
    #[test]
    fn recycles_a_real_file() {
        let p = std::env::temp_dir().join(format!("dtidy-recycle-{}.txt", std::process::id()));
        std::fs::write(&p, b"bye").unwrap();
        super::send(&p).unwrap();
        assert!(!p.exists());
    }
}
