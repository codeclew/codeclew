use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::protocol::{CapabilityDescriptor, CapabilityKey};
use crate::{Validate, ValidationErrors};

/// An exact capability lookup. Evidence grades are category labels and are
/// never upgraded, downgraded, or ranked by the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityQuery {
    pub key: CapabilityKey,
}

impl From<CapabilityKey> for CapabilityQuery {
    fn from(key: CapabilityKey) -> Self {
        Self { key }
    }
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error(transparent)]
    Invalid(#[from] ValidationErrors),
    #[error("descriptor digest {digest} is already registered with different content")]
    DigestCollision { digest: String },
    #[error("capability key is already registered by {existing_digest}")]
    DuplicateKey { existing_digest: String },
    #[error("descriptor {descriptor_digest} requires unregistered capability {required_digest}")]
    MissingDependency {
        descriptor_digest: String,
        required_digest: String,
    },
    #[error("capability dependency graph contains a cycle at {0}")]
    DependencyCycle(String),
}

#[derive(Clone, Debug, Default)]
pub struct ContractRegistry {
    descriptors: BTreeMap<String, CapabilityDescriptor>,
}

impl ContractRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, descriptor: CapabilityDescriptor) -> Result<String, RegistryError> {
        descriptor.validate()?;
        let digest = descriptor.content_digest.clone();
        if let Some(existing) = self.descriptors.get(&digest) {
            if existing == &descriptor {
                return Ok(digest);
            }
            return Err(RegistryError::DigestCollision { digest });
        }
        if let Some(existing) = self
            .descriptors
            .values()
            .find(|existing| existing.key == descriptor.key)
        {
            return Err(RegistryError::DuplicateKey {
                existing_digest: existing.content_digest.clone(),
            });
        }
        self.descriptors.insert(digest.clone(), descriptor);
        Ok(digest)
    }

    pub fn descriptor(&self, digest: &str) -> Option<&CapabilityDescriptor> {
        self.descriptors.get(digest)
    }

    pub fn find_exact(&self, query: &CapabilityQuery) -> Option<&CapabilityDescriptor> {
        self.descriptors
            .values()
            .find(|descriptor| descriptor.key.as_ref() == Some(&query.key))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &CapabilityDescriptor)> {
        self.descriptors
            .iter()
            .map(|(digest, descriptor)| (digest.as_str(), descriptor))
    }

    pub fn validate_closure(&self) -> Result<(), RegistryError> {
        for (digest, descriptor) in &self.descriptors {
            for required in &descriptor.required_capability_digests {
                if !self.descriptors.contains_key(required) {
                    return Err(RegistryError::MissingDependency {
                        descriptor_digest: digest.clone(),
                        required_digest: required.clone(),
                    });
                }
            }
        }
        let mut permanent = BTreeSet::new();
        let mut temporary = BTreeSet::new();
        for digest in self.descriptors.keys() {
            if self.has_cycle(digest, &mut permanent, &mut temporary) {
                return Err(RegistryError::DependencyCycle(digest.clone()));
            }
        }
        Ok(())
    }

    fn has_cycle<'a>(
        &'a self,
        digest: &'a str,
        permanent: &mut BTreeSet<&'a str>,
        temporary: &mut BTreeSet<&'a str>,
    ) -> bool {
        if permanent.contains(digest) {
            return false;
        }
        if !temporary.insert(digest) {
            return true;
        }
        if self
            .descriptors
            .get(digest)
            .into_iter()
            .flat_map(|descriptor| descriptor.required_capability_digests.iter())
            .any(|required| self.has_cycle(required, permanent, temporary))
        {
            return true;
        }
        temporary.remove(digest);
        permanent.insert(digest);
        false
    }
}
