pub const RUNTIME_PROFILE_VERSION: &str = "scriptc-deterministic-v1";
pub const HEAP_POLICY: &str = "per-refine bounded allocator; reset at guest entry";

pub fn selected_runtime_units() -> &'static [&'static str] {
    &[
        "scr_library.c",
        "scr_number.c",
        "scr_string.c",
        "scr_array.c",
        "scr_bytes.c",
        "scr_cycle.c",
        "scr_error.c",
        "scr_exception.c",
        "scr_object.c",
        "scr_lib_cleanup.c",
        "freestanding.c",
    ]
}
