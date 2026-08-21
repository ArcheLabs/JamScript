#![no_std]

pub const RUNTIME_VERSION: &str = "0.1.0";
pub const MAX_ACTION_BYTES: usize = 1_048_576;

#[repr(C)]
pub struct RefineOutput {
    pub data: *const u8,
    pub size: usize,
}

pub fn checked_add_u64(left: u64, right: u64) -> Option<u64> {
    left.checked_add(right)
}
