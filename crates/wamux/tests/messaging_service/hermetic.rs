//! A stress login stays on loopback (#234). The library fetches its client
//! version from `https://web.whatsapp.com/sw.js` on a device that never
//! fetched one, and `connect` waits on that fetch, so a suite that let it
//! through depended on the internet: offline every login timed out, and a
//! stalled read held the test runtime's drop forever.

use whatsapp_rust::version::WA_WEB_VERSION;

use crate::harness::fixture;

// Pinned is the library's own constant; a fetched version is whatever
// `sw.js` serves today, which moves with every WhatsApp Web release.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stress_login_takes_the_library_version_without_fetching_it() {
    let f = fixture("a_stress_login_takes_the_library_version_without_fetching_it").await;
    let device = f.logged.client.persistence_manager().get_device_snapshot();
    let version = (
        device.app_version_primary,
        device.app_version_secondary,
        device.app_version_tertiary,
    );
    assert_eq!(
        version, WA_WEB_VERSION,
        "a stress login must take the library's pinned version, not one fetched from web.whatsapp.com"
    );
    f.cleanup().await;
}
