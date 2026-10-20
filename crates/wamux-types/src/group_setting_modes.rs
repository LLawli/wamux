//! The group settings an admin sets by mode (#198), read from a request. The
//! contract reuses the enums GetGroupMetadata reports, which carry UNSPECIFIED
//! and UNKNOWN; neither names a setting, so both are refused here, in the one
//! refusal shape every request enum shares (#127).

use wacore::iq::groups::{MemberAddMode, MemberLinkMode, MemberShareHistoryMode};
use wamux_proto::v1 as pb;

use crate::error::WamuxError;
use crate::request_enum::{refusal, shown_value};

/// The one refusal shape the three mode settings share.
fn refuse_mode(value: i32, name: Option<&'static str>, accepted: [&'static str; 2]) -> WamuxError {
    refusal("mode", accepted.into_iter(), &shown_value(name, value))
}

/// `InvalidArgument("mode must be one of GROUP_MEMBER_ADD_MODE_ADMIN_ADD|GROUP_MEMBER_ADD_MODE_ALL_MEMBER_ADD, got <v>")`
/// for UNSPECIFIED, UNKNOWN or a number outside the enum.
pub fn parse_member_add_mode(value: i32) -> Result<MemberAddMode, WamuxError> {
    let parsed = match pb::GroupMemberAddMode::try_from(value) {
        Ok(pb::GroupMemberAddMode::AdminAdd) => Some(MemberAddMode::AdminAdd),
        Ok(pb::GroupMemberAddMode::AllMemberAdd) => Some(MemberAddMode::AllMemberAdd),
        // No wildcard: a variant added to the proto breaks the build here.
        Ok(pb::GroupMemberAddMode::Unspecified | pb::GroupMemberAddMode::Unknown) | Err(_) => None,
    };
    parsed.ok_or_else(|| {
        let name = pb::GroupMemberAddMode::try_from(value)
            .ok()
            .map(|wire| wire.as_str_name());
        let accepted = [
            pb::GroupMemberAddMode::AdminAdd.as_str_name(),
            pb::GroupMemberAddMode::AllMemberAdd.as_str_name(),
        ];
        refuse_mode(value, name, accepted)
    })
}

/// Same refusal as [`parse_member_add_mode`], over `GroupMemberLinkMode`.
pub fn parse_member_link_mode(value: i32) -> Result<MemberLinkMode, WamuxError> {
    let parsed = match pb::GroupMemberLinkMode::try_from(value) {
        Ok(pb::GroupMemberLinkMode::AdminLink) => Some(MemberLinkMode::AdminLink),
        Ok(pb::GroupMemberLinkMode::AllMemberLink) => Some(MemberLinkMode::AllMemberLink),
        // No wildcard: a variant added to the proto breaks the build here.
        Ok(pb::GroupMemberLinkMode::Unspecified | pb::GroupMemberLinkMode::Unknown) | Err(_) => {
            None
        }
    };
    parsed.ok_or_else(|| {
        let name = pb::GroupMemberLinkMode::try_from(value)
            .ok()
            .map(|wire| wire.as_str_name());
        let accepted = [
            pb::GroupMemberLinkMode::AdminLink.as_str_name(),
            pb::GroupMemberLinkMode::AllMemberLink.as_str_name(),
        ];
        refuse_mode(value, name, accepted)
    })
}

/// Short name that keeps the share-history parser under the line limit.
type ShareWire = pb::GroupMemberShareHistoryMode;

/// Same refusal as [`parse_member_add_mode`], over `GroupMemberShareHistoryMode`.
pub fn parse_member_share_history_mode(value: i32) -> Result<MemberShareHistoryMode, WamuxError> {
    let parsed = match ShareWire::try_from(value) {
        Ok(ShareWire::AdminShare) => Some(MemberShareHistoryMode::AdminShare),
        Ok(ShareWire::AllMemberShare) => Some(MemberShareHistoryMode::AllMemberShare),
        // No wildcard: a variant added to the proto breaks the build here.
        Ok(ShareWire::Unspecified | ShareWire::Unknown) | Err(_) => None,
    };
    parsed.ok_or_else(|| {
        let name = ShareWire::try_from(value)
            .ok()
            .map(|wire| wire.as_str_name());
        let accepted = [
            ShareWire::AdminShare.as_str_name(),
            ShareWire::AllMemberShare.as_str_name(),
        ];
        refuse_mode(value, name, accepted)
    })
}
