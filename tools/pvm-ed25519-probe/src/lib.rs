#![no_std]

use ed25519_dalek::{Signature, VerifyingKey};

polkavm_derive::min_stack_size!(2 * 1024 * 1024);

struct ProbeAllocator;
static mut HEAP: [u8; 16 * 1024 * 1024] = [0; 16 * 1024 * 1024];
static mut HEAP_OFFSET: usize = 0;
unsafe impl core::alloc::GlobalAlloc for ProbeAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let base = core::ptr::addr_of_mut!(HEAP) as *mut u8 as usize;
        let offset = (HEAP_OFFSET + layout.align() - 1) & !(layout.align() - 1);
        let end = offset.saturating_add(layout.size());
        if end > 16 * 1024 * 1024 {
            core::ptr::null_mut()
        } else {
            HEAP_OFFSET = end;
            (base + offset) as *mut u8
        }
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}
#[global_allocator]
static ALLOCATOR: ProbeAllocator = ProbeAllocator;
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { core::arch::asm!(".4byte 0xc0001073", options(noreturn)) }
}

const PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
    0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
    0xf7, 0x07, 0x51, 0x1a,
];
const SIGNATURE: [u8; 64] = [
    0xe5, 0x56, 0x43, 0x00, 0xc3, 0x60, 0xac, 0x72, 0x90, 0x86, 0xe2, 0xcc, 0x80, 0x6e,
    0x82, 0x8a, 0x84, 0x87, 0x7f, 0x1e, 0xb8, 0xe5, 0xd9, 0x74, 0xd8, 0x73, 0xe0, 0x65,
    0x22, 0x49, 0x01, 0x55, 0x5f, 0xb8, 0x82, 0x15, 0x90, 0xa3, 0x3b, 0xac, 0xc6, 0x1e,
    0x39, 0x70, 0x1c, 0xf9, 0xb4, 0x6b, 0xd2, 0x5b, 0xf5, 0xf0, 0x59, 0x5b, 0xbe, 0x24,
    0x65, 0x51, 0x41, 0x43, 0x8e, 0x7a, 0x10, 0x0b,
];

#[no_mangle]
#[polkavm_derive::polkavm_export]
pub extern "C" fn probe_entry(stage: u64) -> u64 {
    let public = VerifyingKey::from_bytes(&PUBLIC_KEY).unwrap();
    if stage <= 1 {
        return 1;
    }
    let signature = Signature::from_bytes(&SIGNATURE);
    if stage <= 2 {
        return 1;
    }
    if stage <= 3 {
        return public.verify_strict(b"", &signature).is_ok() as u64;
    }
    public.verify_strict(b"", &signature).is_ok() as u64
}
