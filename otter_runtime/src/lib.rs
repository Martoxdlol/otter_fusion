//! Otter runtime — the C-ABI helpers that every program produced by
//! `otter_fusion::codegen` calls into. Built as both `staticlib` (for AOT
//! linking) and `rlib` (so the JIT path can register addresses by name).
//!
//! The allocator here is a leaking `alloc_zeroed`, but the *layout* it
//! produces matches what a real GC will need: every managed object is
//! preceded by a 16-byte prefix `[gc_header | type_id]` and the pointer
//! handed back is to the body, so callers can write field 0 at offset 0.
//! Replacing the bump-style allocator with a tracing GC later is a
//! drop-in change as long as the prefix shape stays the same.

use std::ffi::c_void;

/// Size of the per-object prefix: 8 bytes of GC header + 8 bytes for the
/// type id (the i32 lives in the low 4 bytes; the rest is padding).
/// Body alignment > 16 bumps this up — see [`alloc_with_header`].
const PREFIX_BYTES: usize = 16;

/// Sentinel type ids written into the header for objects whose layout
/// is fixed at the runtime level rather than coming from the MIR type
/// table. Keep these negative to stay disjoint from real `MirTypeId`s.
const TYPE_ID_CLOSURE: i32 = -1;

/// Allocate a managed struct. `size`/`align` come from
/// `MirTypeDef::Struct.layout`; `type_id` is the `MirTypeId` so the GC
/// can look up shape info later.
#[unsafe(no_mangle)]
pub extern "C" fn otter_alloc_struct(size: i32, align: i32, type_id: i32) -> *mut c_void {
    alloc_with_header(size as usize, align as usize, type_id)
}

/// Same as `otter_alloc_struct` but used for closure environments. The
/// env *is* a struct in MIR (`MirTypeDef::Struct` named `closure_env#N`)
/// so the contract is identical — the separate symbol just lets a real
/// GC distinguish them in stats / heuristics if it wants.
#[unsafe(no_mangle)]
pub extern "C" fn otter_alloc_env(size: i32, align: i32, type_id: i32) -> *mut c_void {
    alloc_with_header(size as usize, align as usize, type_id)
}

/// Bundle a function pointer and its environment into a closure value.
/// Body layout: `[fn_ptr | env_ptr]`, two pointers wide.
#[unsafe(no_mangle)]
pub extern "C" fn otter_alloc_closure(
    fn_ptr: *const c_void,
    env: *const c_void,
) -> *mut c_void {
    let p = alloc_with_header(16, 8, TYPE_ID_CLOSURE) as *mut *const c_void;
    unsafe {
        *p = fn_ptr;
        *p.add(1) = env;
    }
    p as *mut c_void
}

/// Construct a union value. `size`/`align` come from
/// `MirTypeDef::Union.layout`; body layout is `[u16 tag | pad | payload]`
/// where the payload sits at byte offset 8 (matches `compute_union_layout`
/// in lower.rs).
#[unsafe(no_mangle)]
pub extern "C" fn otter_union_construct(
    size: i32,
    align: i32,
    type_id: i32,
    tag: i32,
    payload: *const c_void,
) -> *mut c_void {
    let p = alloc_with_header(size as usize, align as usize, type_id);
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
/// reads `type_id` at `recv - 8`, indexes a registered vtable for the
/// (struct, interface) pair, and returns the slot.
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
/// program's entry function under `otter_main`; the platform linker
/// resolves `main` to this shim.
#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    unsafe extern "C" {
        fn otter_main() -> i64;
    }
    let r = unsafe { otter_main() };
    r as i32
}

/// Allocate `[gc_header | type_id | body…]` with the body correctly
/// aligned, write the type id, and return a pointer to the body.
///
/// The header is normally 16 bytes (8 for GC bits, 8 for `type_id`),
/// but if `align > 16` we pad the header up to that alignment so the
/// body offset stays a multiple of `align`. Returned pointer always
/// satisfies `(returned - 8)` = type_id slot, regardless of padding.
fn alloc_with_header(size: usize, align: usize, type_id: i32) -> *mut c_void {
    // Pointers and the header itself need at least 8-byte alignment.
    let align = align.max(8);
    let header_size = ((PREFIX_BYTES + align - 1) / align) * align;
    let total = header_size + size;

    let layout = match std::alloc::Layout::from_size_align(total, align) {
        Ok(l) => l,
        Err(_) => std::process::abort(),
    };
    let raw = unsafe { std::alloc::alloc_zeroed(layout) };
    if raw.is_null() {
        std::process::abort();
    }

    // Fields are written by the caller starting at offset 0 of the
    // returned pointer. Type id sits in the 8 bytes immediately before.
    let value = unsafe { raw.add(header_size) };
    unsafe {
        *(value.sub(8) as *mut i32) = type_id;
    }
    value as *mut c_void
}
