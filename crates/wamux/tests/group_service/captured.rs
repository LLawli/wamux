//! The five group reads as WhatsApp's server answered them on 2026-10-02,
//! transcribed from `captured-2026-10-02.xml` (anonymized; see its header).
//! Each function builds the CHILD of the `<iq type="result">`, which is what
//! `MockWaServer::answer_iq` wraps. `captured_answers_render_exactly_as_received`
//! proves every transcription renders back to the captured line, so a typo here
//! cannot pass for a server answer.

use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;

pub const GROUP: &str = "120363000000000068@g.us";
pub const GROUP_ID: &str = "120363000000000068";
pub const OWNER_PN: &str = "5511900000001@s.whatsapp.net";
pub const OWNER_LID: &str = "100000000000001@lid";
pub const REQUESTER_PN: &str = "5511900000002@s.whatsapp.net";
pub const REQUESTER_LID: &str = "100000000000002@lid";
pub const INVITE_CODE: &str = "AbCdEfGhIjKlMnOpQrStUv";
pub const SUBJECT: &str = "wamux capture 68";
pub const CREATION: &str = "1790947599";

fn text(tag: &'static str, content: &str) -> Node {
    NodeBuilder::new(tag).string_content(content).build()
}

fn approval_on() -> Node {
    NodeBuilder::new("membership_approval_mode")
        .children([NodeBuilder::new("group_join").attr("state", "on").build()])
        .build()
}

fn ephemeral_off() -> Node {
    NodeBuilder::new("ephemeral")
        .attr("expiration", "0")
        .build()
}

/// The owner as a full roster entry (metadata and list answers).
fn owner_full() -> Node {
    NodeBuilder::new("participant")
        .attr("jid", OWNER_LID)
        .attr("type", "superadmin")
        .attr("join_time", CREATION)
        .attr("group_history_sent", "false")
        .attr("phone_number", OWNER_PN)
        .build()
}

/// The `<group>` head both GetGroupMetadata and ListGroups answered with.
fn group_head() -> NodeBuilder {
    NodeBuilder::new("group")
        .attr("addressing_mode", "lid")
        .attr("subject", SUBJECT)
        .attr("creator_country_code", "BR")
        .attr("creator_pn", OWNER_PN)
        .attr("creator", OWNER_LID)
        .attr("s_o_pn", OWNER_PN)
        .attr("s_o", OWNER_LID)
        .attr("id", GROUP_ID)
        .attr("creation", CREATION)
        .attr("s_t", CREATION)
        .attr("p_v_id", "1790947897308104")
        .attr("a_v_id", "1790947599193645")
}

/// GetGroupMetadata: `<iq xmlns="w:g2" type="get"><query request="interactive"/>`.
pub fn metadata() -> Node {
    group_head()
        .attr("size", "1")
        .children([
            NodeBuilder::new("description").build(),
            text("member_add_mode", "all_member_add"),
            text("member_share_group_history_mode", "all_member_share"),
            ephemeral_off(),
            approval_on(),
            text("member_link_mode", "admin_link"),
            owner_full(),
        ])
        .build()
}

/// GetInviteLink: `<invite code=..>`.
pub fn invite() -> Node {
    NodeBuilder::new("invite").attr("code", INVITE_CODE).build()
}

/// GetMembershipRequests: one pending request, made through the invite link.
pub fn membership_requests() -> Node {
    NodeBuilder::new("membership_approval_requests")
        .children([NodeBuilder::new("membership_approval_request")
            .attr("jid", REQUESTER_LID)
            .attr("request_method", "invite_link")
            .attr("request_time", "1790947901")
            .attr("phone_number", REQUESTER_PN)
            .build()])
        .build()
}

/// PreviewInvite: the group as a non-member sees it through the code.
pub fn invite_preview() -> Node {
    NodeBuilder::new("group")
        .attr("addressing_mode", "lid")
        .attr("size", "1")
        .attr("s_o_pn", OWNER_PN)
        .attr("s_o", OWNER_LID)
        .attr("s_t", CREATION)
        .attr("subject", SUBJECT)
        .attr("creator_country_code", "BR")
        .attr("creator_pn", OWNER_PN)
        .attr("creator", OWNER_LID)
        .attr("creation", CREATION)
        .attr("id", GROUP_ID)
        .children([
            approval_on(),
            ephemeral_off(),
            NodeBuilder::new("participant")
                .attr("jid", OWNER_LID)
                .attr("phone_number", OWNER_PN)
                .attr("type", "superadmin")
                .build(),
            NodeBuilder::new("description").build(),
        ])
        .build()
}

/// ListGroups: `<groups>` trimmed to the capture group.
pub fn participating() -> Node {
    let group = group_head()
        .children([
            NodeBuilder::new("description").build(),
            text("member_add_mode", "all_member_add"),
            approval_on(),
            ephemeral_off(),
            text("member_link_mode", "admin_link"),
            text("member_share_group_history_mode", "all_member_share"),
            owner_full(),
        ])
        .build();
    NodeBuilder::new("groups").children([group]).build()
}

#[test]
fn captured_answers_render_exactly_as_received() {
    let file = include_str!("captured-2026-10-02.xml");
    let lines: Vec<&str> = file.lines().filter(|l| l.starts_with("<iq ")).collect();
    let built = [
        metadata(),
        invite(),
        membership_requests(),
        invite_preview(),
        participating(),
    ];
    assert_eq!(lines.len(), built.len(), "one captured <iq> per read");
    for (line, node) in lines.iter().zip(built.iter()) {
        crate::common::mock_wire::assert_transcribes(line, node);
    }
}
