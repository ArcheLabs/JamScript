use crate::allocator_abi::{self, CAllocationError, C_MAX_ALIGNMENT};
use buddy_system_allocator::{Heap, LockedHeap};
use core::{
    alloc::{GlobalAlloc, Layout},
    cmp::max,
    ptr::{self, NonNull},
};
use service_runtime_core::{
    GuestFaultCodeV1, GuestFaultRecordV1, GuestFaultStageV1, GuestMemoryBudgetV1,
    GUEST_FAULT_LOG_MESSAGE_CAPACITY_V1, GUEST_HEAP_PLATFORM_MAX_BYTES, GUEST_MEMORY_PAGE_BYTES,
};

const MAX_BUDDY_ORDER: usize = 32;
const GROWTH_QUANTUM_BYTES: usize = GUEST_MEMORY_PAGE_BYTES as usize;

#[global_allocator]
static ALLOCATOR: RuntimeAllocator = RuntimeAllocator;
static HEAP: LockedHeap<MAX_BUDDY_ORDER> = LockedHeap::empty();

static mut MEMORY_BUDGET: GuestMemoryBudgetV1 = GuestMemoryBudgetV1::DEFAULT;
static mut HEAP_BASE: usize = 0;
static mut HEAP_COMMITTED_BYTES: u32 = 0;
static mut RUNTIME_INITIALIZED: bool = false;
static mut LIVE_BYTES: u32 = 0;
static mut HIGH_WATER_BYTES: u32 = 0;
static mut CUMULATIVE_BYTES: u32 = 0;

#[no_mangle]
pub static mut JAMSCRIPT_GUEST_FAULT_RECORD_V1: GuestFaultRecordV1 = GuestFaultRecordV1::EMPTY;

struct RuntimeAllocator;

unsafe impl GlobalAlloc for RuntimeAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }
        // Some Formal V1 runtimes allocate while entering the exported guest
        // function, before generated JamScript code can install its configured
        // budget. Initialize one page lazily so those runtime allocations do
        // not grow from a null base or pre-commit more than a project's budget.
        if !unsafe { RUNTIME_INITIALIZED }
            && !reset_runtime(
                GuestMemoryBudgetV1 {
                    heap_initial_bytes: GUEST_MEMORY_PAGE_BYTES,
                    heap_max_bytes: GUEST_MEMORY_PAGE_BYTES,
                },
                GuestFaultStageV1::Invocation,
            )
        {
            return ptr::null_mut();
        }
        let mut heap = HEAP.lock();
        match allocate_with_growth(&mut heap, layout) {
            Ok(pointer) => {
                record_allocation(layout.size());
                pointer.as_ptr()
            }
            Err(code) => {
                record_failure(code, layout.size(), layout.align());
                ptr::null_mut()
            }
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer.is_null() || layout.size() == 0 {
            return;
        }
        let Some(pointer) = NonNull::new(pointer) else {
            return;
        };
        unsafe { HEAP.lock().dealloc(pointer, layout) };
        record_deallocation(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        if pointer.is_null() {
            let Ok(layout) = Layout::from_size_align(new_size, old_layout.align()) else {
                record_failure(
                    GuestFaultCodeV1::MemoryAllocationFailed,
                    new_size,
                    old_layout.align(),
                );
                return ptr::null_mut();
            };
            return unsafe { self.alloc(layout) };
        }
        if new_size == 0 {
            unsafe { self.dealloc(pointer, old_layout) };
            return ptr::null_mut();
        }
        let Ok(new_layout) = Layout::from_size_align(new_size, old_layout.align()) else {
            record_failure(
                GuestFaultCodeV1::MemoryAllocationFailed,
                new_size,
                old_layout.align(),
            );
            return ptr::null_mut();
        };
        unsafe { allocator_abi::realloc_preserving(self, pointer, old_layout, new_layout) }
    }
}

fn allocate_with_growth(
    heap: &mut Heap<MAX_BUDDY_ORDER>,
    layout: Layout,
) -> Result<NonNull<u8>, GuestFaultCodeV1> {
    // buddy_system_allocator computes the next power of two internally with
    // an unchecked operation. Reject impossible sizes before entering it so a
    // malicious or accidental usize-sized request becomes a stable guest
    // fault instead of a debug panic or an out-of-range size-class access.
    let requested_block = max(
        layout.size(),
        max(layout.align(), core::mem::size_of::<usize>()),
    );
    let Some(requested_block) = requested_block.checked_next_power_of_two() else {
        return Err(GuestFaultCodeV1::HeapLimitExceeded);
    };
    if requested_block > unsafe { MEMORY_BUDGET.heap_max_bytes as usize } {
        return Err(GuestFaultCodeV1::HeapLimitExceeded);
    }
    loop {
        if let Ok(pointer) = heap.alloc(layout) {
            return Ok(pointer);
        }
        grow_heap(heap, 0, Some(layout))?;
    }
}

fn grow_heap(
    heap: &mut Heap<MAX_BUDDY_ORDER>,
    minimum_growth_bytes: usize,
    failed_layout: Option<Layout>,
) -> Result<(), GuestFaultCodeV1> {
    let budget = unsafe { MEMORY_BUDGET };
    let committed = unsafe { HEAP_COMMITTED_BYTES };
    let remaining = budget.heap_max_bytes.saturating_sub(committed);
    if remaining == 0 {
        return Err(classify_budget_exhaustion(
            failed_layout,
            budget.heap_max_bytes,
        ));
    }

    let current = polkavm_derive::sbrk(0) as usize;
    let required = if let Some(layout) = failed_layout {
        let minimum = max(
            layout.size(),
            max(layout.align(), core::mem::size_of::<usize>()),
        );
        let block = minimum
            .checked_next_power_of_two()
            .ok_or(GuestFaultCodeV1::MemoryAllocationFailed)?;
        if block > budget.heap_max_bytes as usize {
            return Err(GuestFaultCodeV1::HeapLimitExceeded);
        }
        // `Heap::add_to_heap` keeps each newly added range as a separate set
        // of buddy blocks. Include the address gap before the next aligned
        // block so an unaligned `sbrk` break cannot strand every growth chunk
        // below the requested size class (for example, a 1 MiB block in a
        // sequence of 1 MiB chunks whose base is only page aligned).
        let alignment_gap = (block - current % block) % block;
        alignment_gap
            .checked_add(block.max(GROWTH_QUANTUM_BYTES))
            .ok_or(GuestFaultCodeV1::MemoryGrowFailed)?
    } else {
        minimum_growth_bytes
    };
    let page = GUEST_MEMORY_PAGE_BYTES as usize;
    let rounded = required
        .checked_add(page - 1)
        .ok_or(GuestFaultCodeV1::MemoryAllocationFailed)?
        / page
        * page;
    let growth = rounded.min(remaining as usize);
    if growth == 0 {
        return Err(classify_budget_exhaustion(
            failed_layout,
            budget.heap_max_bytes,
        ));
    }

    let base = unsafe { HEAP_BASE };
    let expected = base
        .checked_add(committed as usize)
        .ok_or(GuestFaultCodeV1::MemoryGrowFailed)?;
    if current != expected {
        return Err(GuestFaultCodeV1::MemoryGrowFailed);
    }
    let Some(end) = current.checked_add(growth) else {
        return Err(GuestFaultCodeV1::MemoryGrowFailed);
    };
    if end > u32::MAX as usize || growth > u32::MAX as usize {
        return Err(GuestFaultCodeV1::MemoryGrowFailed);
    }

    // The MiniJAM runner returns the old break for sbrk(n), while the local
    // PolkaVM interpreter returns the new break. Both return zero on failure;
    // this allocator intentionally checks only that sentinel.
    let result = polkavm_derive::sbrk(growth);
    if result.is_null() {
        return Err(GuestFaultCodeV1::MemoryGrowFailed);
    }
    unsafe { heap.add_to_heap(current, end) };
    let committed = committed
        .checked_add(growth as u32)
        .ok_or(GuestFaultCodeV1::MemoryGrowFailed)?;
    unsafe {
        HEAP_COMMITTED_BYTES = committed;
        update_fault_metrics();
    }
    Ok(())
}

fn classify_budget_exhaustion(layout: Option<Layout>, heap_max_bytes: u32) -> GuestFaultCodeV1 {
    let request = layout.map_or(0, |layout| layout.size().min(u32::MAX as usize) as u32);
    if request > heap_max_bytes {
        GuestFaultCodeV1::HeapLimitExceeded
    } else {
        let live = unsafe { LIVE_BYTES };
        if live.saturating_add(request) > heap_max_bytes {
            GuestFaultCodeV1::HeapLimitExceeded
        } else {
            GuestFaultCodeV1::MemoryAllocationFailed
        }
    }
}

pub fn reset_runtime(budget: GuestMemoryBudgetV1, stage: GuestFaultStageV1) -> bool {
    unsafe {
        JAMSCRIPT_GUEST_FAULT_RECORD_V1 = GuestFaultRecordV1 {
            heap_max_bytes: budget.heap_max_bytes,
            stage: stage as u32,
            ..GuestFaultRecordV1::EMPTY
        };
        MEMORY_BUDGET = budget;
        HEAP_BASE = 0;
        HEAP_COMMITTED_BYTES = 0;
        RUNTIME_INITIALIZED = false;
        LIVE_BYTES = 0;
        HIGH_WATER_BYTES = 0;
        CUMULATIVE_BYTES = 0;
    }
    if budget.validate().is_err()
        || budget.heap_max_bytes > GUEST_HEAP_PLATFORM_MAX_BYTES
        || budget.heap_max_bytes > usize::MAX as u32
    {
        record_failure(GuestFaultCodeV1::MemoryConfigInvalid, 0, 0);
        return false;
    }

    let base = polkavm_derive::heap_base() as usize;
    let current = polkavm_derive::sbrk(0) as usize;
    let Some(existing_bytes) = current.checked_sub(base) else {
        record_failure(GuestFaultCodeV1::MemoryGrowFailed, 0, 0);
        return false;
    };
    let Ok(existing_bytes) = u32::try_from(existing_bytes) else {
        record_failure(GuestFaultCodeV1::MemoryGrowFailed, 0, 0);
        return false;
    };
    if existing_bytes > budget.heap_max_bytes {
        record_failure(
            GuestFaultCodeV1::MemoryConfigInvalid,
            existing_bytes as usize,
            GUEST_MEMORY_PAGE_BYTES as usize,
        );
        return false;
    }
    unsafe {
        HEAP_BASE = base;
        HEAP_COMMITTED_BYTES = existing_bytes;
        *HEAP.lock() = Heap::new();
        RUNTIME_INITIALIZED = true;
    }

    let mut heap = HEAP.lock();
    if existing_bytes != 0 {
        unsafe { heap.add_to_heap(base, current) };
    }
    if existing_bytes >= budget.heap_initial_bytes {
        return true;
    }
    if grow_heap(&mut heap, budget.heap_initial_bytes as usize, None).is_err() {
        unsafe { RUNTIME_INITIALIZED = false };
        record_failure(
            GuestFaultCodeV1::MemoryGrowFailed,
            budget.heap_initial_bytes as usize,
            1,
        );
        return false;
    }
    true
}

pub fn fault_record() -> GuestFaultRecordV1 {
    unsafe { ptr::read_volatile(ptr::addr_of!(JAMSCRIPT_GUEST_FAULT_RECORD_V1)) }
}

pub fn fault_record_ptr() -> *const u8 {
    ptr::addr_of!(JAMSCRIPT_GUEST_FAULT_RECORD_V1).cast::<u8>()
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn jamscript_guest_fault_record_v1() -> *const u8 {
    fault_record_ptr()
}

pub fn has_fault() -> bool {
    fault_record().code != 0
}

pub fn record_failure(code: GuestFaultCodeV1, requested_bytes: usize, alignment: usize) {
    unsafe {
        let record = &mut *ptr::addr_of_mut!(JAMSCRIPT_GUEST_FAULT_RECORD_V1);
        if record.code == 0 {
            record.code = code as u32;
            record.requested_bytes = requested_bytes.min(u32::MAX as usize) as u32;
            record.alignment = alignment.min(u32::MAX as usize) as u32;
        }
        update_fault_metrics();
    }
}

fn update_failure_request(requested_bytes: usize, alignment: usize) {
    unsafe {
        let record = &mut *ptr::addr_of_mut!(JAMSCRIPT_GUEST_FAULT_RECORD_V1);
        record.requested_bytes = requested_bytes.min(u32::MAX as usize) as u32;
        record.alignment = alignment.min(u32::MAX as usize) as u32;
        update_fault_metrics();
    }
}

unsafe fn update_fault_metrics() {
    let record = &mut *ptr::addr_of_mut!(JAMSCRIPT_GUEST_FAULT_RECORD_V1);
    record.heap_committed_bytes = HEAP_COMMITTED_BYTES;
    record.heap_max_bytes = MEMORY_BUDGET.heap_max_bytes;
    record.live_requested_bytes = LIVE_BYTES;
    record.high_water_requested_bytes = HIGH_WATER_BYTES;
    record.cumulative_requested_bytes = CUMULATIVE_BYTES;
}

fn record_allocation(bytes: usize) {
    let bytes = bytes.min(u32::MAX as usize) as u32;
    unsafe {
        LIVE_BYTES = LIVE_BYTES.saturating_add(bytes);
        HIGH_WATER_BYTES = HIGH_WATER_BYTES.max(LIVE_BYTES);
        CUMULATIVE_BYTES = CUMULATIVE_BYTES.saturating_add(bytes);
        update_fault_metrics();
    }
}

fn record_deallocation(bytes: usize) {
    let bytes = bytes.min(u32::MAX as usize) as u32;
    unsafe {
        LIVE_BYTES = LIVE_BYTES.saturating_sub(bytes);
        update_fault_metrics();
    }
}

pub unsafe fn c_malloc(size: usize) -> *mut u8 {
    let fault_before = has_fault();
    match unsafe { allocator_abi::malloc_from(&ALLOCATOR, size) } {
        Ok(pointer) => pointer.as_ptr(),
        Err(CAllocationError::SizeOverflow) => {
            record_failure(
                GuestFaultCodeV1::MemoryAllocationFailed,
                size,
                C_MAX_ALIGNMENT,
            );
            ptr::null_mut()
        }
        Err(CAllocationError::OutOfMemory) => {
            if !fault_before {
                update_failure_request(size, C_MAX_ALIGNMENT);
            }
            ptr::null_mut()
        }
        Err(CAllocationError::InvalidPointer) => {
            record_failure(
                GuestFaultCodeV1::MemoryAllocationFailed,
                size,
                C_MAX_ALIGNMENT,
            );
            ptr::null_mut()
        }
    }
}

pub unsafe fn c_calloc(count: usize, size: usize) -> *mut u8 {
    let fault_before = has_fault();
    match unsafe { allocator_abi::calloc_from(&ALLOCATOR, count, size) } {
        Ok(pointer) => pointer.as_ptr(),
        Err(CAllocationError::SizeOverflow) => {
            record_failure(
                GuestFaultCodeV1::MemoryAllocationFailed,
                usize::MAX,
                C_MAX_ALIGNMENT,
            );
            ptr::null_mut()
        }
        Err(CAllocationError::OutOfMemory) => {
            if !fault_before {
                let requested = count.saturating_mul(size);
                update_failure_request(requested, C_MAX_ALIGNMENT);
            }
            ptr::null_mut()
        }
        Err(CAllocationError::InvalidPointer) => ptr::null_mut(),
    }
}

pub unsafe fn c_realloc(pointer: *mut u8, size: usize) -> *mut u8 {
    let fault_before = has_fault();
    match unsafe { allocator_abi::realloc_from(&ALLOCATOR, pointer, size) } {
        Ok(Some(pointer)) => pointer.as_ptr(),
        Ok(None) => ptr::null_mut(),
        Err(CAllocationError::InvalidPointer) | Err(CAllocationError::SizeOverflow) => {
            record_failure(
                GuestFaultCodeV1::MemoryAllocationFailed,
                size,
                C_MAX_ALIGNMENT,
            );
            ptr::null_mut()
        }
        Err(CAllocationError::OutOfMemory) => {
            if !fault_before {
                update_failure_request(size, C_MAX_ALIGNMENT);
            }
            ptr::null_mut()
        }
    }
}

pub unsafe fn c_free(pointer: *mut u8) {
    // Arbitrary and stale pointers remain undefined behavior. A readable
    // invalid header is ignored; the allocator cannot validate every pointer.
    let _ = unsafe { allocator_abi::free_from(&ALLOCATOR, pointer) };
}

pub fn emit_fault_record() {
    unsafe extern "C" {
        fn minijam_host_call(call: u32, args: *const u64) -> u64;
    }
    let record = fault_record();
    if record.code == 0 {
        return;
    }
    let mut message = [0u8; GUEST_FAULT_LOG_MESSAGE_CAPACITY_V1];
    let length = record
        .write_log_message_v1(&mut message)
        .unwrap_or_else(|| {
            // Keep a visible diagnostic if the format ever grows beyond its
            // reviewed capacity; never pass a syntactically valid truncated value.
            const TRUNCATED: &[u8] = b"JSGF;v=1;error=log-record-truncated";
            message[..TRUNCATED.len()].copy_from_slice(TRUNCATED);
            TRUNCATED.len()
        });
    let args = [
        1u64,
        0,
        0,
        message.as_ptr() as usize as u64,
        length as u64,
        0,
    ];
    unsafe { minijam_host_call(100, args.as_ptr()) };
}

pub fn trap_with_fault_record() -> ! {
    emit_fault_record();
    unsafe {
        core::arch::asm!(".4byte 0xc0001073", options(noreturn));
    }
}
