#![no_std]

//! First-party Ownership Control system service.
//!
//! The service is deliberately separate from consumer applications.  It is
//! the only service allowed to mutate the runtime ControlClaim namespace;
//! consumer services only receive proof-backed reads of this state.

extern crate alloc;

use alloc::vec::Vec;
use jamscript_protocol::{ControlClaimActionV1, ProtocolError};
use jamscript_runtime_core::{
    control_claim_bootstrap_key, control_claim_key, ownership_nonce_key, RuntimeError,
};
use service_runtime_core::{ExecutionContext, ServiceApplication, ServiceKeyV1, StateAccessError};

/// Stable logical identity.  The numeric ServiceId remains network-specific.
pub const OWNERSHIP_CONTROL_SERVICE_KEY_V1: ServiceKeyV1 =
    ServiceKeyV1::new(*b"JAMSCRIPT_OWNERSHIP_CONTROL_V1__");

pub const BOOTSTRAP_MATRIX_CONTROLLER_ACTION: &str = "bootstrapMatrixController";
pub const ADD_CONTROLLER_ACTION: &str = "addController";
pub const REVOKE_CONTROLLER_ACTION: &str = "revokeController";

fn selector(name: &[u8]) -> [u8; 8] {
    let mut input = Vec::with_capacity(20 + name.len());
    input.extend_from_slice(b"jamscript/action/v1:");
    input.extend_from_slice(name);
    let hash = service_runtime_core::blake2_256(&input);
    let mut output = [0; 8];
    output.copy_from_slice(&hash[..8]);
    output
}

fn reject(error: RuntimeError) -> StateAccessError {
    StateAccessError::Rejected(error.code())
}

fn invalid_claim() -> StateAccessError {
    reject(RuntimeError::InvalidControlClaim)
}

fn decode_claim_action(bytes: &[u8]) -> Result<ControlClaimActionV1, StateAccessError> {
    ControlClaimActionV1::decode(bytes).map_err(|_| invalid_claim())
}

fn map_protocol_error(error: ProtocolError) -> StateAccessError {
    match error {
        ProtocolError::MatrixInvalidMasterProof => reject(RuntimeError::MatrixInvalidMasterProof),
        ProtocolError::MatrixInvalidSelfSigningProof => {
            reject(RuntimeError::MatrixInvalidSelfSigningProof)
        }
        ProtocolError::MatrixInvalidDeviceProof => reject(RuntimeError::MatrixInvalidDeviceProof),
        ProtocolError::MatrixDeviceNotCrossSigned => {
            reject(RuntimeError::MatrixDeviceNotCrossSigned)
        }
        ProtocolError::MatrixBootstrapAlreadyCompleted => {
            reject(RuntimeError::MatrixBootstrapAlreadyCompleted)
        }
        _ => invalid_claim(),
    }
}

pub struct OwnershipControlService;

impl OwnershipControlService {
    fn execute_action(
        &self,
        context: &mut ExecutionContext<'_>,
        raw_action: &[u8],
    ) -> Result<(), StateAccessError> {
        let signed = jamscript_runtime_core::decode_signed_action_v2(raw_action).map_err(reject)?;
        let (expected_selector, payload_action) =
            if signed.action_selector == selector(BOOTSTRAP_MATRIX_CONTROLLER_ACTION.as_bytes()) {
                (signed.action_selector, decode_claim_action(signed.payload)?)
            } else if signed.action_selector == selector(ADD_CONTROLLER_ACTION.as_bytes()) {
                (signed.action_selector, decode_claim_action(signed.payload)?)
            } else if signed.action_selector == selector(REVOKE_CONTROLLER_ACTION.as_bytes()) {
                (signed.action_selector, decode_claim_action(signed.payload)?)
            } else {
                return Err(reject(RuntimeError::UnknownAction));
            };

        let nonce_owner = signed.act_as.as_ref().unwrap_or(&signed.controller);
        let nonce_key = ownership_nonce_key(nonce_owner).map_err(reject)?;
        let nonce_bytes = context.state().get(&nonce_key)?.unwrap_or_default();
        let expected_nonce = match nonce_bytes.as_slice() {
            [] => 0,
            bytes if bytes.len() == 8 => {
                u64::from_le_bytes(bytes.try_into().map_err(|_| StateAccessError::Backend)?)
            }
            _ => return Err(StateAccessError::Backend),
        };

        let active_controller_claim = match &payload_action {
            ControlClaimActionV1::AddController { subject, .. }
            | ControlClaimActionV1::RevokeController { subject, .. } => {
                if signed.act_as.as_ref() != Some(subject) {
                    return Err(reject(RuntimeError::ControlSubjectMismatch));
                }
                let key = control_claim_key(subject, &signed.controller).map_err(reject)?;
                matches!(context.state().get(&key)?.as_deref(), Some([1]))
            }
            ControlClaimActionV1::BootstrapMatrix { .. } => false,
        };

        let delegated = signed.act_as.is_some();
        let verified = jamscript_runtime_core::verify_signed_action_v2(
            signed,
            context.network_domain(),
            OWNERSHIP_CONTROL_SERVICE_KEY_V1,
            expected_selector,
            Some(expected_nonce),
            active_controller_claim,
        )
        .map_err(reject)?;
        context.constrain_valid_until(verified.valid_until);

        match payload_action {
            ControlClaimActionV1::BootstrapMatrix { bootstrap } => {
                if delegated {
                    return Err(reject(RuntimeError::InvalidControlClaim));
                }
                if bootstrap.controller != verified.controller {
                    return Err(reject(RuntimeError::ControllerMismatch));
                }
                let bootstrap_key =
                    control_claim_bootstrap_key(&bootstrap.subject).map_err(reject)?;
                if context.state().get(&bootstrap_key)?.is_some() {
                    return Err(reject(RuntimeError::ControlAlreadyInitialized));
                }
                bootstrap
                    .verify(context.network_domain())
                    .map_err(map_protocol_error)?;
                let claim_key =
                    control_claim_key(&bootstrap.subject, &bootstrap.controller).map_err(reject)?;
                context.state().set(&bootstrap_key, &[1])?;
                context.state().set(&claim_key, &[1])?;
            }
            ControlClaimActionV1::AddController {
                subject,
                controller,
            } => {
                let claim_key = control_claim_key(&subject, &controller).map_err(reject)?;
                if context.state().get(&claim_key)?.is_some() {
                    return Err(reject(RuntimeError::ControlClaimAlreadyExists));
                }
                context.state().set(&claim_key, &[1])?;
            }
            ControlClaimActionV1::RevokeController {
                subject,
                controller,
            } => {
                let claim_key = control_claim_key(&subject, &controller).map_err(reject)?;
                match context.state().get(&claim_key)?.as_deref() {
                    Some([1]) => context.state().set(&claim_key, &[0])?,
                    Some([0]) => return Err(reject(RuntimeError::ControlClaimRevoked)),
                    _ => return Err(reject(RuntimeError::ControlClaimNotFound)),
                }
            }
        }
        let next_nonce = expected_nonce
            .checked_add(1)
            .ok_or(StateAccessError::Backend)?;
        context.state().set(&nonce_key, &next_nonce.to_le_bytes())?;
        Ok(())
    }
}

impl ServiceApplication for OwnershipControlService {
    type Error = StateAccessError;

    fn execute(
        &self,
        context: &mut ExecutionContext<'_>,
        raw_action: &[u8],
    ) -> Result<(), Self::Error> {
        self.execute_action(context, raw_action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use ed25519_dalek::{Signer, SigningKey};
    use service_runtime_core::ExecutionEnvironmentV1;
    use service_runtime_state::{FullState, StateTransaction};

    fn owner(key: &SigningKey) -> ownership_core::Ownership {
        ownership_core::Ownership::from_array(
            ownership_core::OwnershipKind::Ed25519Key,
            key.verifying_key().to_bytes(),
        )
        .unwrap()
    }

    fn sign_action(
        name: &str,
        controller: &SigningKey,
        act_as: Option<ownership_core::Ownership>,
        nonce: u64,
        payload: Vec<u8>,
    ) -> Vec<u8> {
        let mut action = jamscript_protocol::SignedActionV2::unsigned(
            [9; 32],
            OWNERSHIP_CONTROL_SERVICE_KEY_V1,
            selector(name.as_bytes()),
            owner(controller),
            act_as,
            nonce,
            100,
            payload,
        )
        .unwrap();
        action.authorization_proof = controller
            .sign(&action.signing_message().unwrap())
            .to_bytes()
            .to_vec();
        action.encode().unwrap()
    }

    fn run(state: FullState, action: &[u8]) -> (FullState, Result<(), StateAccessError>) {
        let original = state.clone();
        let mut transaction = StateTransaction::new(state);
        let mut context = ExecutionContext::new(
            &mut transaction,
            None,
            ExecutionEnvironmentV1 {
                network_domain: [9; 32],
                ownership_control_service_id: Some(77),
            },
        );
        let result = OwnershipControlService.execute(&mut context, action);
        drop(context);
        match result {
            Ok(()) => {
                let (next, _) = transaction.finish().unwrap();
                (next, Ok(()))
            }
            Err(error) => {
                transaction.discard();
                (original, Err(error))
            }
        }
    }

    #[test]
    fn selector_domain_is_stable_and_service_key_is_not_a_numeric_id() {
        assert_ne!(selector(ADD_CONTROLLER_ACTION.as_bytes()), [0; 8]);
        assert_eq!(OWNERSHIP_CONTROL_SERVICE_KEY_V1.as_bytes().len(), 32);
    }

    #[test]
    fn service_is_first_party_and_uses_runtime_keys() {
        let _ = OwnershipControlService;
        assert!(jamscript_runtime_core::control_claim_key(
            &ownership_core::Ownership::from_array(
                ownership_core::OwnershipKind::Ed25519Key,
                [1; 32]
            )
            .unwrap(),
            &ownership_core::Ownership::from_array(
                ownership_core::OwnershipKind::Ed25519Key,
                [2; 32]
            )
            .unwrap(),
        )
        .is_ok());
    }

    #[test]
    fn bootstrap_is_one_time_and_revoke_does_not_reopen_it() {
        let master = SigningKey::from_bytes(&[1; 32]);
        let self_signing = SigningKey::from_bytes(&[2; 32]);
        let device = SigningKey::from_bytes(&[3; 32]);
        let device_two = SigningKey::from_bytes(&[4; 32]);
        let master_owner = owner(&master);
        let device_owner = owner(&device);

        let mut matrix_proof = jamscript_crypto::MatrixControlClaimProofV1 {
            user_id: "@alice:example.org".into(),
            self_signing_public_key: self_signing.verifying_key().to_bytes(),
            master_signature: [0; 64],
            device_id: "DEVICE".into(),
            algorithms: vec!["m.olm.v1.curve25519-aes-sha2".into()],
            device_curve25519_key: [5; 32],
            device_ed25519_key: device.verifying_key().to_bytes(),
            self_signing_signature: [0; 64],
        };
        matrix_proof.master_signature = master
            .sign(&matrix_proof.canonical_self_signing_object().unwrap())
            .to_bytes();
        matrix_proof.self_signing_signature = self_signing
            .sign(&matrix_proof.canonical_device_keys_object().unwrap())
            .to_bytes();
        let mut bootstrap = jamscript_protocol::MatrixControlBootstrapV1 {
            network_domain: [9; 32],
            subject: master_owner.clone(),
            controller: device_owner.clone(),
            matrix_proof: matrix_proof.encode().unwrap(),
            controller_proof: Vec::new(),
        };
        bootstrap.controller_proof = device
            .sign(&bootstrap.controller_signing_message().unwrap())
            .to_bytes()
            .to_vec();
        let mut bad_bootstrap = bootstrap.clone();
        let last = bad_bootstrap.matrix_proof.len() - 1;
        bad_bootstrap.matrix_proof[last] ^= 1;
        let (_, result) = run(
            FullState::empty(),
            &sign_action(
                BOOTSTRAP_MATRIX_CONTROLLER_ACTION,
                &device,
                None,
                0,
                ControlClaimActionV1::BootstrapMatrix {
                    bootstrap: bad_bootstrap,
                }
                .encode()
                .unwrap(),
            ),
        );
        assert_eq!(result, Err(reject(RuntimeError::MatrixInvalidDeviceProof)));
        let bootstrap_payload = ControlClaimActionV1::BootstrapMatrix { bootstrap }
            .encode()
            .unwrap();

        let (state, result) = run(
            FullState::empty(),
            &sign_action(
                BOOTSTRAP_MATRIX_CONTROLLER_ACTION,
                &device,
                None,
                0,
                bootstrap_payload.clone(),
            ),
        );
        assert_eq!(result, Ok(()));
        assert_eq!(
            state
                .get(&control_claim_bootstrap_key(&master_owner).unwrap())
                .unwrap(),
            Some(vec![1])
        );
        assert_eq!(
            state
                .get(&control_claim_key(&master_owner, &device_owner).unwrap())
                .unwrap(),
            Some(vec![1])
        );

        let (state, result) = run(
            state,
            &sign_action(
                ADD_CONTROLLER_ACTION,
                &device,
                Some(master_owner.clone()),
                0,
                ControlClaimActionV1::AddController {
                    subject: master_owner.clone(),
                    controller: owner(&device_two),
                }
                .encode()
                .unwrap(),
            ),
        );
        assert_eq!(result, Ok(()));

        let (state, result) = run(
            state,
            &sign_action(
                REVOKE_CONTROLLER_ACTION,
                &device,
                Some(master_owner.clone()),
                1,
                ControlClaimActionV1::RevokeController {
                    subject: master_owner.clone(),
                    controller: device_owner.clone(),
                }
                .encode()
                .unwrap(),
            ),
        );
        assert_eq!(result, Ok(()));
        assert_eq!(
            state
                .get(&control_claim_key(&master_owner, &device_owner).unwrap())
                .unwrap(),
            Some(vec![0])
        );

        let (_, result) = run(
            state.clone(),
            &sign_action(
                BOOTSTRAP_MATRIX_CONTROLLER_ACTION,
                &device,
                None,
                1,
                bootstrap_payload,
            ),
        );
        assert_eq!(result, Err(reject(RuntimeError::ControlAlreadyInitialized)));

        let (_, result) = run(
            state,
            &sign_action(
                ADD_CONTROLLER_ACTION,
                &device,
                Some(master_owner.clone()),
                2,
                ControlClaimActionV1::AddController {
                    subject: master_owner,
                    controller: owner(&device_two),
                }
                .encode()
                .unwrap(),
            ),
        );
        assert_eq!(result, Err(reject(RuntimeError::ControlClaimNotFound)));
    }
}
