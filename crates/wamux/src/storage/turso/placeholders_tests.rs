//! Why the Turso family rewrites `$N` (#106). turso 0.8.1 binds `$N` the way
//! plain SQLite binds a named parameter: by order of FIRST APPEARANCE, not by
//! the number. A statement whose placeholders appear out of order (the
//! tc-token upsert has `$1, $5, $2, $2, $3, $4`) binds every value into the
//! wrong column and still returns Ok. `?N` binds by number.

use super::turso_placeholders;

async fn memory_connection() -> ::turso::Connection {
    let db = ::turso::Builder::new_local(":memory:")
        .build()
        .await
        .expect("in-memory turso");
    db.connect().expect("connect")
}

async fn first_row(conn: &::turso::Connection, sql: &str) -> Vec<::turso::Value> {
    let params = vec![
        ::turso::Value::Text("a".into()),
        ::turso::Value::Text("b".into()),
    ];
    let mut rows = conn.query(sql, params).await.expect("query");
    let row = rows.next().await.expect("step").expect("one row");
    (0..row.column_count())
        .map(|i| row.get_value(i).expect("column"))
        .collect()
}

fn texts(values: &[&str]) -> Vec<::turso::Value> {
    values
        .iter()
        .map(|v| ::turso::Value::Text((*v).into()))
        .collect()
}

#[test]
fn rewrites_every_dollar_placeholder() {
    assert_eq!(
        turso_placeholders("VALUES ($1, $5, $2, $2, $3, $4)"),
        "VALUES (?1, ?5, ?2, ?2, ?3, ?4)"
    );
    assert_eq!(
        turso_placeholders("WHERE id IN ($2, $3, $10, $102) AND x = $1"),
        "WHERE id IN (?2, ?3, ?10, ?102) AND x = ?1"
    );
    assert_eq!(
        turso_placeholders("SELECT 1 FROM t WHERE a = 'x'"),
        "SELECT 1 FROM t WHERE a = 'x'",
        "a statement with no placeholder is unchanged"
    );
}

#[tokio::test]
async fn rewritten_placeholders_bind_by_number() {
    let conn = memory_connection().await;
    let sql = turso_placeholders("SELECT $2, $1, $2");
    assert_eq!(first_row(&conn, &sql).await, texts(&["b", "a", "b"]));
}

/// The upstream behavior the rewrite exists for. If turso starts binding `$N`
/// by number, this fails: the rewrite is then harmless but no longer needed,
/// and the module doc should say so.
#[tokio::test]
async fn turso_binds_raw_dollar_by_appearance() {
    let conn = memory_connection().await;
    assert_eq!(
        first_row(&conn, "SELECT $2, $1").await,
        texts(&["a", "b"]),
        "turso 0.8.1 binds $N by first appearance"
    );
}
