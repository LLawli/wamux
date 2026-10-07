//! #132: the typed projection of a library `BusinessProfile`, asserted as the
//! whole message. `BusinessCategory` has no public constructor, so categories
//! are covered by the captured profile through the socket
//! (`tests/group_service/business_profile.rs`).

use std::str::FromStr;

use wacore::iq::business::{BusinessHourMode, BusinessHours, BusinessHoursConfig, DayOfWeek};
use whatsapp_rust::Jid;

use super::*;

const BUSINESS: &str = "5511900000003@s.whatsapp.net";

/// Every field the test can set, away from its default. The structs are
/// `#[non_exhaustive]`, so they can only be built by assignment.
#[allow(clippy::field_reassign_with_default)]
fn every_settable_field() -> BusinessProfile {
    let mut hours = BusinessHours::default();
    hours.timezone = Some("America/Sao_Paulo".to_string());
    hours.business_config = Some(vec![
        BusinessHoursConfig::with_hours(
            DayOfWeek::Monday,
            BusinessHourMode::SpecificHours,
            480,
            1080,
        ),
        BusinessHoursConfig::new(DayOfWeek::Sunday, BusinessHourMode::Open24H),
        BusinessHoursConfig::new(
            DayOfWeek::Other("hol".to_string()),
            BusinessHourMode::Other("closed".to_string()),
        ),
    ]);
    let mut profile = BusinessProfile::default();
    profile.wid = Some(Jid::from_str(BUSINESS).expect("test jid parses"));
    profile.description = "description".to_string();
    profile.email = Some("contact@example.com".to_string());
    profile.website = vec![
        "https://example.com".to_string(),
        "https://example.org".to_string(),
    ];
    profile.address = Some("Rua Exemplo, 1".to_string());
    profile.business_hours = hours;
    profile
}

fn hours_on_the_wire(
    day: pb::BusinessDayOfWeek,
    day_raw: &str,
    mode: pb::BusinessHourMode,
    mode_raw: &str,
    range: Option<(u32, u32)>,
) -> pb::BusinessHoursConfig {
    pb::BusinessHoursConfig {
        day_of_week: day as i32,
        day_of_week_raw: day_raw.to_string(),
        mode: mode as i32,
        mode_raw: mode_raw.to_string(),
        open_time: range.map(|(open, _)| open),
        close_time: range.map(|(_, close)| close),
    }
}

#[test]
fn business_profile_maps_every_field() {
    use pb::BusinessDayOfWeek as Day;
    use pb::BusinessHourMode as Mode;
    let expected = pb::BusinessProfile {
        wid: Some(pb::Jid {
            value: BUSINESS.to_string(),
        }),
        description: "description".to_string(),
        email: Some("contact@example.com".to_string()),
        website: vec![
            "https://example.com".to_string(),
            "https://example.org".to_string(),
        ],
        categories: Vec::new(),
        address: Some("Rua Exemplo, 1".to_string()),
        business_hours: Some(pb::BusinessHours {
            timezone: Some("America/Sao_Paulo".to_string()),
            business_config: vec![
                hours_on_the_wire(Day::Monday, "", Mode::SpecificHours, "", Some((480, 1080))),
                hours_on_the_wire(Day::Sunday, "", Mode::Open24h, "", None),
                hours_on_the_wire(Day::Unknown, "hol", Mode::Unknown, "closed", None),
            ],
        }),
    };
    assert_eq!(business_profile_of(&every_settable_field()), expected);
}

/// The library's own defaults: no wid, empty description, no hours at all.
/// Absent stays unset; `business_hours` still crosses, empty, because the
/// library's field is not optional.
#[test]
fn business_profile_absent_fields_stay_unset() {
    let expected = pb::BusinessProfile {
        business_hours: Some(pb::BusinessHours::default()),
        ..pb::BusinessProfile::default()
    };
    assert_eq!(business_profile_of(&BusinessProfile::default()), expected);
}
