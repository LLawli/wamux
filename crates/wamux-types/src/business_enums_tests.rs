//! #132: every day and mode the library names maps to its own value with an
//! empty raw; a token it does not name lands in UNKNOWN with the token kept.

use wacore::iq::business::{BusinessHourMode, DayOfWeek};
use wamux_proto::v1 as pb;

use crate::business_enums::{business_hour_mode_of, day_of_week_of};

#[test]
fn every_named_day_maps_with_no_raw() {
    let cases = [
        (DayOfWeek::Sunday, pb::BusinessDayOfWeek::Sunday),
        (DayOfWeek::Monday, pb::BusinessDayOfWeek::Monday),
        (DayOfWeek::Tuesday, pb::BusinessDayOfWeek::Tuesday),
        (DayOfWeek::Wednesday, pb::BusinessDayOfWeek::Wednesday),
        (DayOfWeek::Thursday, pb::BusinessDayOfWeek::Thursday),
        (DayOfWeek::Friday, pb::BusinessDayOfWeek::Friday),
        (DayOfWeek::Saturday, pb::BusinessDayOfWeek::Saturday),
    ];
    for (library, wire) in cases {
        assert_eq!(day_of_week_of(&library), (wire, String::new()), "{wire:?}");
    }
}

#[test]
fn an_unnamed_day_keeps_its_token() {
    assert_eq!(
        day_of_week_of(&DayOfWeek::Other("hol".to_string())),
        (pb::BusinessDayOfWeek::Unknown, "hol".to_string())
    );
}

#[test]
fn every_named_hour_mode_maps_with_no_raw() {
    let cases = [
        (BusinessHourMode::Open24H, pb::BusinessHourMode::Open24h),
        (
            BusinessHourMode::SpecificHours,
            pb::BusinessHourMode::SpecificHours,
        ),
        (
            BusinessHourMode::AppointmentOnly,
            pb::BusinessHourMode::AppointmentOnly,
        ),
    ];
    for (library, wire) in cases {
        assert_eq!(
            business_hour_mode_of(&library),
            (wire, String::new()),
            "{wire:?}"
        );
    }
}

#[test]
fn an_unnamed_hour_mode_keeps_its_token() {
    assert_eq!(
        business_hour_mode_of(&BusinessHourMode::Other("closed".to_string())),
        (pb::BusinessHourMode::Unknown, "closed".to_string())
    );
}
