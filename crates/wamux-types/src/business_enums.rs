//! The business-hours enums as the contract's enums (#132, part 1 of #74).
//!
//! The library keeps a token it does not name in an `Other(String)` fallback,
//! so each function returns UNKNOWN plus that token, and an empty raw for a
//! named value. The core is a relay and never drops what the server said.

use wacore::iq::business::{BusinessHourMode, DayOfWeek};
use wamux_proto::v1 as pb;

/// The day, and the server's token verbatim when it is UNKNOWN (empty otherwise).
pub fn day_of_week_of(day: &DayOfWeek) -> (pb::BusinessDayOfWeek, String) {
    let named = match day {
        DayOfWeek::Sunday => pb::BusinessDayOfWeek::Sunday,
        DayOfWeek::Monday => pb::BusinessDayOfWeek::Monday,
        DayOfWeek::Tuesday => pb::BusinessDayOfWeek::Tuesday,
        DayOfWeek::Wednesday => pb::BusinessDayOfWeek::Wednesday,
        DayOfWeek::Thursday => pb::BusinessDayOfWeek::Thursday,
        DayOfWeek::Friday => pb::BusinessDayOfWeek::Friday,
        DayOfWeek::Saturday => pb::BusinessDayOfWeek::Saturday,
        DayOfWeek::Other(token) => return (pb::BusinessDayOfWeek::Unknown, token.clone()),
    };
    (named, String::new())
}

/// The mode, and the server's token verbatim when it is UNKNOWN (empty otherwise).
pub fn business_hour_mode_of(mode: &BusinessHourMode) -> (pb::BusinessHourMode, String) {
    let named = match mode {
        BusinessHourMode::Open24H => pb::BusinessHourMode::Open24h,
        BusinessHourMode::SpecificHours => pb::BusinessHourMode::SpecificHours,
        BusinessHourMode::AppointmentOnly => pb::BusinessHourMode::AppointmentOnly,
        BusinessHourMode::Other(token) => return (pb::BusinessHourMode::Unknown, token.clone()),
    };
    (named, String::new())
}
