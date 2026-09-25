#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use blake2b_simd::Params;
use ed25519_dalek::{Signature as Ed25519Signature, VerifyingKey as Ed25519VerifyingKey};
use k256::ecdsa::{RecoveryId, Signature as EcdsaSignature, VerifyingKey as EcdsaVerifyingKey};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use ownership_core::{Ownership, OwnershipKind};
use schnorrkel::{context::signing_context, PublicKey, Signature};
use sha3::{Digest, Keccak256};
use thiserror::Error;

/// The standard Substrate sr25519 signing context used by `signRaw`.
///
/// JamScript's protocol domain remains part of the digest constructed by
/// `SignedActionV1`; it must not be used as the sr25519 transcript context.
pub const SR25519_CONTEXT: &[u8] = b"substrate";
pub type Address = [u8; 32];

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CryptoError {
    #[error("invalid sr25519 public key")]
    InvalidPublicKey,
    #[error("invalid sr25519 signature")]
    InvalidSignature,
    #[error("sr25519 signature verification failed")]
    VerificationFailed,
    #[error("invalid ed25519 public key")]
    InvalidEd25519PublicKey,
    #[error("invalid ed25519 signature")]
    InvalidEd25519Signature,
    #[error("ed25519 signature verification failed")]
    Ed25519VerificationFailed,
    #[error("invalid secp256k1 signature")]
    InvalidEcdsaSignature,
    #[error("secp256k1 signature recovery failed")]
    EcdsaRecoveryFailed,
    #[error("ownership kind has no supported authorization adapter")]
    UnsupportedOwnershipKind,
    #[error("invalid Polkadot authorization proof")]
    InvalidPolkadotAuthorization,
    #[error("Polkadot AccountId32 does not match the recovered public key")]
    PolkadotAddressMismatch,
    #[error("EVM signature uses a non-canonical high-s value")]
    EvmHighS,
    #[error("EVM address does not match the recovered public key")]
    EvmAddressMismatch,
}

pub fn blake2_256(bytes: &[u8]) -> [u8; 32] {
    let digest = Params::new().hash_length(32).hash(bytes);
    let mut output = [0u8; 32];
    output.copy_from_slice(digest.as_bytes());
    output
}

pub fn derive_address(public_key: &[u8]) -> Result<Address, CryptoError> {
    let public = PublicKey::from_bytes(public_key).map_err(|_| CryptoError::InvalidPublicKey)?;
    Ok(public.to_bytes())
}

pub fn verify_sr25519(
    public_key: &[u8],
    signature: &[u8],
    message: &[u8],
) -> Result<Address, CryptoError> {
    let public = PublicKey::from_bytes(public_key).map_err(|_| CryptoError::InvalidPublicKey)?;
    let signature = Signature::from_bytes(signature).map_err(|_| CryptoError::InvalidSignature)?;
    let transcript = signing_context(SR25519_CONTEXT).bytes(message);
    public
        .verify(transcript, &signature)
        .map_err(|_| CryptoError::VerificationFailed)?;
    Ok(public.to_bytes())
}

pub fn verify_ed25519(
    public_key: &[u8],
    signature: &[u8],
    message: &[u8],
) -> Result<[u8; 32], CryptoError> {
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| CryptoError::InvalidEd25519PublicKey)?;
    let key = Ed25519VerifyingKey::from_bytes(&public_key)
        .map_err(|_| CryptoError::InvalidEd25519PublicKey)?;
    let signature = Ed25519Signature::from_slice(signature)
        .map_err(|_| CryptoError::InvalidEd25519Signature)?;
    ed25519_dalek::Verifier::verify(&key, message, &signature)
        .map_err(|_| CryptoError::Ed25519VerificationFailed)?;
    Ok(public_key)
}

pub fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut output = [0u8; 32];
    output.copy_from_slice(&Keccak256::digest(bytes));
    output
}

/// Recover a secp256k1 public key from a 65-byte Ethereum-style signature.
/// The first 64 bytes are the compact ECDSA signature and the last byte is
/// the recovery id (0/1, or 27/28 for legacy wire representations).
pub fn recover_secp256k1(digest: &[u8; 32], signature: &[u8]) -> Result<[u8; 33], CryptoError> {
    if signature.len() != 65 {
        return Err(CryptoError::InvalidEcdsaSignature);
    }
    let ecdsa = EcdsaSignature::try_from(&signature[..64])
        .map_err(|_| CryptoError::InvalidEcdsaSignature)?;
    if ecdsa.normalize_s().is_some() {
        return Err(CryptoError::EvmHighS);
    }
    let recovery = match signature[64] {
        0 | 1 => RecoveryId::try_from(signature[64]),
        27 | 28 => RecoveryId::try_from(signature[64] - 27),
        _ => return Err(CryptoError::InvalidEcdsaSignature),
    }
    .map_err(|_| CryptoError::InvalidEcdsaSignature)?;
    let key = EcdsaVerifyingKey::recover_from_prehash(digest, &ecdsa, recovery)
        .map_err(|_| CryptoError::EcdsaRecoveryFailed)?;
    let encoded = key.to_encoded_point(true);
    encoded
        .as_bytes()
        .try_into()
        .map_err(|_| CryptoError::EcdsaRecoveryFailed)
}

/// Build the EIP-712 digest used by the JamScript EVM binding.  The binding
/// intentionally has no chainId or verifyingContract: network_domain is the
/// only network-scoped value supplied by JamScript.
pub fn evm_action_digest(network_domain: &[u8; 32], commitment: &[u8; 32]) -> [u8; 32] {
    let domain_type = keccak256(b"EIP712Domain(string name,string version,bytes32 salt)");
    let name = keccak256(b"JamScript");
    let version = keccak256(b"1");
    let mut domain_preimage = Vec::with_capacity(32 * 4);
    domain_preimage.extend_from_slice(&domain_type);
    domain_preimage.extend_from_slice(&name);
    domain_preimage.extend_from_slice(&version);
    domain_preimage.extend_from_slice(network_domain);
    let domain_separator = keccak256(&domain_preimage);

    let action_type = keccak256(b"JamScriptAction(bytes32 commitment)");
    let mut action_preimage = Vec::with_capacity(64);
    action_preimage.extend_from_slice(&action_type);
    action_preimage.extend_from_slice(commitment);
    let struct_hash = keccak256(&action_preimage);

    let mut digest_preimage = Vec::with_capacity(66);
    digest_preimage.extend_from_slice(b"\x19\x01");
    digest_preimage.extend_from_slice(&domain_separator);
    digest_preimage.extend_from_slice(&struct_hash);
    keccak256(&digest_preimage)
}

pub fn verify_evm_ownership(
    ownership: &Ownership,
    signature: &[u8],
    network_domain: &[u8; 32],
    commitment: &[u8; 32],
) -> Result<(), CryptoError> {
    if ownership.kind != OwnershipKind::Secp256k1Keccak20 {
        return Err(CryptoError::UnsupportedOwnershipKind);
    }
    let digest = evm_action_digest(network_domain, commitment);
    let recovered = recover_secp256k1(&digest, signature)?;
    let key = k256::PublicKey::from_sec1_bytes(&recovered)
        .map_err(|_| CryptoError::EcdsaRecoveryFailed)?;
    let uncompressed = key.to_encoded_point(false);
    let hash = keccak256(&uncompressed.as_bytes()[1..]);
    if hash[12..] != ownership.public[..] {
        return Err(CryptoError::EvmAddressMismatch);
    }
    Ok(())
}

pub fn verify_polkadot_ownership(
    ownership: &Ownership,
    authorization_proof: &[u8],
    message: &[u8],
) -> Result<(), CryptoError> {
    if ownership.kind != OwnershipKind::MulticryptoAccount32
        || ownership.public.len() != 32
        || authorization_proof.is_empty()
    {
        return Err(CryptoError::InvalidPolkadotAuthorization);
    }
    let scheme = authorization_proof[0];
    let signature = &authorization_proof[1..];
    match scheme {
        0 => {
            if signature.len() != 64 {
                return Err(CryptoError::InvalidPolkadotAuthorization);
            }
            verify_ed25519(&ownership.public, signature, message)
                .map_err(|_| CryptoError::InvalidPolkadotAuthorization)?;
        }
        1 => {
            if signature.len() != 64 {
                return Err(CryptoError::InvalidPolkadotAuthorization);
            }
            verify_sr25519(&ownership.public, signature, message)
                .map_err(|_| CryptoError::InvalidPolkadotAuthorization)?;
        }
        2 => {
            if signature.len() != 65 {
                return Err(CryptoError::InvalidPolkadotAuthorization);
            }
            let digest = blake2_256(message);
            let recovered = recover_secp256k1(&digest, signature)?;
            if blake2_256(&recovered) != ownership.public.as_slice() {
                return Err(CryptoError::PolkadotAddressMismatch);
            }
        }
        _ => return Err(CryptoError::InvalidPolkadotAuthorization),
    }
    Ok(())
}

pub fn verify_ownership(
    ownership: &Ownership,
    signature: &[u8],
    message: &[u8],
) -> Result<(), CryptoError> {
    ownership
        .validate()
        .map_err(|_| CryptoError::UnsupportedOwnershipKind)?;
    match ownership.kind {
        OwnershipKind::Ed25519Key => {
            verify_ed25519(&ownership.public, signature, message).map(|_| ())
        }
        OwnershipKind::Sr25519Key => {
            verify_sr25519(&ownership.public, signature, message).map(|_| ())
        }
        OwnershipKind::Secp256k1Key => {
            let digest = blake2_256(message);
            let recovered = recover_secp256k1(&digest, signature)?;
            if recovered == ownership.public.as_slice() {
                Ok(())
            } else {
                Err(CryptoError::EcdsaRecoveryFailed)
            }
        }
        OwnershipKind::Secp256k1Keccak20 => {
            let digest = blake2_256(message);
            let recovered = recover_secp256k1(&digest, signature)?;
            let key = k256::PublicKey::from_sec1_bytes(&recovered)
                .map_err(|_| CryptoError::EcdsaRecoveryFailed)?;
            let uncompressed = key.to_encoded_point(false);
            let hash = keccak256(&uncompressed.as_bytes()[1..]);
            if hash[12..] == ownership.public[..] {
                Ok(())
            } else {
                Err(CryptoError::EcdsaRecoveryFailed)
            }
        }
        OwnershipKind::MulticryptoAccount32 => {
            verify_polkadot_ownership(ownership, signature, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use schnorrkel::{ExpansionMode, MiniSecretKey};

    #[test]
    fn verifies_and_derives_account_id32() {
        let keypair = MiniSecretKey::from_bytes(&[7; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let message = b"deterministic message";
        let signature = keypair.sign(signing_context(SR25519_CONTEXT).bytes(message));
        assert_eq!(
            verify_sr25519(&keypair.public.to_bytes(), &signature.to_bytes(), message).unwrap(),
            keypair.public.to_bytes()
        );
    }

    #[test]
    fn rejects_modified_message() {
        let keypair = MiniSecretKey::from_bytes(&[8; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let signature = keypair.sign(signing_context(SR25519_CONTEXT).bytes(b"a"));
        assert_eq!(
            verify_sr25519(&keypair.public.to_bytes(), &signature.to_bytes(), b"b"),
            Err(CryptoError::VerificationFailed)
        );
    }

    #[test]
    fn generic_ed25519_verification_handles_valid_invalid_and_malformed_inputs() {
        let key = SigningKey::from_bytes(&[19; 32]);
        let other_key = SigningKey::from_bytes(&[20; 32]);
        let message = b"provider-neutral deterministic message";
        let signature = key.sign(message).to_bytes();

        assert_eq!(
            verify_ed25519(&key.verifying_key().to_bytes(), &signature, message),
            Ok(key.verifying_key().to_bytes())
        );
        assert_eq!(
            verify_ed25519(&other_key.verifying_key().to_bytes(), &signature, message),
            Err(CryptoError::Ed25519VerificationFailed)
        );
        assert_eq!(
            verify_ed25519(
                &key.verifying_key().to_bytes(),
                &signature,
                b"wrong message"
            ),
            Err(CryptoError::Ed25519VerificationFailed)
        );
        assert_eq!(
            verify_ed25519(&[1; 31], &signature, message),
            Err(CryptoError::InvalidEd25519PublicKey)
        );
        assert_eq!(
            verify_ed25519(&key.verifying_key().to_bytes(), &[0; 63], message),
            Err(CryptoError::InvalidEd25519Signature)
        );
    }
}
