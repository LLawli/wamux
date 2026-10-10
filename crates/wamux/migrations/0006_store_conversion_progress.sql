-- Which accounts a store conversion has already done (#165).
--
-- Converting a store to encrypted (or back) is one transaction per account, so
-- an interruption leaves some accounts done and some not. A row here is an
-- account whose secret columns are already in the target form; the converter
-- skips those on the retry, so nothing is sealed twice. The last account's
-- transaction flips store_encryption.state and clears this table.
--
-- Read together with store_encryption: 'plaintext' with a verifier is an
-- encryption in progress, 'encrypted' with rows here is a decryption in
-- progress. The CHECK on state is untouched on purpose.

CREATE TABLE store_conversion_progress (
    device_id INTEGER PRIMARY KEY REFERENCES accounts(device_id) ON DELETE CASCADE
);
