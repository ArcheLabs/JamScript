#![no_std]

use blake2b_simd::Params;
use schnorrkel::{context::signing_context, PublicKey, Signature};
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
