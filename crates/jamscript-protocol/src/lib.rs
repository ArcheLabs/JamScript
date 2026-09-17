use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jamscript_crypto::{
    blake2_256, verify_evm_ownership, verify_ownership, verify_polkadot_ownership, verify_sr25519,
    Address, CryptoError,
};
use ownership_core::{Ownership, OwnershipError, OwnershipKind};
use service_runtime_core::ServiceKeyV1;
use thiserror::Error;

pub const SIGNED_ACTION_VERSION_V1: u8 = 1;
pub const SIGNING_DOMAIN_V1: &[u8] = b"JAMSCRIPT_ACTION_V1";
pub const MAX_PAYLOAD_BYTES: usize = 1_048_576;
pub const MAX_PUBLIC_KEY_BYTES: usize = 32;
pub const MAX_SIGNATURE_BYTES: usize = 64;
pub const SIGNED_ACTION_VERSION_V2: u8 = 2;
pub const ACTION_COMMITMENT_DOMAIN_V2: &[u8] = b"JAMSCRIPT_ACTION_V2";
pub const MAX_AUTHORIZATION_PROOF_BYTES: usize = 65_536;
pub const CONTROL_CLAIM_FORMAT_VERSION_V1: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlClaimActionV1 {
    AddController {
        subject: Ownership,
        controller: Ownership,
    },
    RevokeController {
        subject: Ownership,
        controller: Ownership,
    },
}

impl ControlClaimActionV1 {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let (tag, subject, controller) = match self {
            Self::AddController {
                subject,
                controller,
            } => (0, subject, controller),
            Self::RevokeController {
                subject,
                controller,
            } => (1, subject, controller),
        };
        let subject = subject
            .encode()
            .map_err(ProtocolError::from_ownership_error)?;
        let controller = controller
            .encode()
            .map_err(ProtocolError::from_ownership_error)?;
        let mut output = Vec::with_capacity(5 + subject.len() + controller.len());
        output.push(CONTROL_CLAIM_FORMAT_VERSION_V1);
        output.push(tag);
        output.extend_from_slice(&(subject.len() as u16).to_le_bytes());
        output.extend_from_slice(&subject);
        output.extend_from_slice(&(controller.len() as u16).to_le_bytes());
        output.extend_from_slice(&controller);
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader { bytes, offset: 0 };
        if reader.u8()? != CONTROL_CLAIM_FORMAT_VERSION_V1 {
            return Err(ProtocolError::InvalidEnvelope(
                "unsupported ControlClaim version",
            ));
        }
        let tag = reader.u8()?;
        let subject_bytes = reader.bytes_u16()?;
        let subject =
            Ownership::decode(&subject_bytes).map_err(ProtocolError::from_ownership_error)?;
        let controller_bytes = reader.bytes_u16()?;
        let controller =
            Ownership::decode(&controller_bytes).map_err(ProtocolError::from_ownership_error)?;
        if reader.offset != bytes.len() {
            return Err(ProtocolError::InvalidEnvelope(
                "trailing ControlClaim bytes",
            ));
        }
        match tag {
            0 => Ok(Self::AddController {
                subject,
                controller,
            }),
            1 => Ok(Self::RevokeController {
                subject,
                controller,
            }),
            _ => Err(ProtocolError::InvalidEnvelope(
                "unknown ControlClaim action",
            )),
        }
    }
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedActionV2 {
    pub version: u8,
    pub network_domain: [u8; 32],
    pub service_key: ServiceKeyV1,
    pub action_selector: [u8; 8],
    pub controller: Ownership,
    pub act_as: Option<Ownership>,
    pub nonce: u64,
    pub valid_until: u64,
    pub payload_hash: [u8; 32],
    pub authorization_proof: Vec<u8>,
    pub payload: Vec<u8>,
}

impl SignedActionV2 {
    pub fn unsigned(
        network_domain: [u8; 32],
        service_key: ServiceKeyV1,
        action_selector: [u8; 8],
        controller: Ownership,
        act_as: Option<Ownership>,
        nonce: u64,
        valid_until: u64,
        payload: Vec<u8>,
    ) -> Result<Self, ProtocolError> {
        ensure_payload(&payload)?;
        controller
            .validate()
            .map_err(ProtocolError::from_ownership_error)?;
        if let Some(owner) = &act_as {
            owner
                .validate()
                .map_err(ProtocolError::from_ownership_error)?;
        }
        Ok(Self {
            version: SIGNED_ACTION_VERSION_V2,
            network_domain,
            service_key,
            action_selector,
            controller,
            act_as,
            nonce,
            valid_until,
            payload_hash: blake2_256(&payload),
            authorization_proof: Vec::new(),
            payload,
        })
    }

    pub fn action_commitment(&self) -> Result<[u8; 32], ProtocolError> {
        validate_shape_v2(self)?;
        let mut preimage = Vec::new();
        preimage.extend_from_slice(ACTION_COMMITMENT_DOMAIN_V2);
        preimage.extend_from_slice(&self.network_domain);
        preimage.extend_from_slice(self.service_key.as_bytes());
        preimage.extend_from_slice(&self.action_selector);
        preimage.extend_from_slice(
            &self
                .controller
                .encode()
                .map_err(ProtocolError::from_ownership_error)?,
        );
        match &self.act_as {
            Some(owner) => {
                preimage.push(1);
                preimage.extend_from_slice(
                    &owner
                        .encode()
                        .map_err(ProtocolError::from_ownership_error)?,
                );
            }
            None => preimage.push(0),
        }
        preimage.extend_from_slice(&self.nonce.to_le_bytes());
        preimage.extend_from_slice(&self.valid_until.to_le_bytes());
        preimage.extend_from_slice(&self.payload_hash);
        Ok(blake2_256(&preimage))
    }

    pub fn action_hash(&self) -> Result<[u8; 32], ProtocolError> {
        Ok(blake2_256(&self.encode()?))
    }

    pub fn signing_message(&self) -> Result<Vec<u8>, ProtocolError> {
        let encoded = URL_SAFE_NO_PAD.encode(self.action_commitment()?);
        let mut message = Vec::with_capacity(22 + encoded.len());
        message.extend_from_slice(b"JAMSCRIPT_ACTION_V2:");
        message.extend_from_slice(encoded.as_bytes());
        Ok(message)
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        validate_shape_v2(self)?;
        let controller = self
            .controller
            .encode()
            .map_err(ProtocolError::from_ownership_error)?;
        let act_as = self
            .act_as
            .as_ref()
            .map(|owner| owner.encode().map_err(ProtocolError::from_ownership_error))
            .transpose()?;
        let mut output = Vec::new();
        output.push(self.version);
        output.extend_from_slice(&self.network_domain);
        output.extend_from_slice(self.service_key.as_bytes());
        output.extend_from_slice(&self.action_selector);
        output.extend_from_slice(&(controller.len() as u16).to_le_bytes());
        output.extend_from_slice(&controller);
        match act_as {
            Some(owner) => {
                output.push(1);
                output.extend_from_slice(&(owner.len() as u16).to_le_bytes());
                output.extend_from_slice(&owner);
            }
            None => output.push(0),
        }
        output.extend_from_slice(&self.nonce.to_le_bytes());
        output.extend_from_slice(&self.valid_until.to_le_bytes());
        output.extend_from_slice(&self.payload_hash);
        output.extend_from_slice(&(self.authorization_proof.len() as u32).to_le_bytes());
        output.extend_from_slice(&self.authorization_proof);
        output.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        output.extend_from_slice(&self.payload);
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader { bytes, offset: 0 };
        let version = reader.u8()?;
        if version != SIGNED_ACTION_VERSION_V2 {
            return Err(ProtocolError::UnsupportedVersion(version));
        }
        let network_domain = reader.array::<32>()?;
        let service_key = ServiceKeyV1::decode(reader.take(32)?)
            .map_err(|_| ProtocolError::InvalidEnvelope("invalid service key"))?;
        let action_selector = reader.array::<8>()?;
        let controller_bytes = reader.bytes_u16()?;
        let controller =
            Ownership::decode(&controller_bytes).map_err(ProtocolError::from_ownership_error)?;
        let act_as = match reader.u8()? {
            0 => None,
            1 => {
                let act_as_bytes = reader.bytes_u16()?;
                Some(
                    Ownership::decode(&act_as_bytes)
                        .map_err(ProtocolError::from_ownership_error)?,
                )
            }
            _ => return Err(ProtocolError::InvalidEnvelope("invalid act_as flag")),
        };
        let nonce = reader.u64()?;
        let valid_until = reader.u64()?;
        let payload_hash = reader.array::<32>()?;
        let authorization_proof = reader.bytes_u32_limited(MAX_AUTHORIZATION_PROOF_BYTES)?;
        let payload = reader.bytes_u32()?;
        if reader.offset != bytes.len() {
            return Err(ProtocolError::InvalidEnvelope("trailing envelope bytes"));
        }
        let action = Self {
            version,
            network_domain,
            service_key,
            action_selector,
            controller,
            act_as,
            nonce,
            valid_until,
            payload_hash,
            authorization_proof,
            payload,
        };
        validate_shape_v2(&action)?;
        Ok(action)
    }

    pub fn verify(
        &self,
        context: VerifyContextV2,
    ) -> Result<VerifiedOwnershipAction, ProtocolError> {
        validate_shape_v2(self)?;
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
                return Err(ProtocolError::OwnershipNonceMismatch {
                    expected: expected_nonce,
                    actual: self.nonce,
                });
            }
        }
        if blake2_256(&self.payload) != self.payload_hash {
            return Err(ProtocolError::PayloadHashMismatch);
        }
        if self.act_as.is_some() && !context.active_control_claim {
            return Err(ProtocolError::ControlClaimNotFound);
        }
        if self.authorization_proof.is_empty() {
            return Err(ProtocolError::InvalidOwnershipAuthorization);
        }
        let signing_message = self.signing_message()?;
        match self.controller.kind {
            OwnershipKind::Secp256k1Keccak20 => verify_evm_ownership(
                &self.controller,
                &self.authorization_proof,
                &self.network_domain,
                &self.action_commitment()?,
            ),
            OwnershipKind::MulticryptoAccount32 => verify_polkadot_ownership(
                &self.controller,
                &self.authorization_proof,
                &signing_message,
            ),
            _ => verify_ownership(
                &self.controller,
                &self.authorization_proof,
                &signing_message,
            ),
        }
        .map_err(map_ownership_auth_error)?;
        Ok(VerifiedOwnershipAction {
            owner: self
                .act_as
                .clone()
                .unwrap_or_else(|| self.controller.clone()),
            controller: self.controller.clone(),
            action_hash: self.action_hash()?,
            action_selector: self.action_selector,
            nonce: self.nonce,
            valid_until: self.valid_until,
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
    pub active_control_claim: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedOwnershipAction {
    pub owner: Ownership,
    pub controller: Ownership,
    pub action_hash: [u8; 32],
    pub action_selector: [u8; 8],
    pub nonce: u64,
    pub valid_until: u64,
    pub payload: Vec<u8>,
}

fn validate_shape_v2(action: &SignedActionV2) -> Result<(), ProtocolError> {
    if action.version != SIGNED_ACTION_VERSION_V2 {
        return Err(ProtocolError::UnsupportedVersion(action.version));
    }
    action
        .controller
        .validate()
        .map_err(ProtocolError::from_ownership_error)?;
    if let Some(owner) = &action.act_as {
        owner
            .validate()
            .map_err(ProtocolError::from_ownership_error)?;
    }
    if action.authorization_proof.len() > MAX_AUTHORIZATION_PROOF_BYTES {
        return Err(ProtocolError::InvalidOwnershipAuthorization);
    }
    ensure_payload(&action.payload)
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
    #[error("invalid Ownership encoding")]
    InvalidOwnershipEncoding,
    #[error("unsupported Ownership kind")]
    UnsupportedOwnershipKind,
    #[error("invalid Ownership public value")]
    InvalidOwnershipPublicValue,
    #[error("invalid Ownership authorization")]
    InvalidOwnershipAuthorization,
    #[error("Ownership nonce mismatch: expected {expected}, got {actual}")]
    OwnershipNonceMismatch { expected: u64, actual: u64 },
    #[error("ControlClaim not found")]
    ControlClaimNotFound,
    #[error("invalid ControlClaim")]
    InvalidControlClaim,
    #[error("ControlClaim is revoked")]
    ControlClaimRevoked,
    #[error("ControlClaim already exists")]
    ControlClaimAlreadyExists,
    #[error("ControlClaim subject mismatch")]
    ControlSubjectMismatch,
    #[error("controller mismatch")]
    ControllerMismatch,
    #[error("ControlClaim is already initialized")]
    ControlAlreadyInitialized,
    #[error("invalid Polkadot signature")]
    PolkadotInvalidSignature,
    #[error("invalid EVM signature")]
    EvmInvalidSignature,
    #[error("EVM address mismatch")]
    EvmAddressMismatch,
    #[error("invalid Matrix master proof")]
    MatrixInvalidMasterProof,
    #[error("invalid Matrix self-signing proof")]
    MatrixInvalidSelfSigningProof,
    #[error("invalid Matrix device proof")]
    MatrixInvalidDeviceProof,
    #[error("Matrix device is not cross-signed")]
    MatrixDeviceNotCrossSigned,
    #[error("Matrix bootstrap already completed")]
    MatrixBootstrapAlreadyCompleted,
    #[error("unsupported Matrix device profile")]
    MatrixUnsupportedDeviceProfile,
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
            Self::InvalidOwnershipEncoding
            | Self::UnsupportedOwnershipKind
            | Self::InvalidOwnershipPublicValue => 15,
            Self::InvalidOwnershipAuthorization => 16,
            Self::OwnershipNonceMismatch { .. } => 17,
            Self::ControlClaimNotFound => 18,
            Self::InvalidControlClaim => 19,
            Self::ControlClaimRevoked => 20,
            Self::ControlClaimAlreadyExists => 21,
            Self::ControlSubjectMismatch => 22,
            Self::ControllerMismatch => 23,
            Self::ControlAlreadyInitialized => 24,
            Self::PolkadotInvalidSignature => 25,
            Self::EvmInvalidSignature => 26,
            Self::EvmAddressMismatch => 27,
            Self::MatrixInvalidMasterProof => 28,
            Self::MatrixInvalidSelfSigningProof => 29,
            Self::MatrixInvalidDeviceProof => 30,
            Self::MatrixDeviceNotCrossSigned => 31,
            Self::MatrixBootstrapAlreadyCompleted => 32,
            Self::MatrixUnsupportedDeviceProfile => 33,
        }
    }

    fn from_ownership_error(error: OwnershipError) -> Self {
        match error {
            OwnershipError::InvalidVersion | OwnershipError::InvalidEncoding => {
                Self::InvalidOwnershipEncoding
            }
            OwnershipError::UnsupportedKind(_) => Self::UnsupportedOwnershipKind,
            OwnershipError::InvalidPublicLength => Self::InvalidOwnershipEncoding,
            OwnershipError::InvalidPublicValue => Self::InvalidOwnershipPublicValue,
            OwnershipError::TooLarge => Self::InvalidOwnershipEncoding,
        }
    }
}

fn map_ownership_auth_error(error: CryptoError) -> ProtocolError {
    match error {
        CryptoError::EvmAddressMismatch => ProtocolError::EvmAddressMismatch,
        CryptoError::EvmHighS
        | CryptoError::InvalidEcdsaSignature
        | CryptoError::EcdsaRecoveryFailed => ProtocolError::EvmInvalidSignature,
        CryptoError::InvalidPolkadotAuthorization | CryptoError::PolkadotAddressMismatch => {
            ProtocolError::PolkadotInvalidSignature
        }
        _ => ProtocolError::InvalidOwnershipAuthorization,
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
    fn bytes_u16(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let length = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
        Ok(self.take(length)?.to_vec())
    }
    fn bytes_u32_limited(&mut self, limit: usize) -> Result<Vec<u8>, ProtocolError> {
        let length = self.u32()? as usize;
        if length > limit {
            return Err(ProtocolError::InvalidOwnershipAuthorization);
        }
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
    use ed25519_dalek::{Signer, SigningKey};
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

    #[test]
    fn ownership_v2_round_trips_and_verifies_effective_owner() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let controller = Ownership::from_array(
            ownership_core::OwnershipKind::Ed25519Key,
            signing_key.verifying_key().to_bytes(),
        )
        .unwrap();
        let owner =
            Ownership::from_array(ownership_core::OwnershipKind::Ed25519Key, [8; 32]).unwrap();
        let mut action = SignedActionV2::unsigned(
            [3; 32],
            ServiceKeyV1::new([4; 32]),
            [5; 8],
            controller.clone(),
            Some(owner.clone()),
            2,
            20,
            b"payload".to_vec(),
        )
        .unwrap();
        let message = action.signing_message().unwrap();
        action.authorization_proof = signing_key.sign(&message).to_bytes().to_vec();
        let encoded = action.encode().unwrap();
        let decoded = SignedActionV2::decode(&encoded).unwrap();
        let verified = decoded
            .verify(VerifyContextV2 {
                network_domain: [3; 32],
                service_key: ServiceKeyV1::new([4; 32]),
                action_selector: [5; 8],
                current_tick: 10,
                expected_nonce: Some(2),
                active_control_claim: true,
            })
            .unwrap();
        assert_eq!(verified.owner, owner);
        assert_eq!(verified.controller, controller);
        assert_eq!(verified.payload, b"payload");
    }
}
