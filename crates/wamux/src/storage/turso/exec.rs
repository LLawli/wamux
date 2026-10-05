//! Running one statement on the turso connection (#106): the `$N` rewrite, the
//! bind list, and the cursor handling the crate demands.
//!
//! Facts from the 0.8.1 probe these helpers encode:
//! - a live, unfinished `Rows` keeps the connection busy, so every query here
//!   drains its cursor before returning, never a partial read;
//! - `execute` on a statement with `RETURNING` fails with
//!   `Misuse("unexpected row during execution")` BUT writes, so `RETURNING`
//!   goes through `fetch_*`;
//! - fewer binds than parameters become NULL silently, so call sites bind
//!   exactly what the statement names.

use ::turso::{Connection, Row, Value};
use wacore::store::error::Result as StoreResult;

use super::TursoConn;
use super::turso_error::db;
use super::turso_placeholders;

/// The positional values of one statement, in `$1..$N` order.
pub(super) type Binds = Vec<Value>;

/// `Binds` from a list of anything `turso::Value` converts from.
macro_rules! binds {
    ($($value:expr),* $(,)?) => {
        vec![$(::turso::Value::from($value)),*]
    };
}
pub(super) use binds;

/// Every row, the cursor drained. The raw `turso::Error` stays visible for the
/// callers that tell kinds of failure apart (checkpoint tolerates `Busy`).
pub(super) async fn fetch_all_raw(
    conn: &Connection,
    sql: &str,
    binds: Binds,
) -> Result<Vec<Row>, ::turso::Error> {
    let mut rows = conn.query(turso_placeholders(sql), binds).await?;
    let mut collected: Vec<Row> = Vec::new();
    while let Some(row) = rows.next().await? {
        collected.push(row);
    }
    Ok(collected)
}

pub(super) async fn fetch_all(conn: &Connection, sql: &str, binds: Binds) -> StoreResult<Vec<Row>> {
    fetch_all_raw(conn, sql, binds).await.map_err(db)
}

/// The first row, or `None`. The rest, if any, is drained and dropped.
pub(super) async fn fetch_optional(
    conn: &Connection,
    sql: &str,
    binds: Binds,
) -> StoreResult<Option<Row>> {
    Ok(fetch_all(conn, sql, binds).await?.into_iter().next())
}

/// Run a statement that returns no rows; the rows it changed.
pub(super) async fn execute(conn: &Connection, sql: &str, binds: Binds) -> StoreResult<u64> {
    conn.execute(turso_placeholders(sql), binds)
        .await
        .map_err(db)
}

/// One statement on the shared connection: lock, run, unlock. A multi-statement
/// operation uses a `TursoTx` instead, which holds the lock throughout.
impl TursoConn {
    pub(super) async fn execute(&self, sql: &str, binds: Binds) -> StoreResult<u64> {
        execute(&*self.lock().await, sql, binds).await
    }

    pub(super) async fn fetch_all(&self, sql: &str, binds: Binds) -> StoreResult<Vec<Row>> {
        fetch_all(&*self.lock().await, sql, binds).await
    }

    pub(super) async fn fetch_optional(&self, sql: &str, binds: Binds) -> StoreResult<Option<Row>> {
        fetch_optional(&*self.lock().await, sql, binds).await
    }
}

/// The binds of a fixed-size `IN` query (#104): the leading values, then one
/// per list element.
pub(super) fn with_list(leading: Binds, list: impl IntoIterator<Item = Value>) -> Binds {
    let mut binds = leading;
    binds.extend(list);
    binds
}
