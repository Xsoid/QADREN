//! Capability negotiation is descriptive, not permission to perform I/O.
//! Adapter-specific checks must revalidate permissions and path identity at execution time.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    Linux,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Operations {
    pub list_metadata: bool,
    pub read_content: bool,
    pub write_content: bool,
    pub streaming: bool,
    pub create_directory: bool,
    pub delete: bool,
    pub rename: bool,
    pub remote_checksum: bool,
    pub change_listing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDeclaration {
    pub supported_platforms: Vec<Platform>,
    pub operations: Operations,
    /// A capability is not operationally available until the adapter validates
    /// current authorization, OS permissions and provider availability.
    pub prerequisites_verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedOperation {
    ListMetadata,
    ReadContent,
    WriteContent,
    CreateDirectory,
    Delete,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityError {
    UnsupportedPlatform,
    MissingPrerequisites,
    UnsupportedOperation,
}

impl CapabilityDeclaration {
    pub fn allows(
        &self,
        platform: Platform,
        operation: RequestedOperation,
    ) -> Result<(), CapabilityError> {
        if !self.supported_platforms.contains(&platform) {
            return Err(CapabilityError::UnsupportedPlatform);
        }
        if !self.prerequisites_verified {
            return Err(CapabilityError::MissingPrerequisites);
        }
        let supported = match operation {
            RequestedOperation::ListMetadata => self.operations.list_metadata,
            RequestedOperation::ReadContent => self.operations.read_content,
            RequestedOperation::WriteContent => self.operations.write_content,
            RequestedOperation::CreateDirectory => self.operations.create_directory,
            RequestedOperation::Delete => self.operations.delete,
            RequestedOperation::Rename => self.operations.rename,
        };
        if supported {
            Ok(())
        } else {
            Err(CapabilityError::UnsupportedOperation)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_only_yandex_cannot_transfer_content() {
        let declaration = CapabilityDeclaration {
            supported_platforms: vec![Platform::MacOS, Platform::Linux, Platform::Windows],
            operations: Operations {
                list_metadata: true,
                ..Operations::default()
            },
            prerequisites_verified: true,
        };
        assert_eq!(
            declaration.allows(Platform::MacOS, RequestedOperation::ListMetadata),
            Ok(())
        );
        assert_eq!(
            declaration.allows(Platform::MacOS, RequestedOperation::ReadContent),
            Err(CapabilityError::UnsupportedOperation)
        );
    }

    #[test]
    fn icloud_does_not_inherit_other_platform_support() {
        let declaration = CapabilityDeclaration {
            supported_platforms: vec![Platform::MacOS],
            operations: Operations {
                list_metadata: true,
                ..Operations::default()
            },
            prerequisites_verified: true,
        };
        assert_eq!(
            declaration.allows(Platform::Linux, RequestedOperation::ListMetadata),
            Err(CapabilityError::UnsupportedPlatform)
        );
    }

    #[test]
    fn declared_capability_without_authorization_cannot_be_used() {
        let declaration = CapabilityDeclaration {
            supported_platforms: vec![Platform::MacOS],
            operations: Operations {
                delete: true,
                ..Operations::default()
            },
            prerequisites_verified: false,
        };
        assert_eq!(
            declaration.allows(Platform::MacOS, RequestedOperation::Delete),
            Err(CapabilityError::MissingPrerequisites)
        );
    }
}
