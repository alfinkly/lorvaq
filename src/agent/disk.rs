//! Used and total bytes of the filesystem under a path: the agent's own root is an overlay
//! on docker's data partition, which is what fills up on runner hosts.

use crate::scenes::ci::input::DiskUsage;
use std::path::Path;

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // the statvfs field types differ between platforms
pub fn usage(path: &Path) -> Option<DiskUsage> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs only writes into the zeroed struct we own; the path is NUL-terminated.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    let total = st.f_blocks as u64 * block;
    let free = st.f_bfree as u64 * block;
    (total > 0).then(|| DiskUsage {
        used_bytes: total.saturating_sub(free),
        total_bytes: total,
    })
}

#[cfg(not(unix))]
pub fn usage(_path: &Path) -> Option<DiskUsage> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_root_has_a_size() {
        let d = super::usage(std::path::Path::new("/")).unwrap();
        assert!(d.total_bytes > 0 && d.used_bytes <= d.total_bytes);
        assert!(super::usage(std::path::Path::new("/no/such/path")).is_none());
    }
}
