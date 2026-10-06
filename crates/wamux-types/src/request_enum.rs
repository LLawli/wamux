//! The one refusal shape for a request enum (#127): `<field> must be one of
//! <A|B|...>, got <NAME>`. SendMedia, PostStatusMedia, DownloadMedia and
//! SendPresence all refuse through here, so the edge reads one message format
//! for UNSPECIFIED, UNKNOWN, a number outside the enum and a value outside the
//! operation's subset alike.

use crate::error::WamuxError;

/// `accepted` and `shown` are proto names (`as_str_name()`); `shown` is the
/// bare number when the wire value names no variant of the enum.
pub(crate) fn refusal(
    field: &str,
    accepted: impl Iterator<Item = &'static str>,
    shown: &str,
) -> WamuxError {
    let names: Vec<&str> = accepted.collect();
    WamuxError::InvalidArgument(format!(
        "{field} must be one of {}, got {shown}",
        names.join("|")
    ))
}

/// The proto name of a wire value (`name`, from `as_str_name()` when the value
/// names a variant), or the bare number when it does not (#127).
pub(crate) fn shown_value(name: Option<&'static str>, value: i32) -> String {
    name.map_or_else(|| value.to_string(), str::to_string)
}
