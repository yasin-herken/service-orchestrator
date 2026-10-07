//! Developer profiles.
//!
//! A [`Profile`] is a named, commonly used developer environment, such as
//! `backend-development`, `payment-development`, or `full-stack`. It references
//! services and groups by identifier rather than duplicating their
//! configuration, so profiles compose existing definitions instead of forking
//! them.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::ids::{GroupId, ProfileId, ServiceId};

/// A named developer environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The stable identifier of the profile.
    pub id: ProfileId,
    /// The human-readable name shown in interfaces.
    pub display_name: String,
    /// An optional description.
    pub description: Option<String>,
    /// The services included directly in the profile.
    pub services: Vec<ServiceId>,
    /// The groups included in the profile.
    pub groups: Vec<GroupId>,
}

impl Profile {
    /// Creates an empty profile.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the display name is empty.
    pub fn new(id: ProfileId, display_name: impl Into<String>) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "profile",
                "display name must not be empty",
            ));
        }
        Ok(Self {
            id,
            display_name: display_name.trim().to_owned(),
            description: None,
            services: Vec::new(),
            groups: Vec::new(),
        })
    }

    /// Sets the description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a service directly to the profile.
    #[must_use]
    pub fn with_service(mut self, service: ServiceId) -> Self {
        self.services.push(service);
        self
    }

    /// Adds a group to the profile.
    #[must_use]
    pub fn with_group(mut self, group: GroupId) -> Self {
        self.groups.push(group);
        self
    }

    /// Validates the profile's own invariants.
    ///
    /// This does not check that the referenced services and groups exist; that
    /// requires the surrounding workspace and is enforced by
    /// [`Workspace::validate`](crate::workspace::Workspace::validate).
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the name is empty, the profile
    /// references nothing, or a service or group is listed more than once.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "profile",
                "display name must not be empty",
            ));
        }
        if self.services.is_empty() && self.groups.is_empty() {
            return Err(DomainError::validation(
                "profile",
                format!(
                    "profile '{}' must reference at least one service or group",
                    self.id
                ),
            ));
        }
        let mut seen_services = BTreeSet::new();
        for service in &self.services {
            if !seen_services.insert(service) {
                return Err(DomainError::validation(
                    "profile",
                    format!(
                        "profile '{}' lists service '{service}' more than once",
                        self.id
                    ),
                ));
            }
        }
        let mut seen_groups = BTreeSet::new();
        for group in &self.groups {
            if !seen_groups.insert(group) {
                return Err(DomainError::validation(
                    "profile",
                    format!("profile '{}' lists group '{group}' more than once", self.id),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::new(
            ProfileId::new("backend-development").unwrap(),
            "Backend development",
        )
        .unwrap()
    }

    #[test]
    fn accepts_services_and_groups() {
        let profile = profile()
            .with_service(ServiceId::new("auth-service").unwrap())
            .with_group(GroupId::new("customer").unwrap());
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn rejects_empty_profiles() {
        assert!(profile().validate().is_err());
    }

    #[test]
    fn rejects_duplicate_service_references() {
        let profile = profile()
            .with_service(ServiceId::new("auth-service").unwrap())
            .with_service(ServiceId::new("auth-service").unwrap());
        assert!(profile.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_group_references() {
        let profile = profile()
            .with_group(GroupId::new("customer").unwrap())
            .with_group(GroupId::new("customer").unwrap());
        assert!(profile.validate().is_err());
    }

    #[test]
    fn rejects_empty_display_name() {
        let id = ProfileId::new("full-stack").unwrap();
        assert!(Profile::new(id, " ").is_err());
    }
}
