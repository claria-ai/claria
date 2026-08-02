//! Atomic, private local writes selected by the desktop user.

use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

use eyre::{Result, WrapErr, eyre};
use uuid::Uuid;

/// Write a local export through a restrictive temporary file in the same
/// directory, flush it, and atomically rename it over the selected path.
///
/// The path is deliberately absent from errors and logs because it may itself
/// contain identifying information.
pub fn write_private_atomic(destination: &Path, bytes: &[u8]) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| eyre!("the selected export path has no parent directory"))?;
    let filename = destination
        .file_name()
        .ok_or_else(|| eyre!("the selected export path has no filename"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        filename.to_string_lossy(),
        Uuid::new_v4()
    ));

    let result = (|| -> Result<()> {
        let mut file = create_private_new(&temporary)?;
        file.write_all(bytes)
            .wrap_err("could not write the complete export")?;
        file.flush().wrap_err("could not flush the export")?;
        file.sync_all()
            .wrap_err("could not sync the export to local storage")?;
        drop(file);

        atomic_replace(&temporary, destination)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(destination, fs::Permissions::from_mode(0o600))
                .wrap_err("could not restrict exported document permissions")?;
        }
        Ok(())
    })();

    if result.is_err()
        && let Err(cleanup_error) = fs::remove_file(&temporary)
        && cleanup_error.kind() != std::io::ErrorKind::NotFound
    {
        // The primary error remains actionable. This warning contains no path.
        tracing::warn!(error = %cleanup_error, "could not remove failed local export temporary file");
    }
    result
}

#[cfg(unix)]
fn create_private_new(path: &Path) -> Result<File> {
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .wrap_err("could not create a private temporary export file")
}

#[cfg(windows)]
fn create_private_new(path: &Path) -> Result<File> {
    use std::{
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL},
    };

    // Protected DACL: file owner, LocalSystem, and local administrators only.
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;BA)\0"
        .encode_utf16()
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `sddl` and output pointers are valid for this call. The returned
    // descriptor is released with LocalFree after CreateFileW consumes it.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(std::io::Error::last_os_error())
            .wrap_err("could not construct private export permissions");
    }

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: pointers refer to live, NUL-terminated storage and a valid
    // SECURITY_ATTRIBUTES value. CREATE_NEW prevents accidental truncation.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_WRITE,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    // SAFETY: descriptor came from the conversion API above.
    unsafe {
        LocalFree(descriptor);
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .wrap_err("could not create a private temporary export file");
    }
    // SAFETY: CreateFileW returned a uniquely owned valid handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

#[cfg(not(any(unix, windows)))]
fn create_private_new(_path: &Path) -> Result<File> {
    Err(eyre!(
        "private local exports are not supported on this operating system"
    ))
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).wrap_err("could not atomically place the exported document")
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both path buffers are live and NUL-terminated for the call.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
            .wrap_err("could not atomically place the exported document")
    } else {
        Ok(())
    }
}
