//! Atomic, private local writes selected by the desktop user, and the
//! per-platform primitive that restricts a file to the account Claria runs as.

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
        // The temporary is already restricted and `rename` carries that
        // across, so this only restates the post-condition on the file the
        // user was handed. Windows is not repeated here on purpose: the DACL
        // goes on when the temporary is created (see `create_private_new`)
        // and travels with the same-directory rename, while re-applying it
        // afterwards would fail outright on a removable volume that has no
        // ACLs to set — turning a completed export into an error.
        #[cfg(unix)]
        set_private_permissions(destination)
            .wrap_err("could not restrict exported document permissions")?;
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

/// Restrict an existing file to the account Claria runs as: `0o600` on Unix,
/// the `PRIVATE_FILE_SDDL` access control list on Windows. Every platform
/// either applies the restriction or returns an error — none of them report a
/// restriction they did not make.
///
/// The path is deliberately absent from errors because it may itself contain
/// identifying information. Callers whose paths are Claria's own add it back.
#[cfg(unix)]
pub fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .wrap_err("could not restrict the file to the current user")
}

/// Windows has no permission bits, so the equivalent is to replace whatever
/// list the file inherited from its directory with the protected one.
#[cfg(windows)]
pub fn set_private_permissions(path: &Path) -> Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};

    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        Security::{
            Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW},
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        },
    };

    let security = PrivateFileSecurity::new()?;
    let list = security.access_control_list()?;
    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the path is a live, NUL-terminated buffer and `list` borrows
    // from `security`, which outlives the call. Only the DACL is replaced;
    // owner, group, and SACL are left alone by passing null for each.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            list,
            ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .wrap_err("could not restrict the file to the current user");
    }
    Ok(())
}

/// No third platform is supported. Claria refuses to write a file it cannot
/// restrict rather than pretending the restriction happened.
#[cfg(not(any(unix, windows)))]
pub fn set_private_permissions(_path: &Path) -> Result<()> {
    Err(eyre!(
        "restricting a file to the current user is not supported on this operating system"
    ))
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

/// Full control for the file's owner, `SYSTEM`, and the local
/// `Administrators` group, and no access at all for anyone else.
///
/// `D:P` protects the list, so permissive entries inherited from the
/// containing directory are dropped instead of merged in. `OW` is the Owner
/// Rights SID, which an access check resolves to whoever owns the file — the
/// account Claria runs as, since Claria created it. `FA` is `FILE_ALL_ACCESS`
/// rather than the generic `GA`, so the mask stored on the file never depends
/// on generic-to-specific rights mapping being applied for us.
#[cfg(windows)]
const PRIVATE_FILE_SDDL: &str = "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)\0";

/// [`PRIVATE_FILE_SDDL`] in the binary form the Win32 file APIs take, freed on
/// drop.
#[cfg(windows)]
struct PrivateFileSecurity(windows_sys::Win32::Security::PSECURITY_DESCRIPTOR);

#[cfg(windows)]
impl PrivateFileSecurity {
    fn new() -> Result<Self> {
        use std::ptr;

        use windows_sys::Win32::Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR,
        };

        let sddl: Vec<u16> = PRIVATE_FILE_SDDL.encode_utf16().collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is a live, NUL-terminated buffer and `descriptor` is
        // a valid out-pointer. On success the callee hands back a
        // LocalAlloc'd descriptor, which `Drop` releases exactly once.
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
                .wrap_err("could not construct private file permissions");
        }
        Ok(Self(descriptor))
    }

    fn descriptor(&self) -> windows_sys::Win32::Security::PSECURITY_DESCRIPTOR {
        self.0
    }

    /// The access control list inside the descriptor. The pointer borrows from
    /// `self` and must not outlive it.
    fn access_control_list(&self) -> Result<*const windows_sys::Win32::Security::ACL> {
        use std::ptr;

        use windows_sys::{Win32::Security::GetSecurityDescriptorDacl, core::BOOL};

        let mut present: BOOL = 0;
        let mut defaulted: BOOL = 0;
        let mut list = ptr::null_mut();
        // SAFETY: `self.0` is a descriptor built by the conversion API above
        // and every out-pointer is a live local.
        let read =
            unsafe { GetSecurityDescriptorDacl(self.0, &mut present, &mut list, &mut defaulted) };
        if read == 0 {
            return Err(std::io::Error::last_os_error())
                .wrap_err("could not read private file permissions");
        }
        if present == 0 || list.is_null() {
            // A null list here would mean "grant everyone everything", so this
            // fails closed rather than applying it.
            return Err(eyre!(
                "private file permissions carry no access control list"
            ));
        }
        Ok(list)
    }
}

#[cfg(windows)]
impl Drop for PrivateFileSecurity {
    fn drop(&mut self) {
        // SAFETY: the pointer came from
        // ConvertStringSecurityDescriptorToSecurityDescriptorW and is freed
        // once, here.
        unsafe { windows_sys::Win32::Foundation::LocalFree(self.0) };
    }
}

#[cfg(windows)]
fn create_private_new(path: &Path) -> Result<File> {
    use std::{
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr,
    };

    use windows_sys::Win32::{
        Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL},
    };

    let security = PrivateFileSecurity::new()?;
    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
        lpSecurityDescriptor: security.descriptor(),
        bInheritHandle: 0,
    };
    // SAFETY: pointers refer to live, NUL-terminated storage and a valid
    // SECURITY_ATTRIBUTES value borrowed from `security`, which outlives the
    // call. CREATE_NEW prevents accidental truncation.
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
