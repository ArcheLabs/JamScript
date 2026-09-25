#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use alloc::{format, string::String};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use jamscript_crypto::verify_ed25519;
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixProofError {
    #[error("invalid Matrix bootstrap proof")]
    InvalidMatrixProof,
    #[error("Matrix bootstrap proof verification failed")]
    MatrixProofVerificationFailed,
}

pub const MAX_MATRIX_TEXT_BYTES: usize = 255;
pub const MAX_MATRIX_ALGORITHMS: usize = 8;
pub const MAX_MATRIX_ALGORITHM_BYTES: usize = 128;

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
    pub fn decode(bytes: &[u8]) -> Result<Self, MatrixProofError> {
        let mut reader = MatrixReader { bytes, offset: 0 };
        if reader.u8()? != 1 {
            return Err(MatrixProofError::InvalidMatrixProof);
        }
        let user_id = reader.text()?;
        let self_signing_public_key = reader.array::<32>()?;
        let master_signature = reader.array::<64>()?;
        let device_id = reader.text()?;
        let count = reader.u8()? as usize;
        if count > MAX_MATRIX_ALGORITHMS {
            return Err(MatrixProofError::InvalidMatrixProof);
        }
        let mut algorithms = Vec::with_capacity(count);
        for _ in 0..count {
            algorithms.push(reader.text_bounded(MAX_MATRIX_ALGORITHM_BYTES)?);
        }
        let device_curve25519_key = reader.array::<32>()?;
        let device_ed25519_key = reader.array::<32>()?;
        let self_signing_signature = reader.array::<64>()?;
        if reader.offset != bytes.len() {
            return Err(MatrixProofError::InvalidMatrixProof);
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

    pub fn encode(&self) -> Result<Vec<u8>, MatrixProofError> {
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
            return Err(MatrixProofError::InvalidMatrixProof);
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

    pub fn verify(&self, master_public_key: &[u8; 32]) -> Result<(), MatrixProofError> {
        let self_object = self.canonical_self_signing_object()?;
        verify_ed25519(master_public_key, &self.master_signature, &self_object)
            .map_err(|_| MatrixProofError::MatrixProofVerificationFailed)?;
        let device_object = self.canonical_device_keys_object()?;
        verify_ed25519(
            &self.self_signing_public_key,
            &self.self_signing_signature,
            &device_object,
        )
        .map_err(|_| MatrixProofError::MatrixProofVerificationFailed)?;
        Ok(())
    }

    pub fn verify_for(
        &self,
        master_public_key: &[u8; 32],
        expected_device_public_key: &[u8; 32],
    ) -> Result<(), MatrixProofError> {
        self.verify(master_public_key)?;
        if &self.device_ed25519_key != expected_device_public_key {
            return Err(MatrixProofError::MatrixProofVerificationFailed);
        }
        Ok(())
    }

    pub fn canonical_self_signing_object(&self) -> Result<Vec<u8>, MatrixProofError> {
        let key = STANDARD_NO_PAD.encode(self.self_signing_public_key);
        Ok(format_matrix_self_object(&self.user_id, &key))
    }

    pub fn canonical_device_keys_object(&self) -> Result<Vec<u8>, MatrixProofError> {
        let curve = STANDARD_NO_PAD.encode(self.device_curve25519_key);
        let ed = STANDARD_NO_PAD.encode(self.device_ed25519_key);
        if !self.user_id.is_ascii()
            || !self.device_id.is_ascii()
            || self.algorithms.iter().any(|value| !value.is_ascii())
        {
            return Err(MatrixProofError::InvalidMatrixProof);
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

fn put_text(output: &mut Vec<u8>, value: &str) -> Result<(), MatrixProofError> {
    put_text_bounded(output, value, MAX_MATRIX_TEXT_BYTES)
}

fn put_text_bounded(output: &mut Vec<u8>, value: &str, max: usize) -> Result<(), MatrixProofError> {
    if value.len() > max
        || value.len() > u16::MAX as usize
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte < 0x20 || byte == b'"' || byte == b'\\')
    {
        return Err(MatrixProofError::InvalidMatrixProof);
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
    fn take(&mut self, length: usize) -> Result<&'a [u8], MatrixProofError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(MatrixProofError::InvalidMatrixProof)?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or(MatrixProofError::InvalidMatrixProof)?;
        self.offset = end;
        Ok(result)
    }
    fn u8(&mut self) -> Result<u8, MatrixProofError> {
        Ok(self.take(1)?[0])
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], MatrixProofError> {
        self.take(N)?
            .try_into()
            .map_err(|_| MatrixProofError::InvalidMatrixProof)
    }
    fn text(&mut self) -> Result<String, MatrixProofError> {
        self.text_bounded(MAX_MATRIX_TEXT_BYTES)
    }
    fn text_bounded(&mut self, max: usize) -> Result<String, MatrixProofError> {
        let length = u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| MatrixProofError::InvalidMatrixProof)?,
        ) as usize;
        if length > max {
            return Err(MatrixProofError::InvalidMatrixProof);
        }
        let bytes = self.take(length)?;
        if !bytes.is_ascii() {
            return Err(MatrixProofError::InvalidMatrixProof);
        }
        if bytes
            .iter()
            .any(|byte| *byte < 0x20 || *byte == b'"' || *byte == b'\\')
        {
            return Err(MatrixProofError::InvalidMatrixProof);
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| MatrixProofError::InvalidMatrixProof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::String, vec};
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn matrix_proof_is_bounded_wire_compatible_and_verifies_both_signatures() {
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
        assert_eq!(decoded.encode().unwrap(), encoded);
        let mut bad_device_signature = decoded.clone();
        bad_device_signature.device_id = "OTHER".into();
        assert_eq!(
            bad_device_signature.verify(&master.verifying_key().to_bytes()),
            Err(MatrixProofError::MatrixProofVerificationFailed)
        );

        let mut bad_master_signature = encoded.clone();
        let master_signature_offset = 1 + 2 + "@alice:example.org".len() + 32;
        bad_master_signature[master_signature_offset] ^= 1;
        assert!(!verify_matrix_cross_signing(
            &master.verifying_key().to_bytes(),
            &device.verifying_key().to_bytes(),
            &bad_master_signature,
        ));

        let mut bad_device_signature = encoded.clone();
        let device_signature_offset = bad_device_signature.len() - 64;
        bad_device_signature[device_signature_offset] ^= 1;
        assert!(!verify_matrix_cross_signing(
            &master.verifying_key().to_bytes(),
            &device.verifying_key().to_bytes(),
            &bad_device_signature,
        ));
        assert!(!verify_matrix_cross_signing(
            &[8; 32],
            &device.verifying_key().to_bytes(),
            &encoded,
        ));
    }

    #[test]
    fn rejects_malformed_unknown_and_oversized_proofs() {
        assert_eq!(
            MatrixControlClaimProofV1::decode(&[]),
            Err(MatrixProofError::InvalidMatrixProof)
        );
        assert_eq!(
            MatrixControlClaimProofV1::decode(&[2]),
            Err(MatrixProofError::InvalidMatrixProof)
        );
        assert!(!verify_matrix_cross_signing(&[1; 31], &[2; 32], &[1]));
        assert!(!verify_matrix_cross_signing(&[1; 32], &[2; 31], &[1]));
        assert!(!verify_matrix_cross_signing(&[1; 32], &[2; 32], &[1]));
        let mut truncated = vec![1, 0, 0];
        assert_eq!(
            MatrixControlClaimProofV1::decode(&truncated),
            Err(MatrixProofError::InvalidMatrixProof)
        );
        truncated.push(0);
        assert!(!verify_matrix_cross_signing(&[1; 32], &[2; 32], &truncated));
    }
}
