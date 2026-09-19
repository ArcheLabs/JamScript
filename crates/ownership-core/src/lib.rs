#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use blake2b_simd::Params;
use thiserror::Error;

pub const OWNERSHIP_VERSION_V1: u8 = 1;
pub const MAX_OWNERSHIP_BYTES: usize = 4096;
pub const OWNERSHIP_KEY_DOMAIN_V1: &[u8] = b"OWNERSHIP_ABSTRACTION_KEY_V1";

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnershipKind {
    Ed25519Key = 0,
    Sr25519Key = 1,
    Secp256k1Key = 2,
    Secp256k1Keccak20 = 3,
    MulticryptoAccount32 = 4,
}

impl TryFrom<u8> for OwnershipKind {
    type Error = OwnershipError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Ed25519Key),
            1 => Ok(Self::Sr25519Key),
            2 => Ok(Self::Secp256k1Key),
            3 => Ok(Self::Secp256k1Keccak20),
            4 => Ok(Self::MulticryptoAccount32),
            _ => Err(OwnershipError::UnsupportedKind(value)),
        }
    }
}

impl OwnershipKind {
    pub const fn public_len(self) -> usize {
        match self {
            Self::Ed25519Key | Self::Sr25519Key | Self::MulticryptoAccount32 => 32,
            Self::Secp256k1Key => 33,
            Self::Secp256k1Keccak20 => 20,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ownership {
    pub version: u8,
    pub kind: OwnershipKind,
    pub public: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum OwnershipError {
    #[error("invalid Ownership version")]
    InvalidVersion,
    #[error("unsupported Ownership kind {0}")]
    UnsupportedKind(u8),
    #[error("Ownership public value has invalid length")]
    InvalidPublicLength,
    #[error("Ownership public value is not canonical")]
    InvalidPublicValue,
    #[error("Ownership encoding is invalid")]
    InvalidEncoding,
    #[error("Ownership encoding exceeds the 4096-byte bound")]
    TooLarge,
}

pub type OwnershipKey = [u8; 32];

impl Ownership {
    pub fn new(kind: OwnershipKind, public: Vec<u8>) -> Result<Self, OwnershipError> {
        let ownership = Self {
            version: OWNERSHIP_VERSION_V1,
            kind,
            public,
        };
        ownership.validate()?;
        Ok(ownership)
    }

    pub fn from_array<const N: usize>(
        kind: OwnershipKind,
        public: [u8; N],
    ) -> Result<Self, OwnershipError> {
        Self::new(kind, public.to_vec())
    }

    pub fn validate(&self) -> Result<(), OwnershipError> {
        if self.version != OWNERSHIP_VERSION_V1 {
            return Err(OwnershipError::InvalidVersion);
        }
        if self.public.len() != self.kind.public_len() {
            return Err(OwnershipError::InvalidPublicLength);
        }
        if self.kind == OwnershipKind::Secp256k1Key && !matches!(self.public.first(), Some(2 | 3)) {
            return Err(OwnershipError::InvalidPublicValue);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, OwnershipError> {
        self.validate()?;
        let length = u16::try_from(self.public.len()).map_err(|_| OwnershipError::TooLarge)?;
        let mut encoded = Vec::with_capacity(4 + self.public.len());
        encoded.push(self.version);
        encoded.push(self.kind as u8);
        encoded.extend_from_slice(&length.to_le_bytes());
        encoded.extend_from_slice(&self.public);
        if encoded.len() > MAX_OWNERSHIP_BYTES {
            return Err(OwnershipError::TooLarge);
        }
        Ok(encoded)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, OwnershipError> {
        if bytes.len() > MAX_OWNERSHIP_BYTES || bytes.len() < 4 {
            return Err(if bytes.len() > MAX_OWNERSHIP_BYTES {
                OwnershipError::TooLarge
            } else {
                OwnershipError::InvalidEncoding
            });
        }
        let version = bytes[0];
        if version != OWNERSHIP_VERSION_V1 {
            return Err(OwnershipError::InvalidVersion);
        }
        let kind = OwnershipKind::try_from(bytes[1])?;
        let length = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
        if length != kind.public_len() || bytes.len() != 4 + length {
            return Err(OwnershipError::InvalidEncoding);
        }
        Self::new(kind, bytes[4..].to_vec())
    }

    pub fn key(&self) -> Result<OwnershipKey, OwnershipError> {
        let canonical = self.encode()?;
        let mut preimage = Vec::with_capacity(OWNERSHIP_KEY_DOMAIN_V1.len() + canonical.len());
        preimage.extend_from_slice(OWNERSHIP_KEY_DOMAIN_V1);
        preimage.extend_from_slice(&canonical);
        Ok(blake2_256(&preimage))
    }
}

pub fn blake2_256(bytes: &[u8]) -> [u8; 32] {
    let digest = Params::new().hash_length(32).hash(bytes);
    let mut output = [0u8; 32];
    output.copy_from_slice(digest.as_bytes());
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn hex(value: &str) -> Vec<u8> {
        let value = value.strip_prefix("0x").unwrap_or(value);
        assert_eq!(value.len() % 2, 0);
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn ownership_key_matches_shared_v1_vectors() {
        let vectors = [
            (
                OwnershipKind::Ed25519Key,
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "01002000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "76ded8286e67ea442e884ab7d79d5ba42534f5368b866430199de51fd4cbad19",
            ),
            (
                OwnershipKind::Sr25519Key,
                "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
                "01012000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
                "8b2dba2c0a2016616f8178d4100808009d2d2e3fd962a9d67c9c48d8ccfb0e81",
            ),
            (
                OwnershipKind::Secp256k1Key,
                "02404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
                "0102210002404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
                "eab5ba0f8f4f88daed79dffe70d1a652799fb689528a377373e3ed5d76b391b4",
            ),
            (
                OwnershipKind::Secp256k1Keccak20,
                "606162636465666768696a6b6c6d6e6f70717273",
                "01031400606162636465666768696a6b6c6d6e6f70717273",
                "6c0e402a3030ea26b717d56db0bbe8d25c4b70dcaceece84a2860f9e6e1c59ec",
            ),
            (
                OwnershipKind::MulticryptoAccount32,
                "7475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90919293",
                "010420007475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90919293",
                "c86d314201f5257882179547191990558e378743decf6d2c478ad53c3a1c6522",
            ),
        ];
        for (kind, public, canonical, key) in vectors {
            let ownership = Ownership::new(kind, hex(public)).unwrap();
            assert_eq!(ownership.encode().unwrap(), hex(canonical));
            assert_eq!(ownership.key().unwrap().as_slice(), hex(key));
        }
    }

    #[test]
    fn canonical_ed25519_round_trips() {
        let value = Ownership::from_array(OwnershipKind::Ed25519Key, [7; 32]).unwrap();
        let encoded = value.encode().unwrap();
        assert_eq!(&encoded[..4], &[1, 0, 32, 0]);
        assert_eq!(Ownership::decode(&encoded).unwrap(), value);
    }

    #[test]
    fn matrix_and_generic_ed25519_are_the_same_primitive() {
        let matrix = Ownership::from_array(OwnershipKind::Ed25519Key, [9; 32]).unwrap();
        let generic = Ownership::new(OwnershipKind::Ed25519Key, vec![9; 32]).unwrap();
        assert_eq!(matrix, generic);
        assert_eq!(matrix.key().unwrap(), generic.key().unwrap());
    }

    #[test]
    fn rejects_wrong_lengths_and_noncanonical_compressed_keys() {
        assert_eq!(
            Ownership::new(OwnershipKind::Ed25519Key, vec![0; 31]),
            Err(OwnershipError::InvalidPublicLength)
        );
        assert_eq!(
            Ownership::new(OwnershipKind::Secp256k1Key, vec![4; 33]),
            Err(OwnershipError::InvalidPublicValue)
        );
        assert_eq!(
            Ownership::decode(&[1, 0, 32, 0, 0]),
            Err(OwnershipError::InvalidEncoding)
        );
    }

    #[test]
    fn key_is_not_the_canonical_ownership_encoding() {
        let ownership = Ownership::from_array(OwnershipKind::Sr25519Key, [3; 32]).unwrap();
        assert_ne!(
            ownership.key().unwrap().as_slice(),
            ownership.encode().unwrap().as_slice()
        );
    }
}
