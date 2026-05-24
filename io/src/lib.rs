// otter_io — POSIX file, networking and multiplexing syscalls for the
// `of` language. Pairs with `io/of_io.of` on the language side.
//
// Calling convention:
//   - `str` lowers to `*const c_char` (null-terminated).
//   - Byte regions (sockaddr, fd_set, timeval) ride on `Buffer`; data=null
//     means "pass NULL to the kernel".
//   - Functions return the raw libc result. -1 = error, check errno.

use std::os::raw::{c_char, c_int, c_uint};

// Mirrors the of-side `extern struct Buffer { data: *u8, size: u64 }`.
// The of-side lowering treats Buffer as a managed reference, so every
// parameter/return at the C ABI boundary uses `*mut Buffer`.
#[repr(C)]
pub struct Buffer {
    pub data: *mut u8,
    pub size: u64,
}

unsafe fn buf_data_mut<T>(buf: *mut Buffer) -> *mut T {
    if buf.is_null() {
        return std::ptr::null_mut();
    }
    let b = unsafe { &*buf };
    if b.data.is_null() {
        std::ptr::null_mut()
    } else {
        b.data as *mut T
    }
}

unsafe fn buf_data_const<T>(buf: *mut Buffer) -> *const T {
    unsafe { buf_data_mut::<T>(buf) as *const T }
}

fn buffer_from_vec(mut v: Vec<u8>) -> *mut Buffer {
    v.shrink_to_fit();
    let size = v.len() as u64;
    let data = v.as_mut_ptr();
    std::mem::forget(v);
    Box::into_raw(Box::new(Buffer { data, size }))
}

// ---- File operations ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_open(path: *const c_char, flags: c_int, mode: c_uint) -> c_int {
    if path.is_null() {
        return -1;
    }
    unsafe { libc::open(path, flags, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_read(fd: c_int, buf: *mut Buffer, len: u64) -> i64 {
    unsafe { libc::read(fd, buf_data_mut::<libc::c_void>(buf), len as usize) as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_write(fd: c_int, buf: *mut Buffer, len: u64) -> i64 {
    unsafe { libc::write(fd, buf_data_const::<libc::c_void>(buf), len as usize) as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_close(fd: c_int) -> c_int {
    unsafe { libc::close(fd) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_lseek(fd: c_int, offset: i64, whence: c_int) -> i64 {
    unsafe { libc::lseek(fd, offset as libc::off_t, whence) as i64 }
}

// ---- Networking ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_socket(domain: c_int, ty: c_int, protocol: c_int) -> c_int {
    unsafe { libc::socket(domain, ty, protocol) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_bind(fd: c_int, addr: *mut Buffer, len: u32) -> c_int {
    unsafe {
        libc::bind(
            fd,
            buf_data_const::<libc::sockaddr>(addr),
            len as libc::socklen_t,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_listen(fd: c_int, backlog: c_int) -> c_int {
    unsafe { libc::listen(fd, backlog) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_accept(
    fd: c_int,
    addr: *mut Buffer,
    addrlen: *mut Buffer,
) -> c_int {
    unsafe {
        libc::accept(
            fd,
            buf_data_mut::<libc::sockaddr>(addr),
            buf_data_mut::<libc::socklen_t>(addrlen),
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_connect(fd: c_int, addr: *mut Buffer, len: u32) -> c_int {
    unsafe {
        libc::connect(
            fd,
            buf_data_const::<libc::sockaddr>(addr),
            len as libc::socklen_t,
        )
    }
}

// ---- Multiplexing + control ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_select(
    nfds: c_int,
    rfds: *mut Buffer,
    wfds: *mut Buffer,
    efds: *mut Buffer,
    timeout: *mut Buffer,
) -> c_int {
    unsafe {
        libc::select(
            nfds,
            buf_data_mut::<libc::fd_set>(rfds),
            buf_data_mut::<libc::fd_set>(wfds),
            buf_data_mut::<libc::fd_set>(efds),
            buf_data_mut::<libc::timeval>(timeout),
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_fcntl(fd: c_int, cmd: c_int, arg: i64) -> c_int {
    unsafe { libc::fcntl(fd, cmd, arg as c_int) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

// ---- Byte-region builders ----

#[unsafe(no_mangle)]
pub extern "C" fn __of_sockaddr_in(ip_be: u32, port_be: u16) -> *mut Buffer {
    let mut v = vec![0u8; std::mem::size_of::<libc::sockaddr_in>()];
    let sa = v.as_mut_ptr() as *mut libc::sockaddr_in;
    unsafe {
        (*sa).sin_family = libc::AF_INET as libc::sa_family_t;
        (*sa).sin_port = port_be;
        (*sa).sin_addr.s_addr = ip_be;
    }
    buffer_from_vec(v)
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_timeval(sec: i64, usec: i64) -> *mut Buffer {
    let mut v = vec![0u8; std::mem::size_of::<libc::timeval>()];
    let tv = v.as_mut_ptr() as *mut libc::timeval;
    unsafe {
        (*tv).tv_sec = sec as libc::time_t;
        (*tv).tv_usec = usec as libc::suseconds_t;
    }
    buffer_from_vec(v)
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_fdset_new() -> *mut Buffer {
    let v = vec![0u8; std::mem::size_of::<libc::fd_set>()];
    buffer_from_vec(v)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_fdset_zero(set: *mut Buffer) {
    unsafe { libc::FD_ZERO(buf_data_mut::<libc::fd_set>(set)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_fdset_set(set: *mut Buffer, fd: c_int) {
    unsafe { libc::FD_SET(fd, buf_data_mut::<libc::fd_set>(set)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_fdset_clr(set: *mut Buffer, fd: c_int) {
    unsafe { libc::FD_CLR(fd, buf_data_mut::<libc::fd_set>(set)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_fdset_isset(set: *mut Buffer, fd: c_int) -> bool {
    unsafe { libc::FD_ISSET(fd, buf_data_mut::<libc::fd_set>(set)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_htons(x: u16) -> u16 {
    x.to_be()
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_htonl(x: u32) -> u32 {
    x.to_be()
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_or_i32(a: c_int, b: c_int) -> c_int {
    a | b
}

// ---- Platform-resolved flag constants ----

#[unsafe(no_mangle)]
pub extern "C" fn O_CREAT() -> c_int {
    libc::O_CREAT
}
#[unsafe(no_mangle)]
pub extern "C" fn O_TRUNC() -> c_int {
    libc::O_TRUNC
}
#[unsafe(no_mangle)]
pub extern "C" fn O_APPEND() -> c_int {
    libc::O_APPEND
}
#[unsafe(no_mangle)]
pub extern "C" fn O_NONBLOCK() -> c_int {
    libc::O_NONBLOCK
}
#[unsafe(no_mangle)]
pub extern "C" fn AF_INET() -> c_int {
    libc::AF_INET
}
#[unsafe(no_mangle)]
pub extern "C" fn AF_INET6() -> c_int {
    libc::AF_INET6
}
#[unsafe(no_mangle)]
pub extern "C" fn AF_UNIX() -> c_int {
    libc::AF_UNIX
}
#[unsafe(no_mangle)]
pub extern "C" fn SOCK_STREAM() -> c_int {
    libc::SOCK_STREAM
}
#[unsafe(no_mangle)]
pub extern "C" fn SOCK_DGRAM() -> c_int {
    libc::SOCK_DGRAM
}
#[unsafe(no_mangle)]
pub extern "C" fn F_GETFL() -> c_int {
    libc::F_GETFL
}
#[unsafe(no_mangle)]
pub extern "C" fn F_SETFL() -> c_int {
    libc::F_SETFL
}
