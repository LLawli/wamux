//! #132: GetBusinessProfile through the socket, answered with what WhatsApp's
//! server sent on 2026-10-07 (`captured-business-2026-10-07.xml`, anonymized),
//! relayed as the typed `BusinessProfile`. Here and not in a ContactService
//! suite of its own because this harness already serves the ContactService
//! (#121 moved jid fields there).

use wacore_binary::Node;
use wacore_binary::builder::NodeBuilder;
use wamux::proto::v1 as pb;

use crate::common::mock_wire::{assert_transcribes, attr, sent_iq};
use crate::harness::{fixture, jid};

const BIZ: &str = "w:biz";
const BUSINESS: &str = "5511900000003@s.whatsapp.net";

fn text(tag: &'static str, content: &str) -> Node {
    NodeBuilder::new(tag).string_content(content).build()
}

fn hours(day: &str, open: &str, close: &str) -> Node {
    NodeBuilder::new("business_hours_config")
        .attr("day_of_week", day)
        .attr("mode", "specific_hours")
        .attr("open_time", open)
        .attr("close_time", close)
        .build()
}

fn business_hours() -> Node {
    NodeBuilder::new("business_hours")
        .attr("timezone", "America/Sao_Paulo")
        .children([
            hours("mon", "480", "1080"),
            hours("tue", "480", "1080"),
            hours("wed", "480", "1080"),
            hours("thu", "480", "1080"),
            hours("fri", "480", "1080"),
            hours("sat", "480", "720"),
        ])
        .build()
}

/// The children the library does not read, kept so the answer is the capture.
fn profile_options() -> Node {
    let bot_fields = NodeBuilder::new("bot_fields")
        .children([text("is_typing_indicator_enabled", "false")])
        .build();
    NodeBuilder::new("profile_options")
        .children([
            text("commerce_experience", "none"),
            text("cart_enabled", "true"),
            text("direct_connection", "false"),
            text("is_responsive", "true"),
            bot_fields,
        ])
        .build()
}

fn identity() -> Node {
    NodeBuilder::new("biz_identity_info")
        .attr("phone_number", "")
        .attr("type", "smb")
        .attr("display_name", "Clinica Exemplo")
        .attr("vlevel", "low")
        .attr("serial", "1000000000000000003")
        .attr("is_signed", "true")
        .attr("revoked", "false")
        .build()
}

fn unread_tail() -> [Node; 7] {
    [
        NodeBuilder::new("direct_connection")
            .attr("enabled", "false")
            .children([NodeBuilder::new("features").attr("name", "default").build()])
            .build(),
        text("member_since_text", "Joined in March, 2025"),
        text("automated_type", "unknown"),
        identity(),
        text("member_since_ts", "1742915099"),
        NodeBuilder::new("aea_account_type")
            .attr("value", "hybrid_e2ee")
            .build(),
        text("calling_automated_type", "unknown"),
    ]
}

/// GetBusinessProfile: `<iq xmlns="w:biz" type="get"><business_profile>`.
fn captured_profile() -> Node {
    let category = NodeBuilder::new("category")
        .attr("id", "145118935550090")
        .string_content("Medical & health")
        .build();
    let mut children = vec![
        text(
            "address",
            "R. Exemplo, 1 - Centro, Cidade - UF, 00000-000, Brasil",
        ),
        text(
            "email",
            "contato.da.clinica.exemplo.anonimizada@example.com",
        ),
        text("latitude", "-15.0000"),
        text("longitude", "-47.0000"),
        business_hours(),
        NodeBuilder::new("categories").children([category]).build(),
        profile_options(),
    ];
    children.extend(unread_tail());
    let profile = NodeBuilder::new("profile")
        .attr("jid", BUSINESS)
        .attr("tag", "1000000003")
        .children(children)
        .build();
    NodeBuilder::new("business_profile")
        .children([profile])
        .build()
}

fn hours_on_the_wire(day: pb::BusinessDayOfWeek, close: u32) -> pb::BusinessHoursConfig {
    pb::BusinessHoursConfig {
        day_of_week: day as i32,
        day_of_week_raw: String::new(),
        mode: pb::BusinessHourMode::SpecificHours as i32,
        mode_raw: String::new(),
        open_time: Some(480),
        close_time: Some(close),
    }
}

/// What the edge receives for the capture: every field the library parses,
/// by value. The server sent no description and no website, which the library
/// reads as "" and an empty list.
fn captured_profile_on_the_wire() -> pb::BusinessProfile {
    use pb::BusinessDayOfWeek as Day;
    pb::BusinessProfile {
        wid: jid(BUSINESS),
        description: String::new(),
        email: Some("contato.da.clinica.exemplo.anonimizada@example.com".to_string()),
        website: Vec::new(),
        categories: vec![pb::BusinessCategory {
            id: "145118935550090".to_string(),
            name: "Medical & health".to_string(),
        }],
        address: Some("R. Exemplo, 1 - Centro, Cidade - UF, 00000-000, Brasil".to_string()),
        business_hours: Some(pb::BusinessHours {
            timezone: Some("America/Sao_Paulo".to_string()),
            business_config: vec![
                hours_on_the_wire(Day::Monday, 1080),
                hours_on_the_wire(Day::Tuesday, 1080),
                hours_on_the_wire(Day::Wednesday, 1080),
                hours_on_the_wire(Day::Thursday, 1080),
                hours_on_the_wire(Day::Friday, 1080),
                hours_on_the_wire(Day::Saturday, 720),
            ],
        }),
    }
}

fn request(account: &pb::AccountRef) -> pb::JidRequest {
    pb::JidRequest {
        account: Some(account.clone()),
        jid: jid(BUSINESS),
    }
}

#[test]
fn captured_business_profile_renders_exactly_as_received() {
    let file = include_str!("captured-business-2026-10-07.xml");
    let lines: Vec<&str> = file.lines().filter(|l| l.starts_with("<iq ")).collect();
    assert_eq!(lines.len(), 1, "one captured <iq>");
    assert_transcribes(lines[0], &captured_profile());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_business_profile_relays_the_captured_profile() {
    let mut f = fixture("get_business_profile_relays_the_captured_profile").await;
    f.mock
        .answer_iq(BIZ, "get", "business_profile", captured_profile());
    let answer = f
        .contacts
        .get_business_profile(request(&f.account))
        .await
        .expect("business profile")
        .into_inner();
    assert_eq!(answer.profile, Some(captured_profile_on_the_wire()));
    let iq = sent_iq(&f.mock, BIZ, "get", "business_profile").await;
    let asked = iq
        .get_optional_child("business_profile")
        .and_then(|b| b.get_optional_child("profile"))
        .and_then(|p| attr(p, "jid"));
    assert_eq!(asked.as_deref(), Some(BUSINESS));
    f.cleanup().await;
}

/// A jid with no business profile: the server answers `<business_profile>`
/// with no `<profile>`, the library answers `None`, and the field stays unset
/// rather than crossing as an empty profile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_business_profile_of_a_non_business_leaves_it_unset() {
    let mut f = fixture("get_business_profile_of_a_non_business_leaves_it_unset").await;
    let empty = NodeBuilder::new("business_profile").build();
    f.mock.answer_iq(BIZ, "get", "business_profile", empty);
    let answer = f
        .contacts
        .get_business_profile(request(&f.account))
        .await
        .expect("business profile")
        .into_inner();
    assert_eq!(answer.profile, None);
    f.cleanup().await;
}
