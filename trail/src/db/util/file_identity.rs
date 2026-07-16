use super::*;
use std::os::windows::ffi::OsStrExt;
use winapi::um::fileapi::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, OPEN_EXISTING,
};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::winbase::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};
use winapi::um::winnt::{
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowsFileIdentity {
    pub(crate) attributes: u32,
    pub(crate) volume_serial_number: u32,
    pub(crate) file_index: u64,
    pub(crate) number_of_links: u32,
    pub(crate) creation_time: u64,
    pub(crate) last_write_time: u64,
}

pub(crate) fn windows_file_identity(path: &Path) -> Result<WindowsFileIdentity> {
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let succeeded = unsafe { GetFileInformationByHandle(handle, &mut information) } != 0;
    let get_error = (!succeeded).then(std::io::Error::last_os_error);
    let close_result = unsafe { CloseHandle(handle) };
    if let Some(error) = get_error {
        return Err(Error::Io(error));
    }
    if close_result == 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let filetime = |high: u32, low: u32| (u64::from(high) << 32) | u64::from(low);
    Ok(WindowsFileIdentity {
        attributes: information.dwFileAttributes,
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: filetime(information.nFileIndexHigh, information.nFileIndexLow),
        number_of_links: information.nNumberOfLinks,
        creation_time: filetime(
            information.ftCreationTime.dwHighDateTime,
            information.ftCreationTime.dwLowDateTime,
        ),
        last_write_time: filetime(
            information.ftLastWriteTime.dwHighDateTime,
            information.ftLastWriteTime.dwLowDateTime,
        ),
    })
}
