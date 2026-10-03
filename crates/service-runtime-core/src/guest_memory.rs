//! Stable memory-budget and fatal guest-fault records shared by the build,
//! guest runtime, Backend, and diagnostic clients.

pub const GUEST_MEMORY_PAGE_BYTES: u32 = 4096;
pub const DEFAULT_GUEST_HEAP_INITIAL_BYTES: u32 = 1024 * 1024;
pub const DEFAULT_GUEST_HEAP_MAX_BYTES: u32 = 16 * 1024 * 1024;
/// JamScript policy cap. The PVM address-space and stack-derived limits can be
/// lower for a particular artifact and are checked independently by the
/// builder and again by `sbrk` at runtime.
pub const GUEST_HEAP_PLATFORM_MAX_BYTES: u32 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestMemoryBudgetV1 {
    pub heap_initial_bytes: u32,
    pub heap_max_bytes: u32,
}

impl GuestMemoryBudgetV1 {
    pub const DEFAULT: Self = Self {
        heap_initial_bytes: DEFAULT_GUEST_HEAP_INITIAL_BYTES,
        heap_max_bytes: DEFAULT_GUEST_HEAP_MAX_BYTES,
    };

    pub fn validate(self) -> Result<Self, GuestMemoryBudgetError> {
        if self.heap_initial_bytes == 0 {
            return Err(GuestMemoryBudgetError::InitialMustBePositive);
        }
        if self.heap_max_bytes == 0 {
            return Err(GuestMemoryBudgetError::MaximumMustBePositive);
        }
        if !self
            .heap_initial_bytes
            .is_multiple_of(GUEST_MEMORY_PAGE_BYTES)
        {
            return Err(GuestMemoryBudgetError::InitialNotPageAligned(
                self.heap_initial_bytes,
            ));
        }
        if !self.heap_max_bytes.is_multiple_of(GUEST_MEMORY_PAGE_BYTES) {
            return Err(GuestMemoryBudgetError::MaximumNotPageAligned(
                self.heap_max_bytes,
            ));
        }
        if self.heap_initial_bytes > self.heap_max_bytes {
            return Err(GuestMemoryBudgetError::InitialExceedsMaximum {
                initial: self.heap_initial_bytes,
                maximum: self.heap_max_bytes,
            });
        }
        if self.heap_max_bytes > GUEST_HEAP_PLATFORM_MAX_BYTES {
            return Err(GuestMemoryBudgetError::MaximumExceedsPlatform {
                maximum: self.heap_max_bytes,
                platform_maximum: GUEST_HEAP_PLATFORM_MAX_BYTES,
            });
        }
        Ok(self)
    }
}

impl Default for GuestMemoryBudgetV1 {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestMemoryBudgetError {
    InitialMustBePositive,
    MaximumMustBePositive,
    InitialNotPageAligned(u32),
    MaximumNotPageAligned(u32),
    InitialExceedsMaximum { initial: u32, maximum: u32 },
    MaximumExceedsPlatform { maximum: u32, platform_maximum: u32 },
}

pub const GUEST_FAULT_MAGIC_V1: [u8; 4] = *b"JSGF";
pub const GUEST_FAULT_RECORD_VERSION_V1: u16 = 1;
pub const GUEST_FAULT_RECORD_V1_LEN: usize = 44;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GuestFaultCodeV1 {
    HeapLimitExceeded = 1,
    MemoryGrowFailed = 2,
    MemoryAllocationFailed = 3,
    Panic = 4,
    MemoryConfigInvalid = 5,
    Abort = 6,
}

impl GuestFaultCodeV1 {
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::HeapLimitExceeded),
            2 => Some(Self::MemoryGrowFailed),
            3 => Some(Self::MemoryAllocationFailed),
            4 => Some(Self::Panic),
            5 => Some(Self::MemoryConfigInvalid),
            6 => Some(Self::Abort),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HeapLimitExceeded => "GUEST_HEAP_LIMIT_EXCEEDED",
            Self::MemoryGrowFailed => "GUEST_MEMORY_GROW_FAILED",
            Self::MemoryAllocationFailed => "GUEST_MEMORY_ALLOCATION_FAILED",
            Self::Panic => "GUEST_PANIC",
            Self::MemoryConfigInvalid => "GUEST_MEMORY_CONFIG_INVALID",
            Self::Abort => "GUEST_ABORT",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GuestFaultStageV1 {
    Invocation = 0,
    Plan = 1,
    Refine = 2,
}

impl GuestFaultStageV1 {
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Invocation),
            1 => Some(Self::Plan),
            2 => Some(Self::Refine),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invocation => "invocation",
            Self::Plan => "plan",
            Self::Refine => "refine",
        }
    }
}

/// Fixed-width little-endian diagnostic record. It lives outside the managed
/// guest heap, so it remains readable after an allocation failure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct GuestFaultRecordV1 {
    pub magic: [u8; 4],
    pub version: u16,
    pub length: u16,
    pub code: u32,
    pub stage: u32,
    pub requested_bytes: u32,
    pub alignment: u32,
    pub heap_committed_bytes: u32,
    pub heap_max_bytes: u32,
    pub live_requested_bytes: u32,
    pub high_water_requested_bytes: u32,
    pub cumulative_requested_bytes: u32,
}

impl GuestFaultRecordV1 {
    pub const EMPTY: Self = Self {
        magic: GUEST_FAULT_MAGIC_V1,
        version: GUEST_FAULT_RECORD_VERSION_V1,
        length: GUEST_FAULT_RECORD_V1_LEN as u16,
        code: 0,
        stage: GuestFaultStageV1::Invocation as u32,
        requested_bytes: 0,
        alignment: 0,
        heap_committed_bytes: 0,
        heap_max_bytes: 0,
        live_requested_bytes: 0,
        high_water_requested_bytes: 0,
        cumulative_requested_bytes: 0,
    };

    pub fn encode(self) -> [u8; GUEST_FAULT_RECORD_V1_LEN] {
        let mut output = [0u8; GUEST_FAULT_RECORD_V1_LEN];
        output[..4].copy_from_slice(&self.magic);
        output[4..6].copy_from_slice(&self.version.to_le_bytes());
        output[6..8].copy_from_slice(&self.length.to_le_bytes());
        let fields = [
            self.code,
            self.stage,
            self.requested_bytes,
            self.alignment,
            self.heap_committed_bytes,
            self.heap_max_bytes,
            self.live_requested_bytes,
            self.high_water_requested_bytes,
            self.cumulative_requested_bytes,
        ];
        for (index, field) in fields.into_iter().enumerate() {
            let start = 8 + index * 4;
            output[start..start + 4].copy_from_slice(&field.to_le_bytes());
        }
        output
    }

    pub fn decode(input: &[u8]) -> Result<Self, GuestFaultRecordError> {
        if input.len() != GUEST_FAULT_RECORD_V1_LEN {
            return Err(GuestFaultRecordError::InvalidLength(input.len()));
        }
        let magic: [u8; 4] = input[..4]
            .try_into()
            .map_err(|_| GuestFaultRecordError::InvalidLength(input.len()))?;
        let version = u16::from_le_bytes([input[4], input[5]]);
        let length = u16::from_le_bytes([input[6], input[7]]);
        if magic != GUEST_FAULT_MAGIC_V1 {
            return Err(GuestFaultRecordError::InvalidMagic);
        }
        if version != GUEST_FAULT_RECORD_VERSION_V1 {
            return Err(GuestFaultRecordError::UnsupportedVersion(version));
        }
        if usize::from(length) != GUEST_FAULT_RECORD_V1_LEN {
            return Err(GuestFaultRecordError::InvalidLength(usize::from(length)));
        }
        let mut fields = [0u32; 9];
        for (index, field) in fields.iter_mut().enumerate() {
            let start = 8 + index * 4;
            *field = u32::from_le_bytes(
                input[start..start + 4]
                    .try_into()
                    .map_err(|_| GuestFaultRecordError::InvalidLength(input.len()))?,
            );
        }
        let record = Self {
            magic,
            version,
            length,
            code: fields[0],
            stage: fields[1],
            requested_bytes: fields[2],
            alignment: fields[3],
            heap_committed_bytes: fields[4],
            heap_max_bytes: fields[5],
            live_requested_bytes: fields[6],
            high_water_requested_bytes: fields[7],
            cumulative_requested_bytes: fields[8],
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(self) -> Result<(), GuestFaultRecordError> {
        if self.magic != GUEST_FAULT_MAGIC_V1 {
            return Err(GuestFaultRecordError::InvalidMagic);
        }
        if self.version != GUEST_FAULT_RECORD_VERSION_V1 {
            return Err(GuestFaultRecordError::UnsupportedVersion(self.version));
        }
        if usize::from(self.length) != GUEST_FAULT_RECORD_V1_LEN {
            return Err(GuestFaultRecordError::InvalidLength(usize::from(
                self.length,
            )));
        }
        if self.code != 0 && GuestFaultCodeV1::from_u32(self.code).is_none() {
            return Err(GuestFaultRecordError::UnknownCode(self.code));
        }
        if GuestFaultStageV1::from_u32(self.stage).is_none() {
            return Err(GuestFaultRecordError::UnknownStage(self.stage));
        }
        if self.heap_committed_bytes > self.heap_max_bytes
            || self.heap_max_bytes > GUEST_HEAP_PLATFORM_MAX_BYTES
            || self.live_requested_bytes > self.high_water_requested_bytes
            || self.high_water_requested_bytes > self.cumulative_requested_bytes
        {
            return Err(GuestFaultRecordError::InconsistentFields);
        }
        if self.alignment != 0 && !self.alignment.is_power_of_two() {
            return Err(GuestFaultRecordError::InvalidAlignment(self.alignment));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestFaultRecordError {
    InvalidLength(usize),
    InvalidMagic,
    UnsupportedVersion(u16),
    UnknownCode(u32),
    UnknownStage(u32),
    InvalidAlignment(u32),
    InconsistentFields,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_is_valid_and_page_aligned() {
        assert_eq!(
            GuestMemoryBudgetV1::DEFAULT.validate(),
            Ok(GuestMemoryBudgetV1::DEFAULT)
        );
        assert_eq!(GUEST_MEMORY_PAGE_BYTES, 4096);
    }

    #[test]
    fn budget_rejects_invalid_ranges_and_page_rounding() {
        assert_eq!(
            GuestMemoryBudgetV1 {
                heap_initial_bytes: 4097,
                heap_max_bytes: 8192,
            }
            .validate(),
            Err(GuestMemoryBudgetError::InitialNotPageAligned(4097))
        );
        assert_eq!(
            GuestMemoryBudgetV1 {
                heap_initial_bytes: 8192,
                heap_max_bytes: 4096,
            }
            .validate(),
            Err(GuestMemoryBudgetError::InitialExceedsMaximum {
                initial: 8192,
                maximum: 4096,
            })
        );
    }

    #[test]
    fn fault_record_round_trips_and_rejects_unknown_or_stale_values() {
        let record = GuestFaultRecordV1 {
            code: GuestFaultCodeV1::HeapLimitExceeded as u32,
            stage: GuestFaultStageV1::Plan as u32,
            requested_bytes: 65_536,
            alignment: 16,
            heap_committed_bytes: 1_048_576,
            heap_max_bytes: 1_048_576,
            live_requested_bytes: 900_000,
            high_water_requested_bytes: 900_000,
            cumulative_requested_bytes: 2_000_000,
            ..GuestFaultRecordV1::EMPTY
        };
        let encoded = record.encode();
        assert_eq!(GuestFaultRecordV1::decode(&encoded), Ok(record));

        let mut invalid = encoded;
        invalid[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(
            GuestFaultRecordV1::decode(&invalid),
            Err(GuestFaultRecordError::UnknownCode(99))
        );

        assert_eq!(
            GuestFaultRecordV1::decode(&encoded[..encoded.len() - 1]),
            Err(GuestFaultRecordError::InvalidLength(encoded.len() - 1))
        );
    }
}
