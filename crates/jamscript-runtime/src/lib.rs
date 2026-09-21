use jamscript_crypto::Address;
use jamscript_protocol::{ProtocolError, VerifiedAction};
use ownership_core::{Ownership, OwnershipKey};
use service_runtime_core::application_key_v1;
use std::collections::BTreeMap;

pub const STATE_KEY_DOMAIN_V1: &[u8] = b"jamscript/state/v1";
pub const RUNTIME_NONCE_NAMESPACE_V1: &[u8] = b"__jamscript/runtime/auth/nonces/";
pub const CONTROL_CLAIM_NAMESPACE_V1: &[u8] = b"__jamscript/runtime/ownership/control/";

pub type StateKey = Vec<u8>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlClaim {
    pub subject: OwnershipKey,
    pub controller: OwnershipKey,
    pub version: u8,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlClaimError {
    InvalidOwnership,
    InvalidBootstrap,
    Unauthorized,
    AlreadyExists,
    NotFound,
    AlreadyRevoked,
    AlreadyInitialized,
}

#[derive(Clone, Debug, Default)]
pub struct ControlClaimState {
    claims: BTreeMap<(OwnershipKey, OwnershipKey), ControlClaim>,
    initialized: BTreeMap<OwnershipKey, bool>,
}

impl ControlClaimState {
    pub fn is_initialized(&self, subject: &Ownership) -> bool {
        subject
            .key()
            .ok()
            .and_then(|key| self.initialized.get(&key).copied())
            .unwrap_or(false)
    }

    pub fn bootstrap_controller(
        &mut self,
        subject: &Ownership,
        controller: &Ownership,
        effective_owner: &Ownership,
    ) -> Result<(), ControlClaimError> {
        let subject_key = subject
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        if self.initialized.get(&subject_key).copied().unwrap_or(false) {
            return Err(ControlClaimError::AlreadyInitialized);
        }
        self.add_controller(subject, controller, effective_owner)?;
        self.initialized.insert(subject_key, true);
        Ok(())
    }

    /// Validate and apply the one-time Matrix bootstrap proof. `initialized`
    /// is deliberately retained after revoke, so an old public M→S→D proof
    /// cannot bootstrap the subject again.
    pub fn bootstrap_matrix_controller(
        &mut self,
        bootstrap: &jamscript_protocol::MatrixControlBootstrapV1,
        network_domain: [u8; 32],
    ) -> Result<(), ControlClaimError> {
        bootstrap
            .verify(network_domain)
            .map_err(|_| ControlClaimError::InvalidBootstrap)?;
        self.bootstrap_controller(&bootstrap.subject, &bootstrap.controller, &bootstrap.subject)
    }

    pub fn is_active(&self, subject: &Ownership, controller: &Ownership) -> bool {
        let Ok(subject) = subject.key() else {
            return false;
        };
        let Ok(controller) = controller.key() else {
            return false;
        };
        self.claims
            .get(&(subject, controller))
            .is_some_and(|claim| claim.active)
    }

    pub fn add_controller(
        &mut self,
        subject: &Ownership,
        controller: &Ownership,
        effective_owner: &Ownership,
    ) -> Result<(), ControlClaimError> {
        let subject_key = subject
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        let controller_key = controller
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        let effective_key = effective_owner
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        if effective_key != subject_key {
            return Err(ControlClaimError::Unauthorized);
        }
        if self.claims.contains_key(&(subject_key, controller_key)) {
            return Err(ControlClaimError::AlreadyExists);
        }
        self.claims.insert(
            (subject_key, controller_key),
            ControlClaim {
                subject: subject_key,
                controller: controller_key,
                version: 1,
                active: true,
            },
        );
        Ok(())
    }

    pub fn revoke_controller(
        &mut self,
        subject: &Ownership,
        controller: &Ownership,
        effective_owner: &Ownership,
    ) -> Result<(), ControlClaimError> {
        let subject_key = subject
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        let controller_key = controller
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        let effective_key = effective_owner
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        if effective_key != subject_key {
            return Err(ControlClaimError::Unauthorized);
        }
        let claim = self
            .claims
            .get_mut(&(subject_key, controller_key))
            .ok_or(ControlClaimError::NotFound)?;
        if !claim.active {
            return Err(ControlClaimError::AlreadyRevoked);
        }
        claim.active = false;
        Ok(())
    }

    pub fn claims(&self) -> impl Iterator<Item = &ControlClaim> {
        self.claims.values()
    }
}

pub fn state_key(_service_id: u32, schema_id: &[u8], canonical_user_key: &[u8]) -> StateKey {
    application_key_v1(schema_id, canonical_user_key)
        .expect("managed application key length must fit u16")
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryRuntime {
    service_id: u32,
    state: BTreeMap<StateKey, Vec<u8>>,
    next_nonce: BTreeMap<Address, u64>,
    ownership_nonces: BTreeMap<OwnershipKey, u64>,
    control_claims: ControlClaimState,
}

impl InMemoryRuntime {
    pub fn new(service_id: u32) -> Self {
        Self {
            service_id,
            state: BTreeMap::new(),
            next_nonce: BTreeMap::new(),
            ownership_nonces: BTreeMap::new(),
            control_claims: ControlClaimState::default(),
        }
    }

    pub fn service_id(&self) -> u32 {
        self.service_id
    }

    pub fn next_nonce(&self, sender: &Address) -> u64 {
        self.next_nonce.get(sender).copied().unwrap_or(0)
    }

    pub fn next_ownership_nonce(&self, owner: &Ownership) -> Result<u64, ControlClaimError> {
        let key = owner
            .key()
            .map_err(|_| ControlClaimError::InvalidOwnership)?;
        Ok(self.ownership_nonces.get(&key).copied().unwrap_or(0))
    }

    pub fn control_claims(&self) -> &ControlClaimState {
        &self.control_claims
    }

    pub fn control_claims_mut(&mut self) -> &mut ControlClaimState {
        &mut self.control_claims
    }

    pub fn read(&self, schema_id: &[u8], key: &[u8]) -> Option<&[u8]> {
        self.state
            .get(&state_key(self.service_id, schema_id, key))
            .map(Vec::as_slice)
    }

    pub fn apply_action<F>(&mut self, action: &VerifiedAction, commit: F) -> ActionReceipt
    where
        F: FnOnce(&mut StateTransaction<'_>, &VerifiedAction) -> Result<(), u32>,
    {
        let expected = self.next_nonce(&action.sender);
        if action.nonce != expected {
            return ActionReceipt::rejected(
                action,
                ProtocolError::NonceMismatch {
                    expected,
                    actual: action.nonce,
                }
                .code(),
            );
        }
        if expected == u64::MAX {
            return ActionReceipt::rejected(action, RuntimeError::NonceExhausted.code());
        }

        let mut transaction = StateTransaction {
            service_id: self.service_id,
            base: &self.state,
            writes: BTreeMap::new(),
        };
        let result = commit(&mut transaction, action);
        // v0.1 consumes the valid nonce even when user commit fails.
        self.next_nonce.insert(action.sender, expected + 1);
        match result {
            Ok(()) => {
                for (key, value) in transaction.into_writes() {
                    match value {
                        Some(value) => {
                            self.state.insert(key, value);
                        }
                        None => {
                            self.state.remove(&key);
                        }
                    }
                }
                ActionReceipt::applied(action)
            }
            Err(error_code) => ActionReceipt::failed(action, error_code),
        }
    }

    pub fn apply_ownership_action<F>(
        &mut self,
        action: &jamscript_protocol::VerifiedOwnershipAction,
        commit: F,
    ) -> OwnershipActionReceipt
    where
        F: FnOnce(
            &mut StateTransaction<'_>,
            &jamscript_protocol::VerifiedOwnershipAction,
        ) -> Result<(), u32>,
    {
        let owner_key = match action.owner.key() {
            Ok(key) => key,
            Err(_) => return OwnershipActionReceipt::rejected(action, 15),
        };
        let expected = self.ownership_nonces.get(&owner_key).copied().unwrap_or(0);
        if action.nonce != expected {
            return OwnershipActionReceipt::rejected(action, 17);
        }
        if expected == u64::MAX {
            return OwnershipActionReceipt::rejected(action, RuntimeError::NonceExhausted.code());
        }
        let mut transaction = StateTransaction {
            service_id: self.service_id,
            base: &self.state,
            writes: BTreeMap::new(),
        };
        let result = commit(&mut transaction, action);
        self.ownership_nonces.insert(owner_key, expected + 1);
        match result {
            Ok(()) => {
                for (key, value) in transaction.into_writes() {
                    match value {
                        Some(value) => {
                            self.state.insert(key, value);
                        }
                        None => {
                            self.state.remove(&key);
                        }
                    }
                }
                OwnershipActionReceipt::applied(action)
            }
            Err(code) => OwnershipActionReceipt::failed(action, code),
        }
    }

    pub fn apply_batch<I, F>(&mut self, actions: I, mut commit: F) -> Vec<ActionReceipt>
    where
        I: IntoIterator<Item = Result<VerifiedAction, ProtocolError>>,
        F: FnMut(&mut StateTransaction<'_>, &VerifiedAction) -> Result<(), u32>,
    {
        actions
            .into_iter()
            .map(|action| match action {
                Ok(action) => self.apply_action(&action, &mut commit),
                Err(error) => ActionReceipt::invalid(error.code()),
            })
            .collect()
    }
}

pub struct StateTransaction<'a> {
    service_id: u32,
    base: &'a BTreeMap<StateKey, Vec<u8>>,
    writes: BTreeMap<StateKey, Option<Vec<u8>>>,
}

impl StateTransaction<'_> {
    pub fn get(&self, schema_id: &[u8], key: &[u8]) -> Option<&[u8]> {
        let key = state_key(self.service_id, schema_id, key);
        match self.writes.get(&key) {
            Some(Some(value)) => Some(value.as_slice()),
            Some(None) => None,
            None => self.base.get(&key).map(Vec::as_slice),
        }
    }

    pub fn set(&mut self, schema_id: &[u8], key: &[u8], value: Vec<u8>) {
        self.writes
            .insert(state_key(self.service_id, schema_id, key), Some(value));
    }

    pub fn delete(&mut self, schema_id: &[u8], key: &[u8]) {
        self.writes
            .insert(state_key(self.service_id, schema_id, key), None);
    }

    fn into_writes(self) -> BTreeMap<StateKey, Option<Vec<u8>>> {
        self.writes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionStatus {
    Applied,
    Failed,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionReceipt {
    pub action_hash: Option<[u8; 32]>,
    pub sender: Option<Address>,
    pub nonce: Option<u64>,
    pub status: ActionStatus,
    pub error_code: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnershipActionReceipt {
    pub action_hash: [u8; 32],
    pub owner: Ownership,
    pub controller: Ownership,
    pub nonce: u64,
    pub status: ActionStatus,
    pub error_code: Option<u32>,
}

impl OwnershipActionReceipt {
    fn applied(action: &jamscript_protocol::VerifiedOwnershipAction) -> Self {
        Self {
            action_hash: action.action_hash,
            owner: action.owner.clone(),
            controller: action.controller.clone(),
            nonce: action.nonce,
            status: ActionStatus::Applied,
            error_code: None,
        }
    }
    fn failed(action: &jamscript_protocol::VerifiedOwnershipAction, error_code: u32) -> Self {
        Self {
            action_hash: action.action_hash,
            owner: action.owner.clone(),
            controller: action.controller.clone(),
            nonce: action.nonce,
            status: ActionStatus::Failed,
            error_code: Some(error_code),
        }
    }
    fn rejected(action: &jamscript_protocol::VerifiedOwnershipAction, error_code: u32) -> Self {
        Self {
            action_hash: action.action_hash,
            owner: action.owner.clone(),
            controller: action.controller.clone(),
            nonce: action.nonce,
            status: ActionStatus::Rejected,
            error_code: Some(error_code),
        }
    }
}

impl ActionReceipt {
    fn applied(action: &VerifiedAction) -> Self {
        Self {
            action_hash: Some(action.action_hash),
            sender: Some(action.sender),
            nonce: Some(action.nonce),
            status: ActionStatus::Applied,
            error_code: None,
        }
    }
    fn failed(action: &VerifiedAction, error_code: u32) -> Self {
        Self {
            action_hash: Some(action.action_hash),
            sender: Some(action.sender),
            nonce: Some(action.nonce),
            status: ActionStatus::Failed,
            error_code: Some(error_code),
        }
    }
    fn rejected(action: &VerifiedAction, error_code: u32) -> Self {
        Self {
            action_hash: Some(action.action_hash),
            sender: Some(action.sender),
            nonce: Some(action.nonce),
            status: ActionStatus::Rejected,
            error_code: Some(error_code),
        }
    }
    fn invalid(error_code: u32) -> Self {
        Self {
            action_hash: None,
            sender: None,
            nonce: None,
            status: ActionStatus::Rejected,
            error_code: Some(error_code),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    NonceExhausted,
}

impl RuntimeError {
    pub fn code(self) -> u32 {
        match self {
            Self::NonceExhausted => 12,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jamscript_protocol::ProtocolError;

    fn action(sender: u8, nonce: u64, hash: u8) -> VerifiedAction {
        VerifiedAction {
            sender: [sender; 32],
            action_hash: [hash; 32],
            nonce,
            payload: vec![hash],
        }
    }

    #[test]
    fn action_writes_are_transactional_and_failed_commit_rolls_back() {
        let mut runtime = InMemoryRuntime::new(182);
        let first = action(1, 0, 1);
        let receipt = runtime.apply_action(&first, |tx, _| {
            tx.set(b"scores", b"alice", b"10".to_vec());
            Ok(())
        });
        assert_eq!(receipt.status, ActionStatus::Applied);
        assert_eq!(runtime.read(b"scores", b"alice"), Some(&b"10"[..]));
        let second = action(1, 1, 2);
        let receipt = runtime.apply_action(&second, |tx, _| {
            tx.set(b"scores", b"alice", b"20".to_vec());
            Err(77)
        });
        assert_eq!(receipt.status, ActionStatus::Failed);
        assert_eq!(runtime.read(b"scores", b"alice"), Some(&b"10"[..]));
        assert_eq!(runtime.next_nonce(&[1; 32]), 2);
    }

    #[test]
    fn ten_users_apply_in_one_batch_and_bad_action_does_not_poison_them() {
        let mut runtime = InMemoryRuntime::new(182);
        let actions = (0..10)
            .map(|sender| {
                if sender == 5 {
                    Err(ProtocolError::PayloadHashMismatch)
                } else {
                    Ok(action(sender, 0, sender))
                }
            })
            .collect::<Vec<_>>();
        let receipts = runtime.apply_batch(actions, |tx, action| {
            tx.set(b"scores", &action.sender, action.payload.clone());
            Ok(())
        });
        assert_eq!(
            receipts
                .iter()
                .filter(|r| r.status == ActionStatus::Applied)
                .count(),
            9
        );
        assert_eq!(
            receipts
                .iter()
                .filter(|r| r.status == ActionStatus::Rejected)
                .count(),
            1
        );
        for sender in 0..10 {
            if sender == 5 {
                assert_eq!(runtime.read(b"scores", &[sender; 32]), None);
            } else {
                assert_eq!(runtime.read(b"scores", &[sender; 32]), Some(&[sender][..]));
            }
        }
    }

    #[test]
    fn sequential_nonce_and_gap_are_deterministic() {
        let mut runtime = InMemoryRuntime::new(182);
        let first = action(3, 0, 1);
        let second = action(3, 1, 2);
        let gap = action(3, 3, 3);
        assert_eq!(
            runtime
                .apply_batch(vec![Ok(first), Ok(second)], |_, _| Ok(()))
                .iter()
                .filter(|r| r.status == ActionStatus::Applied)
                .count(),
            2
        );
        let receipt = runtime.apply_action(&gap, |_, _| Ok(()));
        assert_eq!(receipt.status, ActionStatus::Rejected);
        assert_eq!(
            receipt.error_code,
            Some(
                ProtocolError::NonceMismatch {
                    expected: 2,
                    actual: 3
                }
                .code()
            )
        );
    }

    #[test]
    fn state_keys_are_schema_scoped_inside_a_service_trie() {
        assert_eq!(
            state_key(1, b"scores", b"alice"),
            state_key(2, b"scores", b"alice")
        );
        assert_ne!(
            state_key(1, b"scores", b"alice"),
            state_key(1, b"notes", b"alice")
        );
        assert_ne!(
            state_key(1, RUNTIME_NONCE_NAMESPACE_V1, b"alice"),
            state_key(1, b"scores", b"alice")
        );
        assert_ne!(state_key(1, b"a", b"bc"), state_key(1, b"ab", b"c"));
    }

    #[test]
    fn control_claims_are_explicit_revocable_and_non_transitive() {
        let subject =
            Ownership::from_array(ownership_core::OwnershipKind::Ed25519Key, [1; 32]).unwrap();
        let first =
            Ownership::from_array(ownership_core::OwnershipKind::Ed25519Key, [2; 32]).unwrap();
        let second =
            Ownership::from_array(ownership_core::OwnershipKind::Ed25519Key, [3; 32]).unwrap();
        let mut claims = ControlClaimState::default();
        claims
            .bootstrap_controller(&subject, &first, &subject)
            .unwrap();
        assert!(claims.is_initialized(&subject));
        assert_eq!(
            claims.bootstrap_controller(&subject, &second, &subject),
            Err(ControlClaimError::AlreadyInitialized)
        );
        assert!(claims.is_active(&subject, &first));
        assert!(!claims.is_active(&subject, &second));
        assert_eq!(
            claims.add_controller(&subject, &second, &first),
            Err(ControlClaimError::Unauthorized)
        );
        claims.add_controller(&subject, &second, &subject).unwrap();
        claims
            .revoke_controller(&subject, &first, &subject)
            .unwrap();
        assert!(!claims.is_active(&subject, &first));
        assert!(claims.is_active(&subject, &second));
    }
}
