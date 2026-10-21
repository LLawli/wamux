//! The checksum each embedded migration carries, pinned to what real stores
//! recorded (#209). sqlx refuses a store whose `_sqlx_migrations` row holds a
//! different checksum for an applied version ("previously applied but has been
//! modified"), so a sqlx release that hashed differently, or an edit to an
//! applied file, would lock every existing store out. The values were read from
//! `_sqlx_migrations` of stores sqlx 0.8 migrated: the live Postgres database
//! (0001 to 0006) and a production SQLite store (0001 to 0003); the SQLite 0005
//! and 0006 rows are the SHA-384 of the files, which is what those stores show
//! sqlx 0.8 recorded. A migration that ships later gets its row appended here.

use sqlx::migrate::Migrator;
use wamux::storage::sql::{PG_MIGRATOR, SQLITE_MIGRATOR};

const POSTGRES_RECORDED: &[(i64, &str)] = &[
    (
        1,
        "a5e25d232f4c99d1d7931cc06003cd764416ae0af7bfb9c16f3f4fcfa8a79b859048551242164489eaf8dc9e00f5b55d",
    ),
    (
        2,
        "a21a5c47dbda69ba5aa37eaa8e8c2e859c6f653bcb23e83c766c6da80a1708089c0fcd90ad8da1bffe7f248c86a1b089",
    ),
    (
        3,
        "afbf76e9986c32a2a86d6ee60c1e2372234d7ac6defe6eeb55aaf8a0f762314fe6b1b66e82594f8f4d0f14aec52cf667",
    ),
    (
        4,
        "5f0001cd7c0bc1b3163417ee139c1f9d3007819fb15e8c39e62c08c486bb55d91afbc663398a982bb670cb8f52de7b8d",
    ),
    (
        5,
        "a78be3b7df8077fe247f0e4bdbc4a2757bd24f5c5296f3797e9727a46468cd2190f405ea8d7d22ed613e3dccdbc259a2",
    ),
    (
        6,
        "59b040e02830328a874d7168305117912a3e8f8b0b4a8e422f8a8797bbbed1d791b6fd83eb0e5a034509cabe7c803d62",
    ),
];

const SQLITE_RECORDED: &[(i64, &str)] = &[
    (
        1,
        "e4f05b3449ebd40bcc15c3a778a7cbf61246b3e66146150b35e80068ec8685affd651b16fff3a661b94d77b0da520c23",
    ),
    (
        2,
        "49815e9fb0f9427cdf7027a0f9a343ca68e0f3306455563e43326e0c393ec007c6b2c091498eaaabe621fcfd2b4d2901",
    ),
    (
        3,
        "5f0001cd7c0bc1b3163417ee139c1f9d3007819fb15e8c39e62c08c486bb55d91afbc663398a982bb670cb8f52de7b8d",
    ),
    (
        5,
        "fa1d63d41955c1e45b20383b3bba3decbde8d88e190b69acc1e09dc8dbeaaf3b4bff1845c49b254364759f15e88a61ea",
    ),
    (
        6,
        "59b040e02830328a874d7168305117912a3e8f8b0b4a8e422f8a8797bbbed1d791b6fd83eb0e5a034509cabe7c803d62",
    ),
];

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Every pinned version is embedded, under the checksum the stores hold.
fn assert_checksums_recorded(migrator: &Migrator, recorded: &[(i64, &str)]) {
    let embedded: Vec<(i64, String)> = migrator
        .iter()
        .map(|migration| (migration.version, to_hex(&migration.checksum)))
        .collect();
    for (version, checksum) in recorded {
        let found = embedded.iter().find(|(v, _)| v == version);
        assert_eq!(
            found.map(|(_, hex)| hex.as_str()),
            Some(*checksum),
            "migration {version}: the embedded checksum is not the one stores recorded; \
             every store that applied it would be refused. Embedded: {embedded:#?}"
        );
    }
}

#[test]
fn postgres_migration_checksums_are_what_stores_recorded() {
    assert_checksums_recorded(&PG_MIGRATOR, POSTGRES_RECORDED);
}

#[test]
fn sqlite_migration_checksums_are_what_stores_recorded() {
    assert_checksums_recorded(&SQLITE_MIGRATOR, SQLITE_RECORDED);
}
