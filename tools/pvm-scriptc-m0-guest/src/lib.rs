#![no_std]

use core::alloc::{GlobalAlloc, Layout};

polkavm_derive::min_stack_size!(2 * 1024 * 1024);

const HEAP_SIZE: usize = 2 * 1024 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
static mut HEAP_OFFSET: usize = 0;
static mut ALLOCATION_COUNT: u64 = 0;
static mut REQUESTED_BYTES: u64 = 0;
static mut HIGH_WATER_MARK: u64 = 0;

struct ProbeAllocator;

unsafe impl GlobalAlloc for ProbeAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT = ALLOCATION_COUNT.saturating_add(1);
        REQUESTED_BYTES = REQUESTED_BYTES.saturating_add(layout.size() as u64);
        let base = core::ptr::addr_of_mut!(HEAP) as *mut u8 as usize;
        let offset = HEAP_OFFSET
            .checked_add(layout.align().saturating_sub(1))
            .map(|value| value & !layout.align().saturating_sub(1));
        let Some(offset) = offset else {
            return core::ptr::null_mut();
        };
        let Some(end) = offset.checked_add(layout.size()) else {
            return core::ptr::null_mut();
        };
        if end > HEAP_SIZE {
            core::ptr::null_mut()
        } else {
            HEAP_OFFSET = end;
            HIGH_WATER_MARK = HIGH_WATER_MARK.max(end as u64);
            (base + offset) as *mut u8
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: ProbeAllocator = ProbeAllocator;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { core::arch::asm!(".4byte 0xc0001073", options(noreturn)) }
}

extern "C" {
    fn jamscript_m0_scalar_init();
    fn jamscript_m0_scalar_entry(left: f64, right: f64) -> f64;
    fn jamscript_m0_u64_add(left: u64, right: u64) -> u64;
}

#[no_mangle]
#[polkavm_derive::polkavm_export]
pub extern "C" fn probe_entry(stage: u64) -> u64 {
    if stage == 1 {
        return 1;
    }
    if stage == 4 {
        return (unsafe { jamscript_m0_u64_add(41, 1) } == 42) as u64;
    }
    unsafe { jamscript_m0_scalar_init() };
    if stage == 2 {
        return 1;
    }
    let result = unsafe { jamscript_m0_scalar_entry(20.0, 22.0) };
    (result == 42.0) as u64
}

#[no_mangle]
pub extern "C" fn malloc(size: usize) -> *mut u8 {
    let layout = match Layout::from_size_align(size, 8) {
        Ok(layout) => layout,
        Err(_) => abort(),
    };
    let pointer = unsafe { ALLOCATOR.alloc(layout) };
    if pointer.is_null() {
        abort();
    }
    pointer
}

#[no_mangle]
pub extern "C" fn calloc(count: usize, size: usize) -> *mut u8 {
    let bytes = count.saturating_mul(size);
    let pointer = malloc(bytes);
    if !pointer.is_null() {
        unsafe { core::ptr::write_bytes(pointer, 0, bytes) };
    }
    pointer
}

#[no_mangle]
pub extern "C" fn realloc(_pointer: *mut u8, size: usize) -> *mut u8 {
    /* The scalar fixture never grows an allocated object. Keep this explicit
     * rather than pretending that a bump allocator has realloc semantics. */
    malloc(size)
}

#[no_mangle]
pub extern "C" fn free(_pointer: *mut u8) {}

#[no_mangle]
pub extern "C" fn abort() -> ! {
    unsafe { core::arch::asm!(".4byte 0xc0001073", options(noreturn)) }
}

#[no_mangle]
#[polkavm_derive::polkavm_export]
pub extern "C" fn probe_allocation_count() -> u64 {
    unsafe { ALLOCATION_COUNT }
}

#[no_mangle]
#[polkavm_derive::polkavm_export]
pub extern "C" fn probe_requested_bytes() -> u64 {
    unsafe { REQUESTED_BYTES }
}

#[no_mangle]
#[polkavm_derive::polkavm_export]
pub extern "C" fn probe_high_water_mark() -> u64 {
    unsafe { HIGH_WATER_MARK }
}

#[no_mangle]
pub unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, length: usize) -> *mut u8 {
    core::ptr::copy_nonoverlapping(source, destination, length);
    destination
}

#[no_mangle]
pub unsafe extern "C" fn memmove(
    destination: *mut u8,
    source: *const u8,
    length: usize,
) -> *mut u8 {
    core::ptr::copy(source, destination, length);
    destination
}

#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut u8, value: i32, length: usize) -> *mut u8 {
    core::ptr::write_bytes(destination, value as u8, length);
    destination
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, length: usize) -> i32 {
    for index in 0..length {
        let a = *left.add(index);
        let b = *right.add(index);
        if a != b {
            return a as i32 - b as i32;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strlen(value: *const u8) -> usize {
    let mut length = 0;
    while *value.add(length) != 0 {
        length += 1;
    }
    length
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(left: *const u8, right: *const u8) -> i32 {
    let mut index = 0;
    loop {
        let a = *left.add(index);
        let b = *right.add(index);
        if a != b || a == 0 {
            return a as i32 - b as i32;
        }
        index += 1;
    }
}

#[no_mangle]
pub extern "C" fn isnan(value: f64) -> i32 {
    (value != value) as i32
}

#[no_mangle]
pub extern "C" fn isinf(value: f64) -> i32 {
    let bits = value.to_bits();
    ((bits & 0x7fff_ffff_ffff_ffff) == 0x7ff0_0000_0000_0000) as i32
}

#[no_mangle]
pub unsafe extern "C" fn stpcpy(destination: *mut u8, source: *const u8) -> *mut u8 {
    let length = strlen(source);
    memcpy(destination, source, length + 1).add(length)
}

#[no_mangle]
pub extern "C" fn getenv(_name: *const u8) -> *mut u8 {
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn strtol(_value: *const u8, end: *mut *mut u8, _base: i32) -> i64 {
    if !end.is_null() {
        *end = _value as *mut u8;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn __assert_fail(
    _message: *const u8,
    _file: *const u8,
    _line: u32,
    _function: *const u8,
) -> ! {
    abort()
}
