use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_alloc(size: u64, type_id: u64) -> *mut c_void {
    let total = (size.max(16) + 16) as usize;
    let layout = std::alloc::Layout::from_size_align(total, 8).unwrap();
    unsafe {
        let ptr = std::alloc::alloc_zeroed(layout);
        // Header is [gc_meta:8 | type_id:8]; user pointer skips both.
        // type_id sits at user_ptr - 8 (gc_meta at user_ptr - 16, zeroed).
        let header = ptr as *mut u64;
        header.add(1).write(type_id);
        ptr.add(16) as *mut c_void
    }
}

// (struct_id, iface_id, slot) → function pointer. Registered at program
// start by `__of_vtable_init` (codegen-emitted), read on each virtual
// dispatch. Globally shared — JIT tests must clear it before re-init
// (handled by `__of_vtable_clear`).
type VTableKey = (u64, u64, u64);
static VTABLE_REGISTRY: Mutex<Option<HashMap<VTableKey, usize>>> = Mutex::new(None);

fn with_registry<R>(f: impl FnOnce(&mut HashMap<VTableKey, usize>) -> R) -> R {
    let mut guard = VTABLE_REGISTRY.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    f(map)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_vtable_register(
    struct_id: u64,
    iface_id: u64,
    slot: u64,
    fn_ptr: *const c_void,
) {
    with_registry(|m| {
        m.insert((struct_id, iface_id, slot), fn_ptr as usize);
    });
}

/// Wipes every registered vtable entry. Codegen emits a call to this at
/// the top of `__of_vtable_init`, so re-running the init (JIT tests, hot
/// reload) starts from a clean slate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_vtable_clear() {
    with_registry(|m| m.clear());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_vtable_lookup(
    recv: *const c_void,
    iface_id: u64,
    slot: u64,
) -> *const c_void {
    if recv.is_null() {
        return std::ptr::null();
    }
    // type_id lives at recv - 8 (see __of_alloc layout).
    let type_id = unsafe { (recv as *const u64).offset(-1).read() };
    with_registry(|m| {
        m.get(&(type_id, iface_id, slot))
            .copied()
            .map(|p| p as *const c_void)
            .unwrap_or(std::ptr::null())
    })
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
    leak_cstring(format!("{}{}", a, b))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_str_eq(a: *const c_char, b: *const c_char) -> u8 {
    if a == b {
        return 1;
    }
    if a.is_null() || b.is_null() {
        return 0;
    }
    let a = unsafe { CStr::from_ptr(a) };
    let b = unsafe { CStr::from_ptr(b) };
    if a.to_bytes() == b.to_bytes() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_i64_to_str(v: i64) -> *const c_char {
    leak_cstring(v.to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_u64_to_str(v: u64) -> *const c_char {
    leak_cstring(v.to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_f64_to_str(v: f64) -> *const c_char {
    leak_cstring(v.to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_bool_to_str(v: u8) -> *const c_char {
    leak_cstring(if v != 0 { "true" } else { "false" }.to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_char_to_str(codepoint: u32) -> *const c_char {
    let s = char::from_u32(codepoint)
        .map(|c| c.to_string())
        .unwrap_or_default();
    leak_cstring(s)
}

fn leak_cstring(mut s: String) -> *const c_char {
    s.push('\0');
    let boxed = s.into_bytes().into_boxed_slice();
    Box::leak(boxed).as_ptr() as *const c_char
}

// ---- Buffer (declared in of_core.of) ----
//
// `extern struct Buffer { data: *u8, size: u64 }`. The of-side lowering
// treats `Buffer` as a managed reference, so every parameter / return at
// the C ABI boundary is `*mut Buffer` — a pointer to a 16-byte struct
// {data, size} that lives on the foreign heap.
//
// Lifetime: each `__of_buffer_alloc` pairs with a `__of_buffer_free`.
// The GC never touches these buffers.

#[repr(C)]
pub struct Buffer {
    pub data: *mut u8,
    pub size: u64,
}

fn buffer_box(data: *mut u8, size: u64) -> *mut Buffer {
    Box::into_raw(Box::new(Buffer { data, size }))
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_buffer_alloc(size: u64) -> *mut Buffer {
    if size == 0 {
        return buffer_box(std::ptr::null_mut(), 0);
    }
    let layout = match std::alloc::Layout::from_size_align(size as usize, 8) {
        Ok(l) => l,
        Err(_) => return std::ptr::null_mut(),
    };
    let data = unsafe { std::alloc::alloc_zeroed(layout) };
    if data.is_null() {
        return std::ptr::null_mut();
    }
    buffer_box(data, size)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_buffer_free(buf: *mut Buffer) {
    if buf.is_null() {
        return;
    }
    // Reclaim both the data region and the Buffer header.
    let owned = unsafe { Box::from_raw(buf) };
    if !owned.data.is_null() && owned.size > 0
        && let Ok(layout) = std::alloc::Layout::from_size_align(owned.size as usize, 8)
    {
        unsafe { std::alloc::dealloc(owned.data, layout) };
    }
}

/// Returns the byte at `i`. Out-of-range reads return 0; callers should
/// range-check via `buf.size` before reading. The of-side declares this
/// as `u8 | null` but the small-primitive nullable ABI isn't wired up in
/// codegen yet, so a clamped getter is the pragmatic shape today.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_buffer_get(buf: *mut Buffer, i: u64) -> u8 {
    if buf.is_null() {
        return 0;
    }
    let buf = unsafe { &*buf };
    if buf.data.is_null() || i >= buf.size {
        return 0;
    }
    unsafe { *buf.data.add(i as usize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_buffer_set(buf: *mut Buffer, i: u64, v: u8) {
    if buf.is_null() {
        return;
    }
    let buf = unsafe { &*buf };
    if buf.data.is_null() || i >= buf.size {
        return;
    }
    unsafe { *buf.data.add(i as usize) = v };
}

// ---- List<T> (declared in of_core.of) ----
//
// Backing store: `Vec<*mut c_void>` boxed and leaked. Elements are read and
// written as opaque pointer-sized slots — fine for `List<SomeStruct>` (managed
// references are pointer-sized) but NOT for `List<i32>` / `List<f64>` /
// `List<bool>`, which the small-primitive ABI would need to widen and the
// runtime would need to read at the right width. The of-side type system
// already exposes that gap: `__of_list_get<T>(l, i): T | null` round-trips
// through a tagged union for primitives, which codegen doesn't emit yet.
//
// No `__of_list_free` — consistent with the rest of `__of_alloc`, lists leak.

type ElemSlot = *mut c_void;
type ListVec = Vec<ElemSlot>;

fn list_mut<'a>(l: *mut ListVec) -> Option<&'a mut ListVec> {
    if l.is_null() { None } else { Some(unsafe { &mut *l }) }
}

#[unsafe(no_mangle)]
pub extern "C" fn __of_list_new() -> *mut ListVec {
    Box::into_raw(Box::new(Vec::new()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_size(l: *mut ListVec) -> i64 {
    match list_mut(l) {
        Some(v) => v.len() as i64,
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_is_empty(l: *mut ListVec) -> u8 {
    match list_mut(l) {
        Some(v) => if v.is_empty() { 1 } else { 0 },
        None => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_clear(l: *mut ListVec) {
    if let Some(v) = list_mut(l) {
        v.clear();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_get(l: *mut ListVec, i: i64) -> *mut c_void {
    let v = match list_mut(l) {
        Some(v) => v,
        None => return std::ptr::null_mut(),
    };
    if i < 0 || (i as usize) >= v.len() {
        return std::ptr::null_mut();
    }
    v[i as usize]
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_set(l: *mut ListVec, i: i64, val: *mut c_void) {
    let v = match list_mut(l) {
        Some(v) => v,
        None => return,
    };
    if i < 0 || (i as usize) >= v.len() {
        return;
    }
    v[i as usize] = val;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_push(l: *mut ListVec, val: *mut c_void) {
    if let Some(v) = list_mut(l) {
        v.push(val);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_pop(l: *mut ListVec) -> *mut c_void {
    match list_mut(l).and_then(|v| v.pop()) {
        Some(p) => p,
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_insert(l: *mut ListVec, i: i64, val: *mut c_void) {
    let v = match list_mut(l) {
        Some(v) => v,
        None => return,
    };
    if i < 0 {
        return;
    }
    let idx = (i as usize).min(v.len());
    v.insert(idx, val);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_remove(l: *mut ListVec, i: i64) -> *mut c_void {
    let v = match list_mut(l) {
        Some(v) => v,
        None => return std::ptr::null_mut(),
    };
    if i < 0 || (i as usize) >= v.len() {
        return std::ptr::null_mut();
    }
    v.remove(i as usize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_truncate(l: *mut ListVec, n: i64) {
    if let Some(v) = list_mut(l) {
        let n = if n < 0 { 0 } else { n as usize };
        v.truncate(n);
    }
}

// Reference-equality only — fine for managed `T` (which all share a single
// allocation per value), wrong for primitives that pass-through the wider
// nullable ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_contains(l: *mut ListVec, val: *mut c_void) -> u8 {
    match list_mut(l) {
        Some(v) => if v.iter().any(|&p| p == val) { 1 } else { 0 },
        None => 0,
    }
}

// `i64 | null` lowers to a tagged union for primitives that codegen doesn't
// emit yet, so this helper isn't reachable from the of-side; declared to keep
// the symbol table aligned with `of_core.of`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __of_list_index_of(l: *mut ListVec, val: *mut c_void) -> i64 {
    match list_mut(l) {
        Some(v) => v.iter().position(|&p| p == val).map(|n| n as i64).unwrap_or(-1),
        None => -1,
    }
}
