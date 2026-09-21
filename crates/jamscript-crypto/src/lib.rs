#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use alloc::{format, string::String};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
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
pub const MAX_MATRIX_TEXT_BYTES: usize = 255;
pub const MAX_MATRIX_ALGORITHMS: usize = 8;
pub const MAX_MATRIX_ALGORITHM_BYTES: usize = 128;
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
    #[error("invalid Matrix bootstrap proof")]
    InvalidMatrixProof,
    #[error("Matrix bootstrap proof verification failed")]
    MatrixProofVerificationFailed,
    #[error("invalid Polkadot authorization proof")]
    InvalidPolkadotAuthorization,
    #[error("Polkadot AccountId32 does not match the recovered public key")]
    PolkadotAddressMismatch,
    #[error("EVM signature uses a non-canonical high-s value")]
    EvmHighS,
    #[error("EVM address does not match the recovered public key")]
    EvmAddressMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixControlClaimProofV1 {
    pub user_id: String,
    pub self_signing_public_key: [u8; 32],
    pub master_signature: [u8; 64],
    pub device_id: String,
    pub algorithms: Vec<String>,
    pub device_curve25519_key: [u8; 32],
    pub device_ed25519_key: [u8; 32],
    pub self_signing_signature: [u8; 64],
}

impl MatrixControlClaimProofV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, CryptoError> {
        let mut reader = MatrixReader { bytes, offset: 0 };
        if reader.u8()? != 1 {
            return Err(CryptoError::InvalidMatrixProof);
        }
        let user_id = reader.text()?;
        let self_signing_public_key = reader.array::<32>()?;
        let master_signature = reader.array::<64>()?;
        let device_id = reader.text()?;
        let count = reader.u8()? as usize;
        if count > MAX_MATRIX_ALGORITHMS {
            return Err(CryptoError::InvalidMatrixProof);
        }
        let mut algorithms = Vec::with_capacity(count);
        for _ in 0..count {
            algorithms.push(reader.text_bounded(MAX_MATRIX_ALGORITHM_BYTES)?);
        }
        let device_curve25519_key = reader.array::<32>()?;
        let device_ed25519_key = reader.array::<32>()?;
        let self_signing_signature = reader.array::<64>()?;
        if reader.offset != bytes.len() {
            return Err(CryptoError::InvalidMatrixProof);
        }
        Ok(Self {
            user_id,
            self_signing_public_key,
            master_signature,
            device_id,
            algorithms,
            device_curve25519_key,
            device_ed25519_key,
            self_signing_signature,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, CryptoError> {
        if self.user_id.len() > MAX_MATRIX_TEXT_BYTES
            || self.device_id.len() > MAX_MATRIX_TEXT_BYTES
            || self.algorithms.len() > MAX_MATRIX_ALGORITHMS
            || self
                .algorithms
                .iter()
                .any(|value| value.len() > MAX_MATRIX_ALGORITHM_BYTES)
            || !self.user_id.is_ascii()
            || !self.device_id.is_ascii()
            || self.algorithms.iter().any(|value| !value.is_ascii())
        {
            return Err(CryptoError::InvalidMatrixProof);
        }
        let mut output = Vec::new();
        output.push(1);
        put_text(&mut output, &self.user_id)?;
        output.extend_from_slice(&self.self_signing_public_key);
        output.extend_from_slice(&self.master_signature);
        put_text(&mut output, &self.device_id)?;
        output.push(self.algorithms.len() as u8);
        for algorithm in &self.algorithms {
            put_text_bounded(&mut output, algorithm, MAX_MATRIX_ALGORITHM_BYTES)?;
        }
        output.extend_from_slice(&self.device_curve25519_key);
        output.extend_from_slice(&self.device_ed25519_key);
        output.extend_from_slice(&self.self_signing_signature);
        Ok(output)
    }

    pub fn verify(&self, master_public_key: &[u8; 32]) -> Result<(), CryptoError> {
        let self_object = self.canonical_self_signing_object()?;
        verify_ed25519(master_public_key, &self.master_signature, &self_object)
            .map_err(|_| CryptoError::MatrixProofVerificationFailed)?;
        let device_object = self.canonical_device_keys_object()?;
        verify_ed25519(
            &self.self_signing_public_key,
            &self.self_signing_signature,
            &device_object,
        )
        .map_err(|_| CryptoError::MatrixProofVerificationFailed)?;
        Ok(())
    }

    pub fn verify_for(
        &self,
        master_public_key: &[u8; 32],
        expected_device_public_key: &[u8; 32],
    ) -> Result<(), CryptoError> {
        self.verify(master_public_key)?;
        if &self.device_ed25519_key != expected_device_public_key {
            return Err(CryptoError::MatrixProofVerificationFailed);
        }
        Ok(())
    }

    pub fn canonical_self_signing_object(&self) -> Result<Vec<u8>, CryptoError> {
        let key = STANDARD_NO_PAD.encode(self.self_signing_public_key);
        Ok(format_matrix_self_object(&self.user_id, &key))
    }

    pub fn canonical_device_keys_object(&self) -> Result<Vec<u8>, CryptoError> {
        let curve = STANDARD_NO_PAD.encode(self.device_curve25519_key);
        let ed = STANDARD_NO_PAD.encode(self.device_ed25519_key);
        if !self.user_id.is_ascii()
            || !self.device_id.is_ascii()
            || self.algorithms.iter().any(|value| !value.is_ascii())
        {
            return Err(CryptoError::InvalidMatrixProof);
        }
        Ok(format_matrix_device_object(
            &self.user_id,
            &self.device_id,
            &self.algorithms,
            &curve,
            &ed,
        ))
    }
}

/// Verify the deterministic Matrix M→S→D cross-signing evidence for a
/// subject master key and controller device key. This primitive is pure:
/// it does not read state, access a network, or perform host calls.
pub fn verify_matrix_cross_signing(
    subject_public_key: &[u8],
    controller_public_key: &[u8],
    proof: &[u8],
) -> bool {
    let Ok(master) = <[u8; 32]>::try_from(subject_public_key) else {
        return false;
    };
    let Ok(device) = <[u8; 32]>::try_from(controller_public_key) else {
        return false;
    };
    let Ok(decoded) = MatrixControlClaimProofV1::decode(proof) else {
        return false;
    };
    decoded.verify_for(&master, &device).is_ok()
}

fn put_text(output: &mut Vec<u8>, value: &str) -> Result<(), CryptoError> {
    put_text_bounded(output, value, MAX_MATRIX_TEXT_BYTES)
}

fn put_text_bounded(output: &mut Vec<u8>, value: &str, max: usize) -> Result<(), CryptoError> {
    if value.len() > max
        || value.len() > u16::MAX as usize
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte < 0x20 || byte == b'"' || byte == b'\\')
    {
        return Err(CryptoError::InvalidMatrixProof);
    }
    output.extend_from_slice(&(value.len() as u16).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn format_matrix_self_object(user_id: &str, key: &str) -> Vec<u8> {
    format!(
        r#"{{"keys":{{"ed25519:{key}":"{key}"}},"usage":["self_signing"],"user_id":"{user_id}"}}"#
    )
    .into_bytes()
}

fn format_matrix_device_object(
    user_id: &str,
    device_id: &str,
    algorithms: &[String],
    curve: &str,
    ed: &str,
) -> Vec<u8> {
    let algorithms = algorithms
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"algorithms":[{algorithms}],"device_id":"{device_id}","keys":{{"curve25519:{device_id}":"{curve}","ed25519:{device_id}":"{ed}"}},"user_id":"{user_id}"}}"#).into_bytes()
}

struct MatrixReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> MatrixReader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], CryptoError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(CryptoError::InvalidMatrixProof)?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or(CryptoError::InvalidMatrixProof)?;
        self.offset = end;
        Ok(result)
    }
    fn u8(&mut self) -> Result<u8, CryptoError> {
        Ok(self.take(1)?[0])
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CryptoError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CryptoError::InvalidMatrixProof)
    }
    fn text(&mut self) -> Result<String, CryptoError> {
        self.text_bounded(MAX_MATRIX_TEXT_BYTES)
    }
    fn text_bounded(&mut self, max: usize) -> Result<String, CryptoError> {
        let length = u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| CryptoError::InvalidMatrixProof)?,
        ) as usize;
        if length > max {
            return Err(CryptoError::InvalidMatrixProof);
        }
        let bytes = self.take(length)?;
        if !bytes.is_ascii() {
            return Err(CryptoError::InvalidMatrixProof);
        }
        if bytes
            .iter()
            .any(|byte| *byte < 0x20 || *byte == b'"' || *byte == b'\\')
        {
            return Err(CryptoError::InvalidMatrixProof);
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| CryptoError::InvalidMatrixProof)
    }
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
    use alloc::vec;
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
    fn matrix_bootstrap_proof_is_bounded_and_verifies_both_signatures() {
        let master = SigningKey::from_bytes(&[1; 32]);
        let self_signing = SigningKey::from_bytes(&[2; 32]);
        let device = SigningKey::from_bytes(&[3; 32]);
        let mut proof = MatrixControlClaimProofV1 {
            user_id: "@alice:example.org".into(),
            self_signing_public_key: self_signing.verifying_key().to_bytes(),
            master_signature: [0; 64],
            device_id: "DEVICE".into(),
            algorithms: vec!["m.olm.v1.curve25519-aes-sha2".into()],
            device_curve25519_key: [4; 32],
            device_ed25519_key: device.verifying_key().to_bytes(),
            self_signing_signature: [0; 64],
        };
        proof.master_signature = master
            .sign(&proof.canonical_self_signing_object().unwrap())
            .to_bytes();
        proof.self_signing_signature = self_signing
            .sign(&proof.canonical_device_keys_object().unwrap())
            .to_bytes();
        let encoded = proof.encode().unwrap();
        let encoded_hex = encoded
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            encoded_hex,
            "01120040616c6963653a6578616d706c652e6f72678139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b3944b46de57790ce84e2c308ff13e3078bdb8c20b7026b0ad22ab754f7a798e3292c87e72d01d1be3dad936ce1e1ec9ca3f4f6298a3a79dffdfbed94814a31c26070600444556494345011c006d2e6f6c6d2e76312e637572766532353531392d6165732d736861320404040404040404040404040404040404040404040404040404040404040404ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1024da97d568567c89a2b3fe71fe720122c3bbfff2873246d9ba6a3ff8a76fb97e6d82534a03eccaede5d81189bed267eb458b009f4a8c7c155c296832919450d"
        );
        let decoded = MatrixControlClaimProofV1::decode(&encoded).unwrap();
        decoded.verify(&master.verifying_key().to_bytes()).unwrap();
        let encoded = decoded.encode().unwrap();
        assert!(verify_matrix_cross_signing(
            &master.verifying_key().to_bytes(),
            &device.verifying_key().to_bytes(),
            &encoded,
        ));
        assert!(!verify_matrix_cross_signing(
            &master.verifying_key().to_bytes(),
            &[9; 32],
            &encoded,
        ));
        let mut tampered = decoded.clone();
        tampered.device_id = "OTHER".into();
        assert_eq!(
            tampered.verify(&master.verifying_key().to_bytes()),
            Err(CryptoError::MatrixProofVerificationFailed)
        );
    }
}
