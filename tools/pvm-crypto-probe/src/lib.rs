#![no_std]

use jamscript_crypto::verify_sr25519;
use schnorrkel::{context::signing_context, PublicKey, Signature};

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
    0x7c, 0x0f, 0x46, 0x9d, 0x3b, 0xd3, 0x40, 0xba, 0xe7, 0x18, 0x20, 0x3f, 0xa3, 0x0c,
    0xa0, 0x71, 0xa5, 0xe3, 0x7c, 0x75, 0x1e, 0x89, 0x1d, 0xbd, 0xed, 0x83, 0x7b, 0x21,
    0x3d, 0x45, 0xd9, 0x1d,
];
const SIGNATURE: [u8; 64] = [
    0x80, 0x84, 0x0c, 0xf6, 0xba, 0xce, 0x8e, 0x88, 0x71, 0x28, 0x1a, 0x67, 0xb7, 0x25,
    0xc7, 0xbd, 0x8f, 0xa0, 0x73, 0xca, 0x93, 0xd6, 0x4f, 0x5b, 0x7f, 0x06, 0xab, 0x59,
    0x53, 0x64, 0x50, 0x39, 0x41, 0x47, 0xbd, 0xf7, 0x1d, 0xfc, 0x19, 0xac, 0xc5, 0x3a,
    0x9c, 0x48, 0xe7, 0xe7, 0xac, 0x5b, 0x49, 0xc1, 0xf0, 0xc5, 0x19, 0x6a, 0x60, 0xdb,
    0xbb, 0xcb, 0xba, 0xc7, 0xcc, 0xa6, 0xdb, 0x8c,
];
const MESSAGE: &[u8] = b"deterministic message";

#[no_mangle]
#[polkavm_derive::polkavm_export]
pub extern "C" fn probe_entry(stage: u64) -> u64 {
    let public = PublicKey::from_bytes(&PUBLIC_KEY).unwrap();
    if stage <= 1 {
        return 1;
    }
    let signature = Signature::from_bytes(&SIGNATURE).unwrap();
    if stage <= 2 {
        return 1;
    }
    let transcript = signing_context(b"substrate").bytes(MESSAGE);
    if stage <= 3 {
        return public.verify(transcript, &signature).is_ok() as u64;
    }
    verify_sr25519(&PUBLIC_KEY, &SIGNATURE, MESSAGE).is_ok() as u64
}
