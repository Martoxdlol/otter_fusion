use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_alloc(size: u64, _type_id: u64) -> *mut c_void {
    let total = (size.max(16) + 16) as usize;
    let layout = std::alloc::Layout::from_size_align(total, 8).unwrap();
    unsafe {
        let ptr = std::alloc::alloc_zeroed(layout);
        // Skip a fake header (16 bytes) so callers see a "body" pointer.
        ptr.add(16) as *mut c_void
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_vtable_lookup(
    _recv: *const c_void,
    _iface_id: u64,
    _slot: u64,
) -> *const c_void {
    // No real vtable resolution yet; virtual calls crash, non-virtual paths run.
    std::ptr::null()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_print(s: *const c_char) {
    if s.is_null() {
        return;
    }
    let cstr = unsafe { CStr::from_ptr(s) };
    print!("{}", cstr.to_string_lossy());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_println(s: *const c_char) {
    unsafe { __of_print(s) };
    println!();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_str_concat(
    a: *const c_char,
    b: *const c_char,
) -> *const c_char {
    let a = if a.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(a) }.to_string_lossy().into_owned()
    };
    let b = if b.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(b) }.to_string_lossy().into_owned()
    };
    let combined = format!("{}{}\0", a, b);
    let boxed = combined.into_bytes().into_boxed_slice();
    Box::leak(boxed).as_ptr() as *const c_char
}
