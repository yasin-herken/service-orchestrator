//! Service groups.
//!
//! A [`ServiceGroup`] is a named, logical collection of services. It references
//! services by identifier and never duplicates their definitions. Group actions
//! reuse the same application-level workflows as individual service actions.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::ids::{GroupId, ServiceId};

/// A named collection of services.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceGroup {
    /// The stable identifier of the group.
    pub id: GroupId,
    /// The human-readable name shown in interfaces.
    pub display_name: String,
    /// An optional description.
    pub description: Option<String>,
    /// The services that belong to the group.
    pub services: Vec<ServiceId>,
}

impl ServiceGroup {
    /// Creates an empty group.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the display name is empty.
    pub fn new(id: GroupId, display_name: impl Into<String>) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "service group",
                "display name must not be empty",
            ));
        }
        Ok(Self {
            id,
            display_name: display_name.trim().to_owned(),
            description: None,
            services: Vec::new(),
        })
    }

    /// Sets the description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a service to the group.
    #[must_use]
    pub fn with_service(mut self, service: ServiceId) -> Self {
        self.services.push(service);
        self
    }

    /// Validates the group's invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the name is empty, the group has
    /// no services, or a service is listed more than once.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "service group",
                "display name must not be empty",
            ));
        }
        if self.services.is_empty() {
            return Err(DomainError::validation(
                "service group",
                format!("group '{}' must contain at least one service", self.id),
            ));
        }
        let mut seen = BTreeSet::new();
        for service in &self.services {
            if !seen.insert(service) {
                return Err(DomainError::validation(
                    "service group",
                    format!(
                        "group '{}' lists service '{service}' more than once",
                        self.id
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> ServiceGroup {
        ServiceGroup::new(GroupId::new("customer").unwrap(), "Customer").unwrap()
    }

    #[test]
    fn accepts_a_valid_group() {
        let group = group()
            .with_service(ServiceId::new("auth-service").unwrap())
            .with_service(ServiceId::new("user-service").unwrap());
        assert!(group.validate().is_ok());
    }

    #[test]
    fn rejects_empty_groups() {
        assert!(group().validate().is_err());
    }

    #[test]
    fn rejects_duplicate_services() {
        let group = group()
            .with_service(ServiceId::new("auth-service").unwrap())
            .with_service(ServiceId::new("auth-service").unwrap());
        assert!(group.validate().is_err());
    }

    #[test]
    fn rejects_empty_display_name() {
        let id = GroupId::new("customer").unwrap();
        assert!(ServiceGroup::new(id, "  ").is_err());
    }
}
