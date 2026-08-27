#![no_std]

use service_runtime_state::{empty_state_root, ManagedState, ProofState};

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

#[no_mangle]
#[polkavm_derive::polkavm_export]
pub extern "C" fn probe_entry(stage: u64) -> u64 {
    let parent = empty_state_root();
    let mut state = ProofState::from_witness(parent, &[]).unwrap();
    if stage <= 1 {
        return 1;
    }
    if state.get(b"counter").unwrap().is_some() {
        return 0;
    }
    if stage <= 2 {
        return 1;
    }
    state.begin_transaction();
    state.set(b"counter", &[1]).unwrap();
    if stage <= 3 {
        state.rollback_transaction().unwrap();
        return 1;
    }
    state.commit_transaction().unwrap();
    let (next, _) = state.finish().unwrap();
    (next != parent) as u64
}
