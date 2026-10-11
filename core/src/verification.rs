//! Verification receipts are constructed only by an adapter-side verifier.
//! This prototype validates the shape; actual provider proof is not yet implemented.
use crate::confirmation::Confirmation;

pub struct VerifiedReceipt {
    attempt_id: String,
    confirmation: Confirmation,
    operation_kind: Option<String>,
    target_connection_id: Option<String>,
    target_path: Option<String>,
}

impl VerifiedReceipt {
    /// Constructed only by an adapter verifier after it has independently
    /// observed the exact durable operation target.
    pub(crate) fn metadata_verified(
        attempt_id: &str,
        operation_kind: &str,
        target_connection_id: &str,
        target_path: &str,
    ) -> Self {
        Self {
            attempt_id: attempt_id.to_owned(),
            confirmation: Confirmation::Confirmed(
                crate::confirmation::EvidenceLevel::MetadataVerified,
            ),
            operation_kind: Some(operation_kind.to_owned()),
            target_connection_id: Some(target_connection_id.to_owned()),
            target_path: Some(target_path.to_owned()),
        }
    }

    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    pub fn operation_kind(&self) -> Option<&str> {
        self.operation_kind.as_deref()
    }

    pub fn target_connection_id(&self) -> Option<&str> {
        self.target_connection_id.as_deref()
    }

    pub fn target_path(&self) -> Option<&str> {
        self.target_path.as_deref()
    }

    pub fn confirmation(&self) -> Confirmation {
        self.confirmation
    }
}

/// Adapter implementers must validate the remote target before issuing a receipt.
/// This trait is deliberately NOT implemented by the current bootstrap adapters.
pub trait OperationVerifier {
    fn verify(&self, attempt_id: &str) -> Option<VerifiedReceipt>;
}

#[cfg(test)]
pub(crate) fn test_bound_receipt(
    attempt_id: &str,
    kind: &str,
    connection: &str,
    path: &str,
) -> VerifiedReceipt {
    VerifiedReceipt::metadata_verified(attempt_id, kind, connection, path)
}
