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

// Toggle SO_REUSEADDR on a socket. Returns 0 on success, -1 on error
// (errno is set as usual). Lets a server rebind to its port immediately
// after a previous instance exits, instead of waiting out TIME_WAIT.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_set_reuseaddr(fd: c_int, on: u8) -> c_int {
    let val: c_int = if on != 0 { 1 } else { 0 };
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &val as *const _ as *const libc::c_void,
            std::mem::size_of::<c_int>() as libc::socklen_t,
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

// ---- Monotonic clock ----
//
// Milliseconds since an unspecified epoch. Suitable for relative deadlines
// (long-poll timeout, retry backoff). Never goes backwards.
#[unsafe(no_mangle)]
pub extern "C" fn __of_now_ms() -> i64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as i64) * 1000 + (ts.tv_nsec as i64) / 1_000_000
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

// a & !b. Pairs with __of_or_i32 — needed to clear flag bits.
#[unsafe(no_mangle)]
pub extern "C" fn __of_and_not_i32(a: c_int, b: c_int) -> c_int {
    a & !b
}

// ---- High-level helpers used by the of-side facade ----

// Copy a null-terminated C string into a fresh foreign-heap Buffer
// (excluding the terminator). Returns NULL on null input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_buffer_from_str(s: *const c_char) -> *mut Buffer {
    if s.is_null() {
        return std::ptr::null_mut();
    }
    let bytes = unsafe { std::ffi::CStr::from_ptr(s) }.to_bytes().to_vec();
    buffer_from_vec(bytes)
}

// Copy `len` bytes starting at `start` out of `buf` into a fresh, leaked,
// null-terminated C string (the `str` ABI on the of-side). Non-UTF8 bytes
// survive — the runtime treats `str` as opaque bytes until printed.
// Returns an empty C string on bad input (null buffer, negative indices,
// out-of-range range). We don't return NULL because the small-primitive
// nullable ABI (str | null) isn't wired up in codegen — the C function
// would need to hand back a 16-byte tagged block, which is more work than
// the failure case warrants. Callers can `s.size()` or compare to "" if
// they care to distinguish.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_str_from_buffer(
    buf: *mut Buffer,
    start: i64,
    len: i64,
) -> *const c_char {
    let empty = || {
        let v = vec![0u8];
        Box::leak(v.into_boxed_slice()).as_ptr() as *const c_char
    };
    if buf.is_null() || start < 0 || len < 0 {
        return empty();
    }
    let b = unsafe { &*buf };
    if b.data.is_null() {
        return empty();
    }
    let s = start as u64;
    let n = len as u64;
    match s.checked_add(n) {
        Some(end) if end <= b.size => {}
        _ => return empty(),
    }
    let slice = unsafe { std::slice::from_raw_parts(b.data.add(s as usize), n as usize) };
    let mut v = Vec::with_capacity(slice.len() + 1);
    v.extend_from_slice(slice);
    v.push(0);
    Box::leak(v.into_boxed_slice()).as_ptr() as *const c_char
}

// Parse a dotted-quad IPv4 string and build a sockaddr_in for (host, port).
// `port` is host-byte-order. Returns NULL on parse failure or null input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_sockaddr_for_host(
    host: *const c_char,
    port: u16,
) -> *mut Buffer {
    if host.is_null() {
        return std::ptr::null_mut();
    }
    let s = match unsafe { std::ffi::CStr::from_ptr(host) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let addr: std::net::Ipv4Addr = match s.parse() {
        Ok(a) => a,
        Err(_) => return std::ptr::null_mut(),
    };
    // octets() is in network order. Reading those bytes as a native u32
    // produces a value whose memory representation matches network byte
    // order — i.e. the right value for sockaddr_in.s_addr.
    let ip_be = u32::from_ne_bytes(addr.octets());
    __of_sockaddr_in(ip_be, port.to_be())
}

// accept() without exposing the peer sockaddr. Allocates a transient
// sockaddr_in + socklen_t on the stack so the facade doesn't have to
// thread two Buffers through.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_accept_simple(fd: c_int) -> c_int {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len: libc::socklen_t = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    unsafe {
        libc::accept(
            fd,
            &mut addr as *mut _ as *mut libc::sockaddr,
            &mut len,
        )
    }
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
