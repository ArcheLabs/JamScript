use jamscript_crypto::{blake2_256, verify_sr25519, Address, CryptoError};
use service_runtime_core::ServiceKeyV1;
use thiserror::Error;

pub const SIGNED_ACTION_VERSION_V1: u8 = 1;
pub const SIGNED_ACTION_VERSION_V2: u8 = 2;
pub const SIGNING_DOMAIN_V1: &[u8] = b"JAMSCRIPT_ACTION_V1";
pub const SIGNING_DOMAIN_V2: &[u8] = b"JAMSCRIPT_ACTION_V2";
pub const MAX_PAYLOAD_BYTES: usize = 1_048_576;
pub const MAX_PUBLIC_KEY_BYTES: usize = 32;
pub const MAX_SIGNATURE_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SignerScheme {
    Sr25519 = 0,
    Ed25519 = 1,
    Ecdsa = 2,
}

impl TryFrom<u8> for SignerScheme {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Sr25519),
            1 => Ok(Self::Ed25519),
            2 => Ok(Self::Ecdsa),
            _ => Err(ProtocolError::InvalidEnvelope("unknown signer scheme")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedActionV1 {
    pub version: u8,
    pub genesis_hash: [u8; 32],
    pub service_id: u32,
    pub action_selector: [u8; 8],
    pub signer_scheme: SignerScheme,
    pub public_key: Vec<u8>,
    pub nonce: u64,
    pub valid_until: u64,
    pub payload_hash: [u8; 32],
    pub signature: Vec<u8>,
    pub payload: Vec<u8>,
}

impl SignedActionV1 {
    pub fn unsigned(
        genesis_hash: [u8; 32],
        service_id: u32,
        action_selector: [u8; 8],
        public_key: [u8; 32],
        nonce: u64,
        valid_until: u64,
        payload: Vec<u8>,
    ) -> Result<Self, ProtocolError> {
        ensure_payload(&payload)?;
        Ok(Self {
            version: SIGNED_ACTION_VERSION_V1,
            genesis_hash,
            service_id,
            action_selector,
            signer_scheme: SignerScheme::Sr25519,
            public_key: public_key.to_vec(),
            nonce,
            valid_until,
            payload_hash: blake2_256(&payload),
            signature: Vec::new(),
            payload,
        })
    }

    pub fn signing_digest(&self) -> [u8; 32] {
        let mut preimage =
            Vec::with_capacity(1 + 32 + 4 + 8 + 1 + 8 + 8 + 32 + SIGNING_DOMAIN_V1.len());
        preimage.extend_from_slice(SIGNING_DOMAIN_V1);
        preimage.push(self.version);
        preimage.extend_from_slice(&self.genesis_hash);
        preimage.extend_from_slice(&self.service_id.to_le_bytes());
        preimage.extend_from_slice(&self.action_selector);
        preimage.push(self.signer_scheme as u8);
        preimage.extend_from_slice(&self.nonce.to_le_bytes());
        preimage.extend_from_slice(&self.valid_until.to_le_bytes());
        preimage.extend_from_slice(&self.payload_hash);
        blake2_256(&preimage)
    }

    pub fn action_hash(&self) -> Result<[u8; 32], ProtocolError> {
        Ok(blake2_256(&self.encode()?))
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        validate_shape(self)?;
        let mut output = Vec::with_capacity(
            1 + 32
                + 4
                + 8
                + 1
                + 1
                + self.public_key.len()
                + 8
                + 8
                + 32
                + 1
                + self.signature.len()
                + 4
                + self.payload.len(),
        );
        output.push(self.version);
        output.extend_from_slice(&self.genesis_hash);
        output.extend_from_slice(&self.service_id.to_le_bytes());
        output.extend_from_slice(&self.action_selector);
        output.push(self.signer_scheme as u8);
        output.push(self.public_key.len() as u8);
        output.extend_from_slice(&self.public_key);
        output.extend_from_slice(&self.nonce.to_le_bytes());
        output.extend_from_slice(&self.valid_until.to_le_bytes());
        output.extend_from_slice(&self.payload_hash);
        output.push(self.signature.len() as u8);
        output.extend_from_slice(&self.signature);
        output.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        output.extend_from_slice(&self.payload);
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader { bytes, offset: 0 };
        let version = reader.u8()?;
        let genesis_hash = reader.array::<32>()?;
        let service_id = reader.u32()?;
        let action_selector = reader.array::<8>()?;
        let signer_scheme = SignerScheme::try_from(reader.u8()?)?;
        let public_key = reader.bytes_u8()?;
        let nonce = reader.u64()?;
        let valid_until = reader.u64()?;
        let payload_hash = reader.array::<32>()?;
        let signature = reader.bytes_u8()?;
        let payload = reader.bytes_u32()?;
        if reader.offset != bytes.len() {
            return Err(ProtocolError::InvalidEnvelope("trailing bytes"));
        }
        let action = Self {
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
        };
        validate_shape(&action)?;
        Ok(action)
    }

    pub fn verify(&self, context: VerifyContext) -> Result<VerifiedAction, ProtocolError> {
        validate_shape(self)?;
        if self.version != SIGNED_ACTION_VERSION_V1 {
            return Err(ProtocolError::UnsupportedVersion(self.version));
        }
        if self.genesis_hash != context.genesis_hash {
            return Err(ProtocolError::WrongNetwork);
        }
        if self.service_id != context.service_id {
            return Err(ProtocolError::WrongService);
        }
        if self.action_selector != context.action_selector {
            return Err(ProtocolError::UnknownAction);
        }
        if context.current_tick > self.valid_until {
            return Err(ProtocolError::Expired);
        }
        if let Some(expected_nonce) = context.expected_nonce {
            if self.nonce != expected_nonce {
                return Err(ProtocolError::NonceMismatch {
                    expected: expected_nonce,
                    actual: self.nonce,
                });
            }
        }
        if blake2_256(&self.payload) != self.payload_hash {
            return Err(ProtocolError::PayloadHashMismatch);
        }
        if self.signer_scheme != SignerScheme::Sr25519 {
            return Err(ProtocolError::UnsupportedSigner(self.signer_scheme));
        }
        let sender = verify_sr25519(&self.public_key, &self.signature, &self.signing_digest())
            .map_err(ProtocolError::Crypto)?;
        Ok(VerifiedAction {
            sender,
            action_hash: self.action_hash()?,
            nonce: self.nonce,
            payload: self.payload.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedActionV2 {
    pub version: u8,
    pub network_domain: [u8; 32],
    pub service_key: ServiceKeyV1,
    pub action_selector: [u8; 8],
    pub signer_scheme: SignerScheme,
    pub public_key: Vec<u8>,
    pub nonce: u64,
    pub valid_until: u64,
    pub payload_hash: [u8; 32],
    pub signature: Vec<u8>,
    pub payload: Vec<u8>,
}

impl SignedActionV2 {
    pub fn unsigned(
        network_domain: [u8; 32],
        service_key: ServiceKeyV1,
        action_selector: [u8; 8],
        public_key: [u8; 32],
        nonce: u64,
        valid_until: u64,
        payload: Vec<u8>,
    ) -> Result<Self, ProtocolError> {
        ensure_payload(&payload)?;
        Ok(Self {
            version: SIGNED_ACTION_VERSION_V2,
            network_domain,
            service_key,
            action_selector,
            signer_scheme: SignerScheme::Sr25519,
            public_key: public_key.to_vec(),
            nonce,
            valid_until,
            payload_hash: blake2_256(&payload),
            signature: Vec::new(),
            payload,
        })
    }

    pub fn signing_digest(&self) -> [u8; 32] {
        let mut preimage =
            Vec::with_capacity(SIGNING_DOMAIN_V2.len() + 1 + 32 + 32 + 8 + 1 + 8 + 8 + 32);
        preimage.extend_from_slice(SIGNING_DOMAIN_V2);
        preimage.push(self.version);
        preimage.extend_from_slice(&self.network_domain);
        preimage.extend_from_slice(self.service_key.as_bytes());
        preimage.extend_from_slice(&self.action_selector);
        preimage.push(self.signer_scheme as u8);
        preimage.extend_from_slice(&self.nonce.to_le_bytes());
        preimage.extend_from_slice(&self.valid_until.to_le_bytes());
        preimage.extend_from_slice(&self.payload_hash);
        blake2_256(&preimage)
    }

    pub fn action_hash(&self) -> Result<[u8; 32], ProtocolError> {
        Ok(blake2_256(&self.encode()?))
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        validate_shape_v2(self)?;
        let mut output = Vec::with_capacity(
            1 + 32
                + 32
                + 8
                + 1
                + 1
                + self.public_key.len()
                + 8
                + 8
                + 32
                + 1
                + self.signature.len()
                + 4
                + self.payload.len(),
        );
        output.push(self.version);
        output.extend_from_slice(&self.network_domain);
        output.extend_from_slice(self.service_key.as_bytes());
        output.extend_from_slice(&self.action_selector);
        output.push(self.signer_scheme as u8);
        output.push(self.public_key.len() as u8);
        output.extend_from_slice(&self.public_key);
        output.extend_from_slice(&self.nonce.to_le_bytes());
        output.extend_from_slice(&self.valid_until.to_le_bytes());
        output.extend_from_slice(&self.payload_hash);
        output.push(self.signature.len() as u8);
        output.extend_from_slice(&self.signature);
        output.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        output.extend_from_slice(&self.payload);
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader { bytes, offset: 0 };
        let version = reader.u8()?;
        let network_domain = reader.array::<32>()?;
        let service_key = ServiceKeyV1::decode(reader.take(32)?)
            .map_err(|_| ProtocolError::InvalidEnvelope("invalid service key"))?;
        let action_selector = reader.array::<8>()?;
        let signer_scheme = SignerScheme::try_from(reader.u8()?)?;
        let public_key = reader.bytes_u8()?;
        let nonce = reader.u64()?;
        let valid_until = reader.u64()?;
        let payload_hash = reader.array::<32>()?;
        let signature = reader.bytes_u8()?;
        let payload = reader.bytes_u32()?;
        if reader.offset != bytes.len() {
            return Err(ProtocolError::InvalidEnvelope("trailing bytes"));
        }
        let action = Self {
            version,
            network_domain,
            service_key,
            action_selector,
            signer_scheme,
            public_key,
            nonce,
            valid_until,
            payload_hash,
            signature,
            payload,
        };
        validate_shape_v2(&action)?;
        Ok(action)
    }

    pub fn verify(&self, context: VerifyContextV2) -> Result<VerifiedAction, ProtocolError> {
        validate_shape_v2(self)?;
        if self.version != SIGNED_ACTION_VERSION_V2 {
            return Err(ProtocolError::UnsupportedVersion(self.version));
        }
        if self.network_domain != context.network_domain {
            return Err(ProtocolError::WrongNetwork);
        }
        if self.service_key != context.service_key {
            return Err(ProtocolError::WrongService);
        }
        if self.action_selector != context.action_selector {
            return Err(ProtocolError::UnknownAction);
        }
        if context.current_tick > self.valid_until {
            return Err(ProtocolError::Expired);
        }
        if let Some(expected_nonce) = context.expected_nonce {
            if self.nonce != expected_nonce {
                return Err(ProtocolError::NonceMismatch {
                    expected: expected_nonce,
                    actual: self.nonce,
                });
            }
        }
        if blake2_256(&self.payload) != self.payload_hash {
            return Err(ProtocolError::PayloadHashMismatch);
        }
        if self.signer_scheme != SignerScheme::Sr25519 {
            return Err(ProtocolError::UnsupportedSigner(self.signer_scheme));
        }
        let sender = verify_sr25519(&self.public_key, &self.signature, &self.signing_digest())
            .map_err(ProtocolError::Crypto)?;
        Ok(VerifiedAction {
            sender,
            action_hash: self.action_hash()?,
            nonce: self.nonce,
            payload: self.payload.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifyContextV2 {
    pub network_domain: [u8; 32],
    pub service_key: ServiceKeyV1,
    pub action_selector: [u8; 8],
    pub current_tick: u64,
    pub expected_nonce: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifyContext {
    pub genesis_hash: [u8; 32],
    pub service_id: u32,
    pub action_selector: [u8; 8],
    pub current_tick: u64,
    pub expected_nonce: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedAction {
    pub sender: Address,
    pub action_hash: [u8; 32],
    pub nonce: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProtocolError {
    #[error("invalid envelope: {0}")]
    InvalidEnvelope(&'static str),
    #[error("unsupported envelope version {0}")]
    UnsupportedVersion(u8),
    #[error("wrong genesis/network")]
    WrongNetwork,
    #[error("wrong target service")]
    WrongService,
    #[error("unknown action selector")]
    UnknownAction,
    #[error("payload exceeds the 1 MiB bound")]
    PayloadTooLarge,
    #[error("payload hash mismatch")]
    PayloadHashMismatch,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("action expired")]
    Expired,
    #[error("nonce mismatch: expected {expected}, got {actual}")]
    NonceMismatch { expected: u64, actual: u64 },
    #[error("unsupported signer scheme {0:?}")]
    UnsupportedSigner(SignerScheme),
    #[error("cryptographic verification failed: {0}")]
    Crypto(CryptoError),
}

impl ProtocolError {
    pub fn code(&self) -> u32 {
        match self {
            Self::InvalidEnvelope(_) => 1,
            Self::UnsupportedVersion(_) => 2,
            Self::WrongNetwork => 3,
            Self::WrongService => 4,
            Self::UnknownAction => 5,
            Self::PayloadTooLarge => 6,
            Self::PayloadHashMismatch => 7,
            Self::InvalidSignature | Self::Crypto(_) => 8,
            Self::Expired => 9,
            Self::NonceMismatch { .. } => 10,
            Self::UnsupportedSigner(_) => 11,
        }
    }
}

fn validate_shape(action: &SignedActionV1) -> Result<(), ProtocolError> {
    if action.public_key.len() > MAX_PUBLIC_KEY_BYTES {
        return Err(ProtocolError::InvalidEnvelope("public key is too large"));
    }
    if action.signature.len() > MAX_SIGNATURE_BYTES {
        return Err(ProtocolError::InvalidEnvelope("signature is too large"));
    }
    ensure_payload(&action.payload)?;
    Ok(())
}

fn validate_shape_v2(action: &SignedActionV2) -> Result<(), ProtocolError> {
    if action.public_key.len() > MAX_PUBLIC_KEY_BYTES {
        return Err(ProtocolError::InvalidEnvelope("public key is too large"));
    }
    if action.signature.len() > MAX_SIGNATURE_BYTES {
        return Err(ProtocolError::InvalidEnvelope("signature is too large"));
    }
    ensure_payload(&action.payload)
}

fn ensure_payload(payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        Err(ProtocolError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ProtocolError::InvalidEnvelope("length overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(ProtocolError::InvalidEnvelope("truncated envelope"))?;
        self.offset = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(*self.take(1)?.first().unwrap())
    }
    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn bytes_u8(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let length = self.u8()? as usize;
        Ok(self.take(length)?.to_vec())
    }
    fn bytes_u32(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let length = self.u32()? as usize;
        ensure_payload_length(length)?;
        Ok(self.take(length)?.to_vec())
    }
}
fn ensure_payload_length(length: usize) -> Result<(), ProtocolError> {
    if length > MAX_PAYLOAD_BYTES {
        Err(ProtocolError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jamscript_crypto::{derive_address, SR25519_CONTEXT};
    use jamscript_ir::action_selector;
    use schnorrkel::{context::signing_context, ExpansionMode, MiniSecretKey};

    fn signed(seed: u8) -> SignedActionV1 {
        let keypair = MiniSecretKey::from_bytes(&[seed; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let mut action = SignedActionV1::unsigned(
            [1; 32],
            182,
            action_selector("increment"),
            keypair.public.to_bytes(),
            4,
            20,
            7u64.to_le_bytes().to_vec(),
        )
        .unwrap();
        let signature =
            keypair.sign(signing_context(SR25519_CONTEXT).bytes(&action.signing_digest()));
        action.signature = signature.to_bytes().to_vec();
        action
    }

    fn context() -> VerifyContext {
        VerifyContext {
            genesis_hash: [1; 32],
            service_id: 182,
            action_selector: action_selector("increment"),
            current_tick: 10,
            expected_nonce: Some(4),
        }
    }

    fn signed_v2(seed: u8) -> SignedActionV2 {
        let keypair = MiniSecretKey::from_bytes(&[seed; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let mut action = SignedActionV2::unsigned(
            [3; 32],
            ServiceKeyV1::new([4; 32]),
            action_selector("increment"),
            keypair.public.to_bytes(),
            4,
            20,
            7u64.to_le_bytes().to_vec(),
        )
        .unwrap();
        action.signature = from_hex("2c83feb138f6a94e02ac286e0f01f179ed7f8968d5a4e3d4d8913013e671e7193003eab0f6bf0bb5902c92b0542db8055c799dd766f16f3401dc0759a02d9f8a");
        action
    }

    fn context_v2() -> VerifyContextV2 {
        VerifyContextV2 {
            network_domain: [3; 32],
            service_key: ServiceKeyV1::new([4; 32]),
            action_selector: action_selector("increment"),
            current_tick: 10,
            expected_nonce: Some(4),
        }
    }

    fn from_hex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn valid_action_verifies_and_round_trips() {
        let action = signed(7);
        let encoded = action.encode().unwrap();
        let decoded = SignedActionV1::decode(&encoded).unwrap();
        let verified = decoded.verify(context()).unwrap();
        assert_eq!(verified.sender, derive_address(&action.public_key).unwrap());
        assert_eq!(verified.payload, 7u64.to_le_bytes());
        assert_eq!(verified.action_hash, action.action_hash().unwrap());
    }

    #[test]
    fn v2_service_key_action_verifies_and_round_trips() {
        let action = signed_v2(7);
        let encoded = action.encode().unwrap();
        let decoded = SignedActionV2::decode(&encoded).unwrap();
        let verified = decoded.verify(context_v2()).unwrap();
        assert_eq!(
            action.signing_digest().as_slice(),
            from_hex("58a1fbab9e0cfd0e73ca84a7b7c5c1e8132f2849e24a69b7ff09e9c4bb4aaa0e")
        );
        assert_eq!(encoded, from_hex("0203030303030303030303030303030303030303030303030303030303030303030404040404040404040404040404040404040404040404040404040404040404f545ebc54c37147200207c0f469d3bd340bae718203fa30ca071a5e37c751e891dbded837b213d45d91d04000000000000001400000000000000acf9bb149d15061f83c799e679c7917955226ca0ac44ae05155e4a89c67b399d402c83feb138f6a94e02ac286e0f01f179ed7f8968d5a4e3d4d8913013e671e7193003eab0f6bf0bb5902c92b0542db8055c799dd766f16f3401dc0759a02d9f8a080000000700000000000000"));
        assert_eq!(verified.sender, derive_address(&action.public_key).unwrap());
        assert_eq!(verified.payload, 7u64.to_le_bytes());
        assert_eq!(verified.action_hash, action.action_hash().unwrap());
        let mut relocated = context_v2();
        relocated.service_key = ServiceKeyV1::new([5; 32]);
        assert_eq!(decoded.verify(relocated), Err(ProtocolError::WrongService));
    }

    #[test]
    fn security_domain_fields_cannot_be_modified() {
        let original = signed(7);
        for check in [
            |a: &mut SignedActionV1| a.payload[0] ^= 1,
            |a: &mut SignedActionV1| a.service_id = 183,
            |a: &mut SignedActionV1| a.action_selector = action_selector("other"),
            |a: &mut SignedActionV1| a.genesis_hash = [2; 32],
            |a: &mut SignedActionV1| a.nonce = 5,
            |a: &mut SignedActionV1| a.valid_until = 21,
        ] {
            let mut modified = original.clone();
            check(&mut modified);
            let result = modified.verify(context());
            assert!(
                result.is_err(),
                "modified action unexpectedly verified: {modified:?}"
            );
        }
    }

    #[test]
    fn rejects_expiry_nonce_and_trailing_bytes() {
        let mut expired = signed(7);
        expired.valid_until = 9;
        assert_eq!(expired.verify(context()), Err(ProtocolError::Expired));
        let mut wrong_nonce = signed(7);
        wrong_nonce.nonce = 5;
        assert_eq!(
            wrong_nonce.verify(context()),
            Err(ProtocolError::NonceMismatch {
                expected: 4,
                actual: 5
            })
        );
        let encoded = signed(7).encode().unwrap();
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert_eq!(
            SignedActionV1::decode(&trailing),
            Err(ProtocolError::InvalidEnvelope("trailing bytes"))
        );
    }

    #[test]
    fn different_accounts_have_different_senders() {
        let first = signed(7).verify(context()).unwrap();
        let second = signed(8).verify(context()).unwrap();
        assert_ne!(first.sender, second.sender);
    }
}
