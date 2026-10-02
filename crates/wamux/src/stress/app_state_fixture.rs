//! App-state for the socket suites (#71): what a store needs so the client
//! sends a mutation at once, and how a test reads the mutation back.
//!
//! The library sends an app-state patch only with a sync key in the store and
//! the collection marked bootstrapped; otherwise it syncs first and the mock
//! has nothing to sync. A test seeds both before connecting, then opens the
//! patch the client put on the wire with that same key, as the user's other
//! devices would. Same recipe as the library's own `capture_app_state_mutation`
//! (cfg(test) there, so unreachable from here).

use anyhow::{Context, anyhow};
use wacore::appstate::hash::{HashState, generate_patch_mac};
use wacore::appstate::{Mutation, decode_record, expand_app_state_keys};
use wacore::store::traits::{AppStateSyncKey, Backend};
use wacore_binary::Node;
use whatsapp_rust::WAPatchName;
use whatsapp_rust::waproto::codec::syncd_patch_decode;

use super::node_read::{bytes_of, str_attr};

/// The key material behind `MOCK_SYNC_KEY_ID`. Fixed, since both ends of the
/// fixture (the seeding and the reading) are in this process.
const MOCK_SYNC_KEY_DATA: [u8; 32] = [5; 32];

/// Every collection the library can send a patch to; `Unknown` is its
/// catch-all for names it cannot parse, not a collection.
const COLLECTIONS: [WAPatchName; 5] = [
    WAPatchName::CriticalBlock,
    WAPatchName::CriticalUnblockLow,
    WAPatchName::Regular,
    WAPatchName::RegularHigh,
    WAPatchName::RegularLow,
];

/// The id of the sync key `seed_app_state` stores.
pub const MOCK_SYNC_KEY_ID: &[u8] = b"wamux-mock-sync-key";

/// One patch as the client sent it.
#[derive(Debug)]
pub struct OpenedPatch {
    /// The `<collection name>` it was sent to (`regular_high`, `regular_low`...).
    pub collection: String,
    /// Its mutations, decoded and MAC-checked, in order.
    pub mutations: Vec<Mutation>,
}

/// Store a sync key under `MOCK_SYNC_KEY_ID` and a bootstrapped, empty version
/// for every app-state collection, before the client connects.
pub async fn seed_app_state(backend: &dyn Backend) -> anyhow::Result<()> {
    let key = AppStateSyncKey {
        key_data: MOCK_SYNC_KEY_DATA.to_vec(),
        ..Default::default()
    };
    backend
        .set_sync_key(MOCK_SYNC_KEY_ID, key)
        .await
        .context("store the mock sync key")?;
    for collection in COLLECTIONS {
        // Bootstrapped: `send_app_state_patch` syncs a collection first
        // otherwise, and the mock has nothing to sync (client/app_state.rs:3030).
        let state = HashState {
            version: 7,
            bootstrapped: true,
            ..Default::default()
        };
        backend
            .set_version(collection.as_str(), state)
            .await
            .with_context(|| format!("seed the {} version", collection.as_str()))?;
    }
    Ok(())
}

/// Decode the patch in a client's `<iq xmlns="w:sync:app:state" type="set">`
/// with the seeded key: every record's index and value MACs and the patch MAC
/// are checked, so a patch the key cannot vouch for is an error, not a result.
pub fn open_app_state_patch(iq: &Node) -> anyhow::Result<OpenedPatch> {
    let collection = iq
        .get_optional_child_by_tag(&["sync", "collection"])
        .context("the <iq> carries no <sync><collection>")?;
    let name = str_attr(collection, "name").context("<collection> without a name")?;
    let base_version: u64 = str_attr(collection, "version")
        .context("<collection> without a version")?
        .parse()
        .context("<collection version> is not a number")?;
    let patch_node = collection
        .get_optional_child("patch")
        .context("<collection> carries no <patch>")?;
    let patch = syncd_patch_decode(bytes_of(patch_node)?).context("decode the patch")?;
    let keys = expand_app_state_keys(&MOCK_SYNC_KEY_DATA);
    // The patch carries no version of its own: the MAC covers the one after
    // the base the client built on (processor::validate_patch_macs reads it
    // off a server patch, which this is not).
    let expected = generate_patch_mac(&patch, &name, &keys.patch_mac, base_version + 1);
    if patch.patch_mac.as_deref() != Some(expected.as_slice()) {
        return Err(anyhow!("the patch MAC of {name} does not verify"));
    }
    let key_id = patch
        .key_id
        .id
        .clone()
        .context("the patch names no sync key")?;
    let mutations = patch
        .mutations
        .iter()
        .map(|mutation| open_mutation(mutation, &keys, &key_id))
        .collect::<anyhow::Result<Vec<Mutation>>>()?;
    Ok(OpenedPatch {
        collection: name,
        mutations,
    })
}

fn open_mutation(
    mutation: &whatsapp_rust::waproto::whatsapp::SyncdMutation,
    keys: &wacore::appstate::ExpandedAppStateKeys,
    key_id: &[u8],
) -> anyhow::Result<Mutation> {
    let operation = mutation
        .operation
        .and_then(|operation| operation.as_known())
        .context("a mutation without a known operation")?;
    let record = mutation
        .record
        .as_option()
        .context("a mutation without a record")?;
    let (opened, _macs) =
        decode_record(operation, record, keys, key_id, true).context("open the record")?;
    Ok(opened)
}
