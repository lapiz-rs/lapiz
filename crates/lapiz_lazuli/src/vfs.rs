use std::{
    collections::HashMap,
    ffi::{CStr, c_char, c_int, c_void},
    fs::File,
    io::{self, ErrorKind},
    ptr, slice,
    sync::{LazyLock, OnceLock},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags, ffi};
use uuid::Uuid;

const VFS_NAME_STR: &str = "lapiz_file";
const VFS_NAME: &CStr = c"lapiz_file";
const UNIX_EPOCH_JULIAN_DAY: f64 = 2440587.5;
const SECONDS_PER_DAY: f64 = 86400.0;

static PENDING_FILES: LazyLock<Mutex<HashMap<String, File>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static VFS_REGISTERED: OnceLock<Result<(), c_int>> = OnceLock::new();

#[repr(C)]
struct FileVfsHandle {
    base: ffi::sqlite3_file,
    file: Option<File>,
}

static IO_METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 1,
    xClose: Some(x_close),
    xRead: Some(x_read),
    xWrite: Some(x_write),
    xTruncate: Some(x_truncate),
    xSync: Some(x_sync),
    xFileSize: Some(x_file_size),
    xLock: Some(x_lock),
    xUnlock: Some(x_unlock),
    xCheckReservedLock: Some(x_check_reserved_lock),
    xFileControl: Some(x_file_control),
    xSectorSize: Some(x_sector_size),
    xDeviceCharacteristics: Some(x_device_characteristics),
    xShmMap: None,
    xShmLock: None,
    xShmBarrier: None,
    xShmUnmap: None,
    xFetch: None,
    xUnfetch: None,
};

pub(crate) fn open_connection(file: File) -> Result<Connection> {
    register_vfs()?;

    let token = Uuid::new_v4().to_string();
    PENDING_FILES.lock().insert(token.clone(), file);

    let result = Connection::open_with_flags_and_vfs(
        &token,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        VFS_NAME_STR,
    );
    PENDING_FILES.lock().remove(&token);

    let conn = result?;
    conn.pragma_update(None, "journal_mode", "MEMORY")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(conn)
}

fn register_vfs() -> Result<()> {
    let result = VFS_REGISTERED.get_or_init(|| {
        let vfs = Box::into_raw(Box::new(ffi::sqlite3_vfs {
            iVersion: 1,
            szOsFile: size_of::<FileVfsHandle>() as c_int,
            mxPathname: 512,
            pNext: ptr::null_mut(),
            zName: VFS_NAME.as_ptr(),
            pAppData: ptr::null_mut(),
            xOpen: Some(x_open),
            xDelete: Some(x_delete),
            xAccess: Some(x_access),
            xFullPathname: Some(x_full_pathname),
            xDlOpen: None,
            xDlError: None,
            xDlSym: None,
            xDlClose: None,
            xRandomness: Some(x_randomness),
            xSleep: Some(x_sleep),
            xCurrentTime: Some(x_current_time),
            xGetLastError: Some(x_get_last_error),
            xCurrentTimeInt64: None,
            xSetSystemCall: None,
            xGetSystemCall: None,
            xNextSystemCall: None,
        }));

        // SAFETY: `vfs` is a valid heap-allocated `sqlite3_vfs` that lives for the rest of the process.
        let code = unsafe { ffi::sqlite3_vfs_register(vfs, 0) };
        if code == ffi::SQLITE_OK {
            Ok(())
        } else {
            // SAFETY: Registration failed so SQLite does not retain `vfs`.
            unsafe { drop(Box::from_raw(vfs)) };
            Err(code)
        }
    });

    if let Err(code) = result {
        bail!("failed to register SQLite file VFS (error code {code})");
    }

    Ok(())
}

fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt as _;

        file.read_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt as _;

        file.seek_read(buf, offset)
    }
}

fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt as _;

        file.write_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt as _;

        file.seek_write(buf, offset)
    }
}

unsafe extern "C" fn x_open(
    _vfs: *mut ffi::sqlite3_vfs,
    z_name: *const c_char,
    p_file: *mut ffi::sqlite3_file,
    flags: c_int,
    p_out_flags: *mut c_int,
) -> c_int {
    // SAFETY: SQLite allocates `p_file` with size `szOsFile` and expects `pMethods` to be null on failure.
    unsafe {
        (*p_file).pMethods = ptr::null();
    }

    if (flags & ffi::SQLITE_OPEN_MAIN_DB) == 0 || z_name.is_null() {
        return ffi::SQLITE_CANTOPEN;
    }

    // SAFETY: `z_name` is a non-null NUL-terminated string provided by SQLite.
    let Ok(name) = unsafe { CStr::from_ptr(z_name) }.to_str() else {
        return ffi::SQLITE_CANTOPEN;
    };

    let Some(file) = PENDING_FILES.lock().remove(name) else {
        return ffi::SQLITE_CANTOPEN;
    };

    // SAFETY: `p_file` has size `size_of::<FileVfsHandle>()` and proper alignment from SQLite.
    unsafe {
        ptr::write(
            p_file.cast::<FileVfsHandle>(),
            FileVfsHandle {
                base: ffi::sqlite3_file {
                    pMethods: &IO_METHODS,
                },
                file: Some(file),
            },
        );
        if !p_out_flags.is_null() {
            *p_out_flags = flags;
        }
    }

    ffi::SQLITE_OK
}

unsafe extern "C" fn x_delete(
    _vfs: *mut ffi::sqlite3_vfs,
    _z_name: *const c_char,
    _sync_dir: c_int,
) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_access(
    _vfs: *mut ffi::sqlite3_vfs,
    _z_name: *const c_char,
    _flags: c_int,
    p_res_out: *mut c_int,
) -> c_int {
    if !p_res_out.is_null() {
        // SAFETY: `p_res_out` is a valid out-pointer supplied by SQLite.
        unsafe { *p_res_out = 0 };
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_full_pathname(
    _vfs: *mut ffi::sqlite3_vfs,
    z_name: *const c_char,
    n_out: c_int,
    z_out: *mut c_char,
) -> c_int {
    if z_name.is_null() || z_out.is_null() || n_out <= 0 {
        return ffi::SQLITE_CANTOPEN;
    }

    // SAFETY: `z_name` is NUL-terminated and `z_out` has capacity `n_out`.
    let bytes = unsafe { CStr::from_ptr(z_name) }.to_bytes_with_nul();
    if bytes.len() > n_out as usize {
        return ffi::SQLITE_CANTOPEN;
    }

    // SAFETY: `z_out` is non-overlapping with `z_name` and has space for `bytes.len()`.
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), z_out, bytes.len());
    }

    ffi::SQLITE_OK
}

unsafe extern "C" fn x_randomness(
    _vfs: *mut ffi::sqlite3_vfs,
    n_byte: c_int,
    z_out: *mut c_char,
) -> c_int {
    if n_byte <= 0 || z_out.is_null() {
        return 0;
    }

    // SAFETY: Querying SQLite's default VFS is thread-safe after SQLite initialization.
    let default_vfs = unsafe { ffi::sqlite3_vfs_find(ptr::null()) };
    // SAFETY: `default_vfs` is either null or points to a registered SQLite VFS with `'static` lifetime.
    if let Some(default_vfs) = unsafe { default_vfs.as_mut() }
        && let Some(randomness) = default_vfs.xRandomness
    {
        // SAFETY: `z_out` is a writable buffer of `n_byte` bytes supplied by SQLite.
        return unsafe { randomness(default_vfs, n_byte, z_out) };
    }

    // SAFETY: `z_out` has length `n_byte`.
    let out = unsafe { slice::from_raw_parts_mut(z_out.cast::<u8>(), n_byte as usize) };
    for chunk in out.chunks_mut(16) {
        let bytes = Uuid::new_v4().into_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    n_byte
}

unsafe extern "C" fn x_sleep(_vfs: *mut ffi::sqlite3_vfs, microseconds: c_int) -> c_int {
    if microseconds > 0 {
        thread::sleep(Duration::from_micros(microseconds as u64));
    }
    microseconds.max(0)
}

unsafe extern "C" fn x_current_time(_vfs: *mut ffi::sqlite3_vfs, p_time_out: *mut f64) -> c_int {
    if p_time_out.is_null() {
        return ffi::SQLITE_ERROR;
    }

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    // SAFETY: `p_time_out` is a valid out-pointer supplied by SQLite.
    unsafe {
        *p_time_out = UNIX_EPOCH_JULIAN_DAY + seconds / SECONDS_PER_DAY;
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_get_last_error(
    _vfs: *mut ffi::sqlite3_vfs,
    _n_buf: c_int,
    _z_buf: *mut c_char,
) -> c_int {
    0
}

unsafe extern "C" fn x_close(p_file: *mut ffi::sqlite3_file) -> c_int {
    // SAFETY: `p_file` was initialized by `x_open` as a `FileVfsHandle`.
    let handle = unsafe { &mut *p_file.cast::<FileVfsHandle>() };
    handle.file = None;
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_read(
    p_file: *mut ffi::sqlite3_file,
    z_buf: *mut c_void,
    i_amt: c_int,
    i_ofst: ffi::sqlite3_int64,
) -> c_int {
    if i_amt < 0 || i_ofst < 0 {
        return ffi::SQLITE_IOERR_READ;
    }
    if i_amt == 0 {
        return ffi::SQLITE_OK;
    }

    // SAFETY: `p_file` was initialized by `x_open` as a `FileVfsHandle`.
    let handle = unsafe { &*p_file.cast::<FileVfsHandle>() };
    let Some(file) = handle.file.as_ref() else {
        return ffi::SQLITE_IOERR_READ;
    };
    // SAFETY: `z_buf` points to `i_amt` writable bytes supplied by SQLite.
    let buf = unsafe { slice::from_raw_parts_mut(z_buf.cast::<u8>(), i_amt as usize) };

    let mut total_read = 0;
    let base_offset = i_ofst as u64;
    while total_read < buf.len() {
        match read_at(
            file,
            &mut buf[total_read..],
            base_offset + total_read as u64,
        ) {
            Ok(0) => {
                buf[total_read..].fill(0);
                return ffi::SQLITE_IOERR_SHORT_READ;
            }
            Ok(n) => total_read += n,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return ffi::SQLITE_IOERR_READ,
        }
    }

    ffi::SQLITE_OK
}

unsafe extern "C" fn x_write(
    p_file: *mut ffi::sqlite3_file,
    z_buf: *const c_void,
    i_amt: c_int,
    i_ofst: ffi::sqlite3_int64,
) -> c_int {
    if i_amt < 0 || i_ofst < 0 {
        return ffi::SQLITE_IOERR_WRITE;
    }
    if i_amt == 0 {
        return ffi::SQLITE_OK;
    }

    // SAFETY: `p_file` was initialized by `x_open` as a `FileVfsHandle`.
    let handle = unsafe { &*p_file.cast::<FileVfsHandle>() };
    let Some(file) = handle.file.as_ref() else {
        return ffi::SQLITE_IOERR_WRITE;
    };
    // SAFETY: `z_buf` points to `i_amt` readable bytes supplied by SQLite.
    let buf = unsafe { slice::from_raw_parts(z_buf.cast::<u8>(), i_amt as usize) };

    let mut total_written = 0;
    let base_offset = i_ofst as u64;
    while total_written < buf.len() {
        match write_at(
            file,
            &buf[total_written..],
            base_offset + total_written as u64,
        ) {
            Ok(0) => return ffi::SQLITE_IOERR_WRITE,
            Ok(n) => total_written += n,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return ffi::SQLITE_IOERR_WRITE,
        }
    }

    ffi::SQLITE_OK
}

unsafe extern "C" fn x_truncate(p_file: *mut ffi::sqlite3_file, size: ffi::sqlite3_int64) -> c_int {
    if size < 0 {
        return ffi::SQLITE_IOERR_TRUNCATE;
    }

    // SAFETY: `p_file` was initialized by `x_open`.
    let handle = unsafe { &*p_file.cast::<FileVfsHandle>() };
    let Some(file) = handle.file.as_ref() else {
        return ffi::SQLITE_IOERR_TRUNCATE;
    };

    match file.set_len(size as u64) {
        Ok(()) => ffi::SQLITE_OK,
        Err(_) => ffi::SQLITE_IOERR_TRUNCATE,
    }
}

unsafe extern "C" fn x_sync(p_file: *mut ffi::sqlite3_file, _flags: c_int) -> c_int {
    // SAFETY: `p_file` was initialized by `x_open`.
    let handle = unsafe { &*p_file.cast::<FileVfsHandle>() };
    let Some(file) = handle.file.as_ref() else {
        return ffi::SQLITE_IOERR_FSYNC;
    };

    match file.sync_data() {
        Ok(()) => ffi::SQLITE_OK,
        Err(_) => ffi::SQLITE_IOERR_FSYNC,
    }
}

unsafe extern "C" fn x_file_size(
    p_file: *mut ffi::sqlite3_file,
    p_size: *mut ffi::sqlite3_int64,
) -> c_int {
    if p_size.is_null() {
        return ffi::SQLITE_IOERR_FSTAT;
    }

    // SAFETY: `p_file` was initialized by `x_open`.
    let handle = unsafe { &*p_file.cast::<FileVfsHandle>() };
    let Some(file) = handle.file.as_ref() else {
        return ffi::SQLITE_IOERR_FSTAT;
    };

    match file.metadata() {
        Ok(metadata) => {
            // SAFETY: `p_size` is a valid out-pointer supplied by SQLite.
            unsafe { *p_size = metadata.len() as ffi::sqlite3_int64 };
            ffi::SQLITE_OK
        }
        Err(_) => ffi::SQLITE_IOERR_FSTAT,
    }
}

unsafe extern "C" fn x_lock(_p_file: *mut ffi::sqlite3_file, _lock_type: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_unlock(_p_file: *mut ffi::sqlite3_file, _lock_type: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_check_reserved_lock(
    _p_file: *mut ffi::sqlite3_file,
    p_res_out: *mut c_int,
) -> c_int {
    if !p_res_out.is_null() {
        // SAFETY: `p_res_out` is a valid out-pointer supplied by SQLite.
        unsafe { *p_res_out = 0 };
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn x_file_control(
    _p_file: *mut ffi::sqlite3_file,
    _op: c_int,
    _p_arg: *mut c_void,
) -> c_int {
    ffi::SQLITE_NOTFOUND
}

unsafe extern "C" fn x_sector_size(_p_file: *mut ffi::sqlite3_file) -> c_int {
    4096
}

unsafe extern "C" fn x_device_characteristics(_p_file: *mut ffi::sqlite3_file) -> c_int {
    0
}
