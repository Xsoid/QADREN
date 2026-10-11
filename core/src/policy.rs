//! Scoped, fail-closed authorization for potentially destructive operations.
//! This is a pure gate; adapters must still revalidate capability and path
//! identity immediately before I/O.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedOperation {
    Overwrite,
    Delete,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationScope {
    pub operation: ProtectedOperation,
    pub connection_id: String,
    pub relative_path: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accepted,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    MissingDecision,
    Denied,
    Expired,
    ScopeMismatch,
    InvalidScope,
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

/// Prove that an explicit decision authorizes exactly one operation scope at a
/// fixed clock value. It intentionally cannot grant wildcard or future access.
pub fn require_scoped_consent(
    decision: Option<(Decision, &OperationScope)>,
    requested: &OperationScope,
    now: u64,
) -> Result<(), PolicyError> {
    if requested.connection_id.is_empty() || !valid_path(&requested.relative_path) {
        return Err(PolicyError::InvalidScope);
    }
    let Some((decision, granted)) = decision else {
        return Err(PolicyError::MissingDecision);
    };
    if decision == Decision::Denied {
        return Err(PolicyError::Denied);
    }
    if now > granted.expires_at {
        return Err(PolicyError::Expired);
    }
    if granted.operation != requested.operation
        || granted.connection_id != requested.connection_id
        || granted.relative_path != requested.relative_path
    {
        return Err(PolicyError::ScopeMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope(operation: ProtectedOperation, connection: &str, path: &str) -> OperationScope {
        OperationScope {
            operation,
            connection_id: connection.into(),
            relative_path: path.into(),
            expires_at: 10,
        }
    }
    #[test]
    fn exact_accepted_scope_is_required() {
        let requested = scope(ProtectedOperation::Delete, "connection-a", "file.txt");
        assert_eq!(
            require_scoped_consent(Some((Decision::Accepted, &requested)), &requested, 10),
            Ok(())
        );
        assert_eq!(
            require_scoped_consent(None, &requested, 0),
            Err(PolicyError::MissingDecision)
        );
        assert_eq!(
            require_scoped_consent(Some((Decision::Denied, &requested)), &requested, 0),
            Err(PolicyError::Denied)
        );
    }
    #[test]
    fn consent_cannot_cross_connection_path_operation_or_time() {
        let granted = scope(ProtectedOperation::Delete, "connection-a", "file.txt");
        for requested in [
            scope(ProtectedOperation::Delete, "connection-b", "file.txt"),
            scope(ProtectedOperation::Delete, "connection-a", "other.txt"),
            scope(ProtectedOperation::Overwrite, "connection-a", "file.txt"),
        ] {
            assert_eq!(
                require_scoped_consent(Some((Decision::Accepted, &granted)), &requested, 0),
                Err(PolicyError::ScopeMismatch)
            );
        }
        assert_eq!(
            require_scoped_consent(Some((Decision::Accepted, &granted)), &granted, 11),
            Err(PolicyError::Expired)
        );
    }
}
