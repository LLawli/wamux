//! What `logout_e2e` asserts about the account after `Logout` (#257).
//!
//! Logout leaves the account LOGGED_OUT, not DISCONNECTED: the server removed
//! the device, so the state is terminal and a later socket close must not
//! downgrade it. The verdict lives here, apart from the binary, so a test can
//! pin it without a daemon.

use wamux::proto::v1 as pb;

use crate::report::Report;

const CHECK_NAME: &str = "Account.GetAccountStatus after Logout";

/// Record whether the single post-Logout read of the state is LOGGED_OUT.
pub fn record_post_logout_state(report: &mut Report, state: i32) {
    let shown: String = pb::ConnectionState::try_from(state)
        .map(|s| format!("{s:?}"))
        .unwrap_or_else(|_| state.to_string());
    report.verify(
        CHECK_NAME,
        state == pb::ConnectionState::LoggedOut as i32,
        format!("post-logout state = {shown}, expected LoggedOut"),
    );
}
