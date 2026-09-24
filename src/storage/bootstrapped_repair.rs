//! PALLIATIVE (#36): remove when whatsapp-rust marks an up-to-date collection
//! as bootstrapped and the pinned rev has the fix.
//!
//! The 0.7.0 -> main migration (#30) wrote `bootstrapped: false` into every
//! `HashState` it carried over, expecting the next sync to set it. whatsapp-rust
//! main only sets it when a sync applies a patch, a snapshot, or (with no
//! baseline) an empty answer: `process_patch_list` in `wacore/src/appstate_sync.rs`
//! requires `!had_baseline` on its "nothing to apply" branch. A migrated
//! collection has a baseline (`version > 0`), and when it is already at the
//! server's head the answer is empty, so the flag is never written. The send
//! path (`src/client/app_state.rs`) then refuses every patch on it with "its
//! bootstrap has not completed": since the 2026-09-23 deploy, every
//! `MarkChatRead` answered `unavailable` (448 `ok` before, 0 after).
//!
//! The repair marks such a collection bootstrapped. The cost, accepted: the flag
//! exists to catch a paged bootstrap that stopped part way, and a collection
//! that was mid-bootstrap under 0.7.0 is marked complete too. For the stores
//! this runs on, the collections had been syncing for weeks.
//!
//! It runs inside the one-time bincode -> protobuf conversion
//! (`bincode_upgrade`), which rewrites every `HashState` anyway, so it happens
//! exactly once per store and needs no marker of its own.

use wacore::appstate::hash::HashState;

/// Mark a synced-but-unmarked collection as bootstrapped. Returns whether it
/// changed anything. Every other field is left exactly as it was:
/// `mac_mismatch_fatal` in particular is a real latch, inherited from 0.7.0 on
/// the production stores, and not part of this bug.
pub fn repair_inherited_bootstrap(state: &mut HashState) -> bool {
    // version 0 is "never synced" or "synced and empty", byte-identical on
    // disk; only a real bootstrap can tell them apart, so it is left to one.
    if state.version == 0 || state.bootstrapped {
        return false;
    }
    state.bootstrapped = true;
    true
}

#[cfg(test)]
mod bootstrapped_repair_tests {
    use std::collections::HashMap;

    use super::*;

    fn state(version: u64, bootstrapped: bool, mac_mismatch_fatal: bool) -> HashState {
        HashState {
            version,
            hash: [0x3C; 128],
            index_value_map: HashMap::from([("i".to_string(), vec![7u8])]),
            mac_mismatch_fatal,
            bootstrapped,
        }
    }

    #[test]
    fn a_synced_unmarked_collection_is_marked() {
        let mut s = state(1399, false, false);
        assert!(repair_inherited_bootstrap(&mut s));
        assert!(s.bootstrapped);
    }

    #[test]
    fn version_zero_is_left_for_a_real_bootstrap() {
        let mut s = state(0, false, false);
        assert!(!repair_inherited_bootstrap(&mut s));
        assert!(!s.bootstrapped);
    }

    #[test]
    fn an_already_marked_collection_is_not_touched() {
        let mut s = state(339, true, false);
        assert!(!repair_inherited_bootstrap(&mut s));
        assert!(s.bootstrapped);
    }

    #[test]
    fn only_the_flag_changes_and_the_mac_latch_survives() {
        let mut s = state(339, false, true);
        assert!(repair_inherited_bootstrap(&mut s));
        assert!(s.mac_mismatch_fatal, "the latch is not this bug's to clear");
        assert_eq!(s.version, 339);
        assert_eq!(s.hash, [0x3C; 128]);
        assert_eq!(
            s.index_value_map,
            HashMap::from([("i".to_string(), vec![7u8])])
        );
    }
}
