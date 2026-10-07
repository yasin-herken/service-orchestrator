//! Network port value object.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// A TCP/UDP port number.
///
/// Ports are modelled as a validated value object rather than a bare `u16` so
/// that the invalid value `0` can be rejected once, at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Port(u16);

impl Port {
    /// The smallest assignable port number.
    pub const MIN: u16 = 1;
    /// The largest valid port number.
    pub const MAX: u16 = u16::MAX;

    /// Validates `value` and constructs a port.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if `value` is `0`, which is not a
    /// usable port.
    pub fn new(value: u16) -> Result<Self, DomainError> {
        if value == 0 {
            return Err(DomainError::validation(
                "port",
                "port 0 is not a usable port",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the raw port number.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for Port {
    type Error = DomainError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Port> for u16 {
    fn from(value: Port) -> Self {
        value.0
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_ports() {
        assert_eq!(Port::new(8081).unwrap().value(), 8081);
        assert_eq!(Port::new(Port::MAX).unwrap().value(), u16::MAX);
    }

    #[test]
    fn rejects_port_zero() {
        assert!(Port::new(0).is_err());
    }

    #[test]
    fn converts_to_raw_value() {
        let port = Port::new(3000).unwrap();
        assert_eq!(u16::from(port), 3000);
        assert_eq!(port.to_string(), "3000");
    }
}
