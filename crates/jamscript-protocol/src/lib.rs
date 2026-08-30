use jamscript_crypto::{blake2_256, verify_sr25519, Address, CryptoError};
use service_runtime_core::ServiceKeyV1;
use thiserror::Error;

pub const SIGNED_ACTION_VERSION_V1: u8 = 1;
pub const SIGNING_DOMAIN_V1: &[u8] = b"JAMSCRIPT_ACTION_V1";
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

impl SignedActionV1 {
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
            version: SIGNED_ACTION_VERSION_V1,
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
            Vec::with_capacity(SIGNING_DOMAIN_V1.len() + 1 + 32 + 32 + 8 + 1 + 8 + 8 + 32);
        preimage.extend_from_slice(SIGNING_DOMAIN_V1);
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
        validate_shape_v1(self)?;
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
        if version != SIGNED_ACTION_VERSION_V1 {
            return Err(ProtocolError::UnsupportedVersion(version));
        }
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
        validate_shape_v1(&action)?;
        Ok(action)
    }

    pub fn verify(&self, context: VerifyContextV1) -> Result<VerifiedAction, ProtocolError> {
        validate_shape_v1(self)?;
        if self.version != SIGNED_ACTION_VERSION_V1 {
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
pub struct VerifyContextV1 {
    pub network_domain: [u8; 32],
    pub service_key: ServiceKeyV1,
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

fn validate_shape_v1(action: &SignedActionV1) -> Result<(), ProtocolError> {
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
    use jamscript_crypto::derive_address;
    use jamscript_ir::action_selector;
    use schnorrkel::{ExpansionMode, MiniSecretKey};

    fn signed(seed: u8) -> SignedActionV1 {
        let keypair = MiniSecretKey::from_bytes(&[seed; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let mut action = SignedActionV1::unsigned(
            [3; 32],
            ServiceKeyV1::new([4; 32]),
            action_selector("increment"),
            keypair.public.to_bytes(),
            4,
            20,
            7u64.to_le_bytes().to_vec(),
        )
        .unwrap();
        action.signature = from_hex("d615611e1047cd2e9a3fa0062506a2795ec8a27004ae8079dbe22f7383dc791b8cfe98266833889102bdf3a7757f0a9e89ced6017fe42c07e4e65620cacf8182");
        action
    }

    fn context() -> VerifyContextV1 {
        VerifyContextV1 {
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
    fn formal_v1_service_key_action_verifies_and_round_trips() {
        let action = signed(7);
        let encoded = action.encode().unwrap();
        let decoded = SignedActionV1::decode(&encoded).unwrap();
        let verified = decoded.verify(context()).unwrap();
        assert_eq!(
            action.signing_digest().as_slice(),
            from_hex("e2b7e99b5b6bac326f7973a05136dca8a78e39420280673983b807042340b77a")
        );
        assert_eq!(
            encoded,
            from_hex("0103030303030303030303030303030303030303030303030303030303030303030404040404040404040404040404040404040404040404040404040404040404f545ebc54c37147200207c0f469d3bd340bae718203fa30ca071a5e37c751e891dbded837b213d45d91d04000000000000001400000000000000acf9bb149d15061f83c799e679c7917955226ca0ac44ae05155e4a89c67b399d40d615611e1047cd2e9a3fa0062506a2795ec8a27004ae8079dbe22f7383dc791b8cfe98266833889102bdf3a7757f0a9e89ced6017fe42c07e4e65620cacf8182080000000700000000000000")
        );
        assert_eq!(
            action.action_hash().unwrap().as_slice(),
            from_hex("b837e3985f982d07b2c25c0f4b52558010186de80aa2ec8fc7d0ddf0fe45f0a7").as_slice()
        );
        assert_eq!(verified.sender, derive_address(&action.public_key).unwrap());
        assert_eq!(verified.payload, 7u64.to_le_bytes());
        assert_eq!(verified.action_hash, action.action_hash().unwrap());
        let mut relocated = context();
        relocated.service_key = ServiceKeyV1::new([5; 32]);
        assert_eq!(decoded.verify(relocated), Err(ProtocolError::WrongService));
    }

    #[test]
    fn old_v2_action_is_rejected_by_the_formal_decoder() {
        let mut encoded = signed(7).encode().unwrap();
        encoded[0] = 2;
        assert_eq!(
            SignedActionV1::decode(&encoded),
            Err(ProtocolError::UnsupportedVersion(2))
        );
    }
}
