//! Otter runtime — the C-ABI helpers that every program produced by
//! `otter_fusion::codegen` calls into. Built as both `staticlib` (for AOT
//! linking) and `rlib` (so the JIT path can register addresses by name).
//!
//! Everything here is a stub: a real implementation needs a GC, a type
//! registry keyed on `MirTypeId`, and a vtable registry keyed on
//! `(MirTypeId, MirTypeId)`. None of that exists yet — the helpers
//! return reasonable shapes so simple programs (no allocation, no
//! virtual dispatch, no unions) can run end-to-end.

use std::ffi::c_void;

/// Allocate and zero a managed struct. Real impl: look `type_id` up in a
/// registry to know the size, allocate via the GC, write the gc header
/// and type id, return a pointer to the first field.
#[unsafe(no_mangle)]
pub extern "C" fn otter_alloc_struct(_type_id: i32) -> *mut c_void {
    alloc_zeroed(64, 8)
}

/// Same as `otter_alloc_struct` but for closure environments.
#[unsafe(no_mangle)]
pub extern "C" fn otter_alloc_env(_type_id: i32) -> *mut c_void {
    alloc_zeroed(64, 8)
}

/// Bundle a function pointer and its environment into a closure value.
/// Layout: `[fn_ptr | env_ptr]`.
#[unsafe(no_mangle)]
pub extern "C" fn otter_alloc_closure(
    fn_ptr: *const c_void,
    env: *const c_void,
) -> *mut c_void {
    let p = alloc_zeroed(16, 8) as *mut *const c_void;
    unsafe {
        *p = fn_ptr;
        *p.add(1) = env;
    }
    p as *mut c_void
}

/// Construct a union value. Layout: `[u16 tag | pad | ptr payload]`.
#[unsafe(no_mangle)]
pub extern "C" fn otter_union_construct(
    _type_id: i32,
    tag: i32,
    payload: *const c_void,
) -> *mut c_void {
    let p = alloc_zeroed(16, 8);
    unsafe {
        *(p as *mut u16) = tag as u16;
        *((p as *mut u8).add(8) as *mut *const c_void) = payload;
    }
    p
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_union_tag(val: *const c_void) -> i32 {
    unsafe { *(val as *const u16) as i32 }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_union_payload(val: *const c_void) -> *mut c_void {
    unsafe { *((val as *const u8).add(8) as *const *mut c_void) }
}

/// Look up a concrete function pointer for a virtual call. Real impl
/// reads the type id at offset −8 from `recv`, indexes into a registered
/// vtable for the (struct, interface) pair, and returns the slot.
#[unsafe(no_mangle)]
pub extern "C" fn otter_vcall_lookup(
    _recv: *const c_void,
    _iface_id: i32,
    _slot: i32,
) -> *const c_void {
    panic!("otter_vcall_lookup: vtable registry not implemented");
}

/// Abort the program with a reason code. Reasons match
/// `mir::TrapReason`: 1 = AsMismatch, 2 = NullDeref.
#[unsafe(no_mangle)]
pub extern "C" fn otter_trap(reason: i32) -> ! {
    eprintln!("otter trap: reason={reason}");
    std::process::abort();
}

/// Entry shim used by AOT-linked binaries. The compiler emits the
/// program's entry function under the symbol `otter_main`; the platform
/// linker resolves `main` to this shim, which calls into the program
/// and forwards its return code.
#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    unsafe extern "C" {
        fn otter_main() -> i64;
    }
    let r = unsafe { otter_main() };
    r as i32
}

fn alloc_zeroed(size: usize, align: usize) -> *mut c_void {
    let layout = std::alloc::Layout::from_size_align(size, align).expect("bad layout");
    unsafe { std::alloc::alloc_zeroed(layout) as *mut c_void }
}
