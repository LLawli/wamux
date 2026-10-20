//! The three group setting modes (#198): both values of each parse to the
//! library's mode, and every other wire value is refused in the #127 shape.

use wacore::iq::groups::{MemberAddMode, MemberLinkMode, MemberShareHistoryMode};
use wamux_proto::v1 as pb;

use crate::WamuxError;
use crate::group_setting_modes::{
    parse_member_add_mode, parse_member_link_mode, parse_member_share_history_mode,
};

fn invalid_argument<T: std::fmt::Debug>(result: Result<T, WamuxError>) -> String {
    match result {
        Err(WamuxError::InvalidArgument(message)) => message,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

/// UNSPECIFIED (0, what an omitted field reads as), UNKNOWN (1) and a number
/// the enum does not name, each with what the refusal shows for it.
fn refused(prefix: &str) -> [(i32, String); 3] {
    [
        (0, format!("{prefix}_UNSPECIFIED")),
        (1, format!("{prefix}_UNKNOWN")),
        (42, "42".to_string()),
    ]
}

#[test]
fn member_add_mode_parses_both_modes() {
    assert_eq!(
        parse_member_add_mode(pb::GroupMemberAddMode::AdminAdd as i32).unwrap(),
        MemberAddMode::AdminAdd
    );
    assert_eq!(
        parse_member_add_mode(pb::GroupMemberAddMode::AllMemberAdd as i32).unwrap(),
        MemberAddMode::AllMemberAdd
    );
}

#[test]
fn member_add_mode_refuses_unspecified_unknown_and_out_of_range() {
    for (value, shown) in refused("GROUP_MEMBER_ADD_MODE") {
        assert_eq!(
            invalid_argument(parse_member_add_mode(value)),
            format!(
                "mode must be one of GROUP_MEMBER_ADD_MODE_ADMIN_ADD|\
                 GROUP_MEMBER_ADD_MODE_ALL_MEMBER_ADD, got {shown}"
            ),
            "value {value}"
        );
    }
}

#[test]
fn member_link_mode_parses_both_modes() {
    assert_eq!(
        parse_member_link_mode(pb::GroupMemberLinkMode::AdminLink as i32).unwrap(),
        MemberLinkMode::AdminLink
    );
    assert_eq!(
        parse_member_link_mode(pb::GroupMemberLinkMode::AllMemberLink as i32).unwrap(),
        MemberLinkMode::AllMemberLink
    );
}

#[test]
fn member_link_mode_refuses_unspecified_unknown_and_out_of_range() {
    for (value, shown) in refused("GROUP_MEMBER_LINK_MODE") {
        assert_eq!(
            invalid_argument(parse_member_link_mode(value)),
            format!(
                "mode must be one of GROUP_MEMBER_LINK_MODE_ADMIN_LINK|\
                 GROUP_MEMBER_LINK_MODE_ALL_MEMBER_LINK, got {shown}"
            ),
            "value {value}"
        );
    }
}

#[test]
fn member_share_history_mode_parses_both_modes() {
    assert_eq!(
        parse_member_share_history_mode(pb::GroupMemberShareHistoryMode::AdminShare as i32)
            .unwrap(),
        MemberShareHistoryMode::AdminShare
    );
    assert_eq!(
        parse_member_share_history_mode(pb::GroupMemberShareHistoryMode::AllMemberShare as i32)
            .unwrap(),
        MemberShareHistoryMode::AllMemberShare
    );
}

#[test]
fn member_share_history_mode_refuses_unspecified_unknown_and_out_of_range() {
    for (value, shown) in refused("GROUP_MEMBER_SHARE_HISTORY_MODE") {
        assert_eq!(
            invalid_argument(parse_member_share_history_mode(value)),
            format!(
                "mode must be one of GROUP_MEMBER_SHARE_HISTORY_MODE_ADMIN_SHARE|\
                 GROUP_MEMBER_SHARE_HISTORY_MODE_ALL_MEMBER_SHARE, got {shown}"
            ),
            "value {value}"
        );
    }
}
