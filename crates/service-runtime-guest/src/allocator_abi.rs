use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{self, NonNull},
};

pub const C_ALLOCATION_MAGIC: u32 = 0x4a53_4354;
pub const C_MAX_ALIGNMENT: usize = 16;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct CAllocationHeader {
    pub magic: u32,
    pub size: u32,
}

pub const C_HEADER_SIZE: usize = core::mem::size_of::<CAllocationHeader>();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CAllocationError {
    SizeOverflow,
    InvalidPointer,
    OutOfMemory,
}

pub fn calloc_size(count: usize, size: usize) -> Result<usize, CAllocationError> {
    count
        .checked_mul(size)
        .ok_or(CAllocationError::SizeOverflow)
}

pub fn allocation_layout(size: usize) -> Result<Layout, CAllocationError> {
    let total = C_HEADER_SIZE
        .checked_add(size.max(1))
        .ok_or(CAllocationError::SizeOverflow)?;
    Layout::from_size_align(total, C_MAX_ALIGNMENT).map_err(|_| CAllocationError::SizeOverflow)
}

/// Allocate C-compatible storage using the allocator selected by the guest.
/// A zero-size C allocation receives a distinct one-byte payload.
pub unsafe fn malloc_from<A: GlobalAlloc>(
    allocator: &A,
    size: usize,
) -> Result<NonNull<u8>, CAllocationError> {
    let layout = allocation_layout(size)?;
    let base =
        NonNull::new(unsafe { allocator.alloc(layout) }).ok_or(CAllocationError::OutOfMemory)?;
    unsafe {
        base.cast::<CAllocationHeader>().write(CAllocationHeader {
            magic: C_ALLOCATION_MAGIC,
            size: size.min(u32::MAX as usize) as u32,
        });
    }
    Ok(unsafe { NonNull::new_unchecked(base.as_ptr().add(C_HEADER_SIZE)) })
}

pub unsafe fn calloc_from<A: GlobalAlloc>(
    allocator: &A,
    count: usize,
    size: usize,
) -> Result<NonNull<u8>, CAllocationError> {
    let total = calloc_size(count, size)?;
    let pointer = unsafe { malloc_from(allocator, total)? };
    unsafe { ptr::write_bytes(pointer.as_ptr(), 0, total) };
    Ok(pointer)
}

/// Reallocate C-compatible storage. A failed allocation leaves the old block
/// valid; size zero frees the old block and returns `None`.
pub unsafe fn realloc_from<A: GlobalAlloc>(
    allocator: &A,
    pointer: *mut u8,
    size: usize,
) -> Result<Option<NonNull<u8>>, CAllocationError> {
    if pointer.is_null() {
        return if size == 0 {
            Ok(None)
        } else {
            unsafe { malloc_from(allocator, size) }.map(Some)
        };
    }
    let Some(payload) = NonNull::new(pointer) else {
        return Err(CAllocationError::InvalidPointer);
    };
    let header = unsafe {
        payload
            .as_ptr()
            .sub(C_HEADER_SIZE)
            .cast::<CAllocationHeader>()
    };
    if unsafe { (*header).magic } != C_ALLOCATION_MAGIC {
        return Err(CAllocationError::InvalidPointer);
    }
    let old_size = unsafe { (*header).size as usize };
    let old_layout = allocation_layout(old_size)?;
    let old_base = unsafe { payload.as_ptr().sub(C_HEADER_SIZE) };
    if size == 0 {
        unsafe { free_from(allocator, pointer) }?;
        return Ok(None);
    }
    let new_layout = allocation_layout(size)?;
    let Some(new_base) =
        NonNull::new(unsafe { realloc_preserving(allocator, old_base, old_layout, new_layout) })
    else {
        return Err(CAllocationError::OutOfMemory);
    };
    unsafe {
        new_base
            .cast::<CAllocationHeader>()
            .write(CAllocationHeader {
                magic: C_ALLOCATION_MAGIC,
                size: size.min(u32::MAX as usize) as u32,
            });
    }
    Ok(Some(unsafe {
        NonNull::new_unchecked(new_base.as_ptr().add(C_HEADER_SIZE))
    }))
}

/// The shared fallback used by Rust `GlobalAlloc::realloc` and C `realloc`.
pub unsafe fn realloc_preserving<A: GlobalAlloc>(
    allocator: &A,
    pointer: *mut u8,
    old_layout: Layout,
    new_layout: Layout,
) -> *mut u8 {
    let replacement = unsafe { allocator.alloc(new_layout) };
    if replacement.is_null() {
        return ptr::null_mut();
    }
    if !pointer.is_null() {
        unsafe {
            ptr::copy_nonoverlapping(
                pointer,
                replacement,
                old_layout.size().min(new_layout.size()),
            );
            allocator.dealloc(pointer, old_layout);
        }
    }
    replacement
}

pub unsafe fn free_from<A: GlobalAlloc>(
    allocator: &A,
    pointer: *mut u8,
) -> Result<(), CAllocationError> {
    let Some(payload) = NonNull::new(pointer) else {
        return Ok(());
    };
    let header = unsafe {
        payload
            .as_ptr()
            .sub(C_HEADER_SIZE)
            .cast::<CAllocationHeader>()
    };
    if unsafe { (*header).magic } != C_ALLOCATION_MAGIC {
        return Err(CAllocationError::InvalidPointer);
    }
    let size = unsafe { (*header).size as usize };
    let layout = allocation_layout(size)?;
    unsafe {
        (*header).magic = 0;
        allocator.dealloc(header.cast::<u8>(), layout);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buddy_system_allocator::Heap;
    use std::sync::Mutex;

    const ARENA_BYTES: usize = 1024 * 1024;

    #[repr(align(65536))]
    struct Arena([u8; ARENA_BYTES]);

    static mut TEST_ARENA: Arena = Arena([0; ARENA_BYTES]);

    struct TestAllocator {
        arena: *mut u8,
        heap: Mutex<Heap<32>>,
    }

    unsafe impl Send for TestAllocator {}
    unsafe impl Sync for TestAllocator {}

    impl TestAllocator {
        unsafe fn new(arena: *mut Arena) -> Self {
            let arena_bytes = unsafe { &mut (*arena).0 };
            let start = arena_bytes.as_mut_ptr() as usize;
            let mut heap = Heap::new();
            unsafe { heap.add_to_heap(start, start + ARENA_BYTES) };
            Self {
                arena: arena_bytes.as_mut_ptr(),
                heap: Mutex::new(heap),
            }
        }
    }

    unsafe impl GlobalAlloc for TestAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            self.heap
                .lock()
                .unwrap()
                .alloc(layout)
                .map_or(ptr::null_mut(), |pointer| pointer.as_ptr())
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            if let Some(pointer) = NonNull::new(pointer) {
                unsafe { self.heap.lock().unwrap().dealloc(pointer, layout) };
            }
        }
    }

    #[test]
    fn c_abi_alignment_zeroing_reuse_coalescing_and_realloc_are_deterministic() {
        let allocator = unsafe { TestAllocator::new(ptr::addr_of_mut!(TEST_ARENA)) };

        let zero = unsafe { malloc_from(&allocator, 0) }.unwrap();
        assert_eq!(zero.as_ptr() as usize % C_MAX_ALIGNMENT, 0);
        unsafe { free_from(&allocator, zero.as_ptr()) }.unwrap();
        let reused = unsafe { malloc_from(&allocator, 0) }.unwrap();
        assert_eq!(reused, zero, "freed C blocks should be reused");
        unsafe { free_from(&allocator, reused.as_ptr()) }.unwrap();

        let zeroed = unsafe { calloc_from(&allocator, 64, 4) }.unwrap();
        assert!(unsafe { core::slice::from_raw_parts(zeroed.as_ptr(), 256) }
            .iter()
            .all(|byte| *byte == 0));

        let grown = unsafe { realloc_from(&allocator, zeroed.as_ptr(), 768) }
            .unwrap()
            .unwrap();
        assert!(unsafe { core::slice::from_raw_parts(grown.as_ptr(), 256) }
            .iter()
            .all(|byte| *byte == 0));
        unsafe { core::ptr::write_bytes(grown.as_ptr(), 0x5a, 256) };

        let failed = unsafe { realloc_from(&allocator, grown.as_ptr(), ARENA_BYTES * 2) };
        assert_eq!(failed, Err(CAllocationError::OutOfMemory));
        assert!(unsafe { core::slice::from_raw_parts(grown.as_ptr(), 256) }
            .iter()
            .all(|byte| *byte == 0x5a));

        let shrink = unsafe { realloc_from(&allocator, grown.as_ptr(), 128) }
            .unwrap()
            .unwrap();
        assert!(unsafe { core::slice::from_raw_parts(shrink.as_ptr(), 128) }
            .iter()
            .all(|byte| *byte == 0x5a));
        unsafe { free_from(&allocator, shrink.as_ptr()) }.unwrap();

        assert_eq!(
            calloc_size(usize::MAX, 2),
            Err(CAllocationError::SizeOverflow)
        );

        let left = unsafe { allocator.alloc(Layout::from_size_align(1024, 16).unwrap()) };
        let right = unsafe { allocator.alloc(Layout::from_size_align(1024, 16).unwrap()) };
        assert!(!left.is_null() && !right.is_null());
        let left_layout = Layout::from_size_align(1024, 16).unwrap();
        unsafe {
            allocator.dealloc(left, left_layout);
            allocator.dealloc(right, left_layout);
        }
        let merged = unsafe { allocator.alloc(Layout::from_size_align(2048, 16).unwrap()) };
        assert!(!merged.is_null(), "freed buddy blocks should coalesce");
        assert!(merged as usize >= allocator.arena as usize);
        assert!(merged as usize + 2048 <= allocator.arena as usize + ARENA_BYTES);
    }
}
