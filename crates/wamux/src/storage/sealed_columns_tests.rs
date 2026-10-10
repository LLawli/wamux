//! #165: the list the conversion walks, and the row key it names each row by.
//! The row keys here must equal what the stores pass to `seal_at`/`open_at`;
//! `tests/store_conversion.rs` proves it end to end by reading a converted
//! store through the traits.

use super::{RowPart, SealedColumn, row_key, sealed_columns};

fn column(table: &str, name: &str) -> SealedColumn {
    *sealed_columns()
        .iter()
        .find(|c| c.table == table && c.column == name)
        .unwrap_or_else(|| panic!("{table}.{name} is not in the list"))
}

fn text(value: &str) -> RowPart {
    RowPart::Text(value.to_string())
}

#[test]
fn the_list_has_exactly_the_thirteen_sealed_columns() {
    let mut listed: Vec<(&str, &str)> = sealed_columns()
        .iter()
        .map(|c| (c.table, c.column))
        .collect();
    listed.sort();
    let expected = [
        ("app_state_keys", "key_data"),
        ("app_state_mutation_macs", "value_mac"),
        ("app_state_versions", "state_data"),
        ("base_keys", "base_key"),
        ("device", "data"),
        ("identities", "key"),
        ("msg_secrets", "secret"),
        ("prekeys", "key"),
        ("sender_keys", "record"),
        ("sent_messages", "payload"),
        ("sessions", "record"),
        ("signed_prekeys", "record"),
        ("tc_tokens", "token"),
    ];
    assert_eq!(listed, expected);
}

#[test]
fn row_key_of_a_prekey_is_its_id_big_endian() {
    for (table, name) in [("prekeys", "key"), ("signed_prekeys", "record")] {
        let key = row_key(&column(table, name), &[RowPart::Int(7)]);
        assert_eq!(key, [0, 0, 0, 7], "{table}");
        let big = row_key(&column(table, name), &[RowPart::Int(0x0102_0304)]);
        assert_eq!(big, [1, 2, 3, 4], "{table}");
    }
}

#[test]
fn row_key_of_a_single_column_is_its_bytes() {
    let cases = [
        (
            "sessions",
            "record",
            text("alice@s.whatsapp.net"),
            b"alice@s.whatsapp.net".to_vec(),
        ),
        (
            "identities",
            "key",
            text("bob@s.whatsapp.net"),
            b"bob@s.whatsapp.net".to_vec(),
        ),
        (
            "sender_keys",
            "record",
            text("g@g.us::alice"),
            b"g@g.us::alice".to_vec(),
        ),
        ("tc_tokens", "token", text("1@lid"), b"1@lid".to_vec()),
        (
            "app_state_versions",
            "state_data",
            text("regular_low"),
            b"regular_low".to_vec(),
        ),
        (
            "app_state_keys",
            "key_data",
            RowPart::Bytes(vec![0, 1, 0xff]),
            vec![0, 1, 0xff],
        ),
    ];
    for (table, name, part, expected) in cases {
        assert_eq!(
            row_key(&column(table, name), &[part]),
            expected,
            "{table}.{name}"
        );
    }
}

#[test]
fn row_key_joins_the_parts_with_a_zero_byte() {
    let secret = row_key(
        &column("msg_secrets", "secret"),
        &[text("chat"), text("sender"), text("M1")],
    );
    assert_eq!(secret, b"chat\0sender\0M1");
    let base = row_key(
        &column("base_keys", "base_key"),
        &[text("alice.0"), text("M1")],
    );
    assert_eq!(base, b"alice.0\0M1");
    let sent = row_key(
        &column("sent_messages", "payload"),
        &[text("chat"), text("M1")],
    );
    assert_eq!(sent, b"chat\0M1");
    let mac = row_key(
        &column("app_state_mutation_macs", "value_mac"),
        &[text("regular_low"), RowPart::Bytes(vec![0x1d, 0x00, 0x1d])],
    );
    assert_eq!(mac, b"regular_low\0\x1d\x00\x1d");
}

#[test]
fn row_key_of_the_device_row_is_empty() {
    assert!(row_key(&column("device", "data"), &[]).is_empty());
}
