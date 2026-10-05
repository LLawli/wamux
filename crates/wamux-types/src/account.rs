//! Account identifiers (#63, #114): the canonical UUID, the edge's optional
//! external reference, and the reference a request names an account by.

use uuid::Uuid;
use wamux_proto::v1 as pb;

use crate::error::WamuxError;

/// An account's canonical UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccountId(Uuid);

/// The edge's own name for an account. Not validated: an empty one is looked
/// up like any other and answers NotFound, as it did before this type existed
/// (#114 changes nothing on the wire).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalRef(String);

/// How a request names an account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountRef {
    Id(AccountId),
    External(ExternalRef),
}

impl AccountId {
    /// `InvalidArgument("bad uuid '<v>'")`, as `resolve` answers today.
    pub fn parse(value: &str) -> Result<Self, WamuxError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| WamuxError::InvalidArgument(format!("bad uuid '{value}'")))
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl From<Uuid> for AccountId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl ExternalRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExternalRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AccountRef {
    /// The `account` field of a request. Absent, or present with no `ref` set,
    /// is `InvalidArgument("missing account ref")`, as `resolve` answers today.
    pub fn from_proto(account: Option<&pb::AccountRef>) -> Result<Self, WamuxError> {
        match account.and_then(|a| a.r#ref.as_ref()) {
            Some(pb::account_ref::Ref::Uuid(uuid)) => AccountId::parse(uuid).map(Self::Id),
            Some(pb::account_ref::Ref::ExternalRef(external)) => {
                Ok(Self::External(ExternalRef::new(external.as_str())))
            }
            None => Err(WamuxError::InvalidArgument(
                "missing account ref".to_string(),
            )),
        }
    }
}

impl TryFrom<pb::AccountRef> for AccountRef {
    type Error = WamuxError;

    fn try_from(account: pb::AccountRef) -> Result<Self, WamuxError> {
        Self::from_proto(Some(&account))
    }
}

impl From<AccountRef> for pb::AccountRef {
    fn from(account: AccountRef) -> Self {
        let reference = match account {
            AccountRef::Id(id) => pb::account_ref::Ref::Uuid(id.to_string()),
            AccountRef::External(external) => {
                pb::account_ref::Ref::ExternalRef(external.as_str().to_string())
            }
        };
        Self {
            r#ref: Some(reference),
        }
    }
}
