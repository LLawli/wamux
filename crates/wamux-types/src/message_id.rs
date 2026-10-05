//! A message id (#63, #114), never empty. Opaque: the server assigns it, and
//! the core relays it as it came.

use crate::error::WamuxError;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageId(String);

impl MessageId {
    /// `InvalidArgument("empty message id")`.
    pub fn new(value: impl Into<String>) -> Result<Self, WamuxError> {
        let value: String = value.into();
        if value.is_empty() {
            return Err(WamuxError::InvalidArgument("empty message id".to_string()));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
