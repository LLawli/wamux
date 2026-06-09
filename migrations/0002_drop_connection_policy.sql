-- Remove connection_policy: connect/disconnect is now entirely edge-driven.
-- The core no longer decides which accounts to keep connected (no always-on
-- auto-reconnect); the edge calls Connect/Disconnect per account. See the
-- "Core purity" section in CLAUDE.md.
ALTER TABLE accounts DROP COLUMN IF EXISTS connection_policy;
