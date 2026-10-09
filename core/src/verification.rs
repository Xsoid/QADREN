//! Verification receipts are constructed only by an adapter-side verifier.
//! This prototype validates the shape; actual provider proof is not yet implemented.
use crate::confirmation::{Confirmation, EvidenceLevel};

pub struct VerifiedReceipt {
    attempt_id: String,
    confirmation: Confirmation,
}

impl VerifiedReceipt {
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
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
pub(crate) fn test_receipt(attempt_id: &str) -> VerifiedReceipt {
    VerifiedReceipt {
        attempt_id: attempt_id.to_owned(),
        confirmation: Confirmation::Confirmed(EvidenceLevel::MetadataVerified),
    }
}
