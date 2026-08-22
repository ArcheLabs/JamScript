#![no_std]

extern crate alloc;

use jamscript_crypto::{blake2_256, verify_sr25519, Address};
use service_runtime_core::{application_key_v1, wallet_nonce_key_v1};

pub const RUNTIME_VERSION: &str = "0.1.0";
pub const MAX_ACTION_BYTES: usize = 1_048_576;
pub const MAX_RESULT_BYTES: usize = 1_048_576;
/// Domain separator retained inside the SignedActionV1 digest.
pub const ACTION_DOMAIN_V1: &[u8] = b"JAMSCRIPT_ACTION_V1";
pub const STATE_KEY_DOMAIN_V1: &[u8] = b"jamscript/state/v1";
pub const NONCE_SCHEMA_V1: &[u8] = b"__jamscript/runtime/auth/nonces/";

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    InvalidEnvelope = 1,
    UnsupportedVersion = 2,
    WrongNetwork = 3,
    WrongService = 4,
    UnknownAction = 5,
    PayloadTooLarge = 6,
    PayloadHashMismatch = 7,
    InvalidSignature = 8,
    Expired = 9,
    NonceMismatch = 10,
    UnsupportedSigner = 11,
    OutputTooLarge = 14,
}

impl RuntimeError {
    pub const fn code(self) -> u32 {
        self as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedActionView<'a> {
    pub version: u8,
    pub genesis_hash: [u8; 32],
    pub service_id: u32,
    pub action_selector: [u8; 8],
    pub signer_scheme: u8,
    pub public_key: &'a [u8],
    pub nonce: u64,
    pub valid_until: u64,
    pub payload_hash: [u8; 32],
    pub signature: &'a [u8],
    pub payload: &'a [u8],
    pub encoded: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedAction<'a> {
    pub sender: Address,
    pub action_hash: [u8; 32],
    pub action_selector: [u8; 8],
    pub nonce: u64,
    pub valid_until: u64,
    pub payload: &'a [u8],
}

#[repr(C)]
pub struct RefineOutput {
    pub data: *const u8,
    pub size: usize,
}

pub fn checked_add_u64(left: u64, right: u64) -> Option<u64> {
    left.checked_add(right)
}

pub fn decode_signed_action(bytes: &[u8]) -> Result<SignedActionView<'_>, RuntimeError> {
    if bytes.len() > MAX_ACTION_BYTES {
        return Err(RuntimeError::PayloadTooLarge);
    }
    let mut reader = Reader { bytes, offset: 0 };
    let version = reader.u8()?;
    let genesis_hash = reader.array::<32>()?;
    let service_id = reader.u32()?;
    let action_selector = reader.array::<8>()?;
    let signer_scheme = reader.u8()?;
    let public_key = reader.bytes_u8()?;
    let nonce = reader.u64()?;
    let valid_until = reader.u64()?;
    let payload_hash = reader.array::<32>()?;
    let signature = reader.bytes_u8()?;
    let payload = reader.bytes_u32()?;
    if reader.offset != bytes.len() {
        return Err(RuntimeError::InvalidEnvelope);
    }
    if public_key.len() > 32 || signature.len() > 64 {
        return Err(RuntimeError::InvalidEnvelope);
    }
    Ok(SignedActionView {
        version,
        genesis_hash,
        service_id,
        action_selector,
        signer_scheme,
        public_key,
        nonce,
        valid_until,
        payload_hash,
        signature,
        payload,
        encoded: bytes,
    })
}

pub fn verify_signed_action<'a>(
    action: SignedActionView<'a>,
    expected_genesis_hash: [u8; 32],
    expected_service_id: u32,
    expected_action_selector: [u8; 8],
) -> Result<VerifiedAction<'a>, RuntimeError> {
    if action.version != 1 {
        return Err(RuntimeError::UnsupportedVersion);
    }
    if action.genesis_hash != expected_genesis_hash {
        return Err(RuntimeError::WrongNetwork);
    }
    if action.service_id != expected_service_id {
        return Err(RuntimeError::WrongService);
    }
    if action.action_selector != expected_action_selector {
        return Err(RuntimeError::UnknownAction);
    }
    if action.signer_scheme != 0 {
        return Err(RuntimeError::UnsupportedSigner);
    }
    if action.public_key.len() != 32 || action.signature.len() != 64 {
        return Err(RuntimeError::InvalidSignature);
    }
    if blake2_256(action.payload) != action.payload_hash {
        return Err(RuntimeError::PayloadHashMismatch);
    }
    let digest = signing_digest(&action);
    let sender = verify_sr25519(action.public_key, action.signature, &digest)
        .map_err(|_| RuntimeError::InvalidSignature)?;
    Ok(VerifiedAction {
        sender,
        action_hash: blake2_256(action.encoded),
        action_selector: action.action_selector,
        nonce: action.nonce,
        valid_until: action.valid_until,
        payload: action.payload,
    })
}

pub fn check_expiry(valid_until: u64, authoritative_tick: u64) -> Result<(), RuntimeError> {
    if authoritative_tick > valid_until {
        Err(RuntimeError::Expired)
    } else {
        Ok(())
    }
}

pub fn signing_digest(action: &SignedActionView<'_>) -> [u8; 32] {
    let mut preimage = [0u8; ACTION_DOMAIN_V1.len() + 1 + 32 + 4 + 8 + 1 + 8 + 8 + 32];
    let mut offset = 0;
    preimage[offset..offset + ACTION_DOMAIN_V1.len()].copy_from_slice(ACTION_DOMAIN_V1);
    offset += ACTION_DOMAIN_V1.len();
    preimage[offset] = action.version;
    offset += 1;
    preimage[offset..offset + 32].copy_from_slice(&action.genesis_hash);
    offset += 32;
    preimage[offset..offset + 4].copy_from_slice(&action.service_id.to_le_bytes());
    offset += 4;
    preimage[offset..offset + 8].copy_from_slice(&action.action_selector);
    offset += 8;
    preimage[offset] = action.signer_scheme;
    offset += 1;
    preimage[offset..offset + 8].copy_from_slice(&action.nonce.to_le_bytes());
    offset += 8;
    preimage[offset..offset + 8].copy_from_slice(&action.valid_until.to_le_bytes());
    offset += 8;
    preimage[offset..offset + 32].copy_from_slice(&action.payload_hash);
    blake2_256(&preimage)
}

pub fn encode_refined_action(
    action: &VerifiedAction<'_>,
    result: &[u8],
    output: &mut [u8],
) -> Result<usize, RuntimeError> {
    if result.len() > MAX_RESULT_BYTES {
        return Err(RuntimeError::OutputTooLarge);
    }
    let required = 1 + 8 + 32 + 8 + 8 + 32 + 4 + result.len();
    if required > output.len() {
        return Err(RuntimeError::OutputTooLarge);
    }
    let mut offset = 0;
    output[offset] = 1;
    offset += 1;
    output[offset..offset + 8].copy_from_slice(&action.action_selector);
    offset += 8;
    output[offset..offset + 32].copy_from_slice(&action.sender);
    offset += 32;
    output[offset..offset + 8].copy_from_slice(&action.nonce.to_le_bytes());
    offset += 8;
    output[offset..offset + 8].copy_from_slice(&action.valid_until.to_le_bytes());
    offset += 8;
    output[offset..offset + 32].copy_from_slice(&action.action_hash);
    offset += 32;
    output[offset..offset + 4].copy_from_slice(&(result.len() as u32).to_le_bytes());
    offset += 4;
    output[offset..offset + result.len()].copy_from_slice(result);
    Ok(required)
}

pub fn decode_refined_action(bytes: &[u8]) -> Result<RefinedActionView<'_>, RuntimeError> {
    let mut reader = Reader { bytes, offset: 0 };
    let version = reader.u8()?;
    if version != 1 {
        return Err(RuntimeError::UnsupportedVersion);
    }
    let action_selector = reader.array::<8>()?;
    let sender = reader.array::<32>()?;
    let nonce = reader.u64()?;
    let valid_until = reader.u64()?;
    let action_hash = reader.array::<32>()?;
    let result = reader.bytes_u32()?;
    if reader.offset != bytes.len() {
        return Err(RuntimeError::InvalidEnvelope);
    }
    Ok(RefinedActionView {
        action_selector,
        sender,
        nonce,
        valid_until,
        action_hash,
        result,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinedActionView<'a> {
    pub action_selector: [u8; 8],
    pub sender: Address,
    pub nonce: u64,
    pub valid_until: u64,
    pub action_hash: [u8; 32],
    pub result: &'a [u8],
}

pub fn state_key(_service_id: u32, schema: &[u8], key: &[u8]) -> alloc::vec::Vec<u8> {
    application_key_v1(schema, key).expect("managed application key length must fit u16")
}

pub fn nonce_key(account: &Address) -> alloc::vec::Vec<u8> {
    wallet_nonce_key_v1(account)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], RuntimeError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(RuntimeError::InvalidEnvelope)?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or(RuntimeError::InvalidEnvelope)?;
        self.offset = end;
        Ok(result)
    }

    fn u8(&mut self) -> Result<u8, RuntimeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, RuntimeError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| RuntimeError::InvalidEnvelope)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, RuntimeError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| RuntimeError::InvalidEnvelope)?,
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], RuntimeError> {
        self.take(N)?
            .try_into()
            .map_err(|_| RuntimeError::InvalidEnvelope)
    }

    fn bytes_u8(&mut self) -> Result<&'a [u8], RuntimeError> {
        let length = self.u8()? as usize;
        self.take(length)
    }

    fn bytes_u32(&mut self) -> Result<&'a [u8], RuntimeError> {
        let length = self.u32()? as usize;
        if length > MAX_ACTION_BYTES {
            return Err(RuntimeError::PayloadTooLarge);
        }
        self.take(length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_key_is_length_delimited() {
        assert_ne!(state_key(1, b"a", b"bc"), state_key(1, b"ab", b"c"));
        assert_eq!(
            state_key(1, b"scores", b"alice"),
            state_key(2, b"scores", b"alice")
        );
        assert_eq!(nonce_key(&[7; 32])[..2], [0, 1]);
    }

    #[test]
    fn checked_add_remains_available() {
        assert_eq!(checked_add_u64(1, 2), Some(3));
    }

    #[test]
    fn verifies_the_reference_protocol_envelope() {
        use jamscript_crypto::SR25519_CONTEXT;
        use jamscript_protocol::SignedActionV1;
        use schnorrkel::{context::signing_context, ExpansionMode, MiniSecretKey};

        let keypair = MiniSecretKey::from_bytes(&[7; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let mut action = SignedActionV1::unsigned(
            [1; 32],
            182,
            [3; 8],
            keypair.public.to_bytes(),
            4,
            20,
            7u64.to_le_bytes().to_vec(),
        )
        .unwrap();
        let signature =
            keypair.sign(signing_context(SR25519_CONTEXT).bytes(&action.signing_digest()));
        action.signature = signature.to_bytes().to_vec();
        let encoded = action.encode().unwrap();
        let verified = verify_signed_action(
            decode_signed_action(&encoded).unwrap(),
            [1; 32],
            182,
            [3; 8],
        )
        .unwrap();
        check_expiry(verified.valid_until, 10).unwrap();
        assert_eq!(verified.nonce, 4);
        assert_eq!(verified.payload, 7u64.to_le_bytes());
    }
}
