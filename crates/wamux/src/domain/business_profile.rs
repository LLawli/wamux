//! A business profile as the library parses it, projected onto the contract's
//! typed `BusinessProfile` (#132, part 1 of #74). It used to cross as the
//! library's `serde` JSON, a second schema the edge had to follow.

use wacore::iq::business::{BusinessCategory, BusinessHours, BusinessHoursConfig, BusinessProfile};
use wamux_types::business_enums::{business_hour_mode_of, day_of_week_of};
use wamux_types::relay_optional_lib_jid;

use crate::proto::v1 as pb;

/// Every field the library parses; the hours' day and mode as enums plus raw.
pub fn business_profile_of(profile: &BusinessProfile) -> pb::BusinessProfile {
    pb::BusinessProfile {
        wid: relay_optional_lib_jid(profile.wid.as_ref()),
        description: profile.description.clone(),
        email: profile.email.clone(),
        website: profile.website.clone(),
        categories: profile.categories.iter().map(category_of).collect(),
        address: profile.address.clone(),
        // Not optional in the library, so it always crosses.
        business_hours: Some(hours_of(&profile.business_hours)),
    }
}

fn category_of(category: &BusinessCategory) -> pb::BusinessCategory {
    pb::BusinessCategory {
        id: category.id.clone(),
        name: category.name.clone(),
    }
}

/// The library's `None` config (it only says so for an empty list) is an
/// empty repeated field.
fn hours_of(hours: &BusinessHours) -> pb::BusinessHours {
    pb::BusinessHours {
        timezone: hours.timezone.clone(),
        business_config: hours
            .business_config
            .iter()
            .flatten()
            .map(hours_config_of)
            .collect(),
    }
}

fn hours_config_of(config: &BusinessHoursConfig) -> pb::BusinessHoursConfig {
    let (day, day_raw) = day_of_week_of(&config.day_of_week);
    let (mode, mode_raw) = business_hour_mode_of(&config.mode);
    pb::BusinessHoursConfig {
        day_of_week: day as i32,
        day_of_week_raw: day_raw,
        mode: mode as i32,
        mode_raw,
        open_time: config.open_time,
        close_time: config.close_time,
    }
}

#[cfg(test)]
#[path = "business_profile_tests.rs"]
mod tests;
