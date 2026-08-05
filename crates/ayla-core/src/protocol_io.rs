use std::sync::OnceLock;
use tokio::sync::Mutex;

static PROTOCOL_IO: OnceLock<Mutex<()>> = OnceLock::new();

/// Shared lock for the sidecar's stdout protocol stream.
pub fn protocol_io_guard() -> &'static Mutex<()> {
    PROTOCOL_IO.get_or_init(|| Mutex::new(()))
}

/// Suppress third-party diagnostics printed directly to stdout while keeping
/// the sidecar's framed MessagePack stream intact.
#[cfg(unix)]
pub fn with_stdout_suppressed<T>(callback: impl FnOnce() -> T) -> T {
    use std::os::fd::AsRawFd;

    let _protocol_lock = protocol_io_guard().blocking_lock();
    let stdout_fd = std::io::stdout().as_raw_fd();
    let saved_fd = unsafe { libc::dup(stdout_fd) };
    if saved_fd < 0 {
        return callback();
    }
    let dev_null = match std::fs::File::options().write(true).open("/dev/null") {
        Ok(file) => file,
        Err(_) => {
            unsafe { libc::close(saved_fd) };
            return callback();
        }
    };
    unsafe {
        libc::fflush(std::ptr::null_mut());
        libc::dup2(dev_null.as_raw_fd(), stdout_fd);
    }
    let result = callback();
    unsafe {
        libc::fflush(std::ptr::null_mut());
        libc::dup2(saved_fd, stdout_fd);
        libc::close(saved_fd);
    }
    result
}

#[cfg(not(unix))]
pub fn with_stdout_suppressed<T>(callback: impl FnOnce() -> T) -> T {
    let _protocol_lock = protocol_io_guard().blocking_lock();
    callback()
}
