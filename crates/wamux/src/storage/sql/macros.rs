//! The macros that let every statement be written once (#65).
//!
//! sqlx's `query(..).bind(..).execute(pool)` chain is generic over the driver,
//! but a `SqlPool` holds one concrete pool per driver, so the chain has to be
//! spelled in a `match`. `on_sql_pool!` expands the SAME body in each arm: the
//! source has one copy, the compiler monomorphizes it per driver. Placeholders
//! are always `$N`: sqlx-sqlite binds them by number (repeated and out of order
//! included, pinned by the `existing_store` suite), so one string runs on both.
//!
//! The arms must return one type, and the drivers' result types differ
//! (`PgQueryResult`, `SqliteQueryResult`), so bodies convert before returning
//! (`rows_affected()`, commit inside the body).

/// Run `$body` once per driver, with `$conn` bound to that driver's pool.
macro_rules! on_sql_pool {
    ($pool:expr, |$conn:ident| $body:expr) => {
        match $pool {
            $crate::storage::sql::SqlPool::Pg($conn) => $body,
            $crate::storage::sql::SqlPool::Sqlite($conn) => $body,
        }
    };
}

/// Same, for an open `SqlTx`: `$conn` is that driver's `&mut` connection.
macro_rules! on_sql_tx {
    ($tx:expr, |$conn:ident| $body:expr) => {
        match &mut $tx {
            $crate::storage::sql::SqlTx::Pg(inner) => {
                let $conn = &mut **inner;
                $body
            }
            $crate::storage::sql::SqlTx::Sqlite(inner) => {
                let $conn = &mut **inner;
                $body
            }
        }
    };
}

/// Run one statement and return the rows affected. `in tx` runs it inside an
/// open `SqlTx`, otherwise on the pool.
macro_rules! execute_sql {
    (in $tx:ident, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_tx!($tx, |conn| {
            sqlx::query($sql)$(.bind($bind))*.execute(conn).await.map(|done| done.rows_affected())
        })
        .map_err($crate::storage::sqlx_error::db)
    };
    ($pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_pool!($pool, |conn| {
            sqlx::query($sql)$(.bind($bind))*.execute(conn).await.map(|done| done.rows_affected())
        })
        .map_err($crate::storage::sqlx_error::db)
    };
}

/// First column of the first row, or `None`.
macro_rules! scalar_optional_sql {
    ($ty:ty, in $tx:ident, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_tx!($tx, |conn| {
            sqlx::query_scalar::<_, $ty>($sql)$(.bind($bind))*.fetch_optional(conn).await
        })
        .map_err($crate::storage::sqlx_error::db)
    };
    ($ty:ty, $pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_pool!($pool, |conn| {
            sqlx::query_scalar::<_, $ty>($sql)$(.bind($bind))*.fetch_optional(conn).await
        })
        .map_err($crate::storage::sqlx_error::db)
    };
}

/// First column of exactly one row.
macro_rules! scalar_one_sql {
    ($ty:ty, $pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_pool!($pool, |conn| {
            sqlx::query_scalar::<_, $ty>($sql)$(.bind($bind))*.fetch_one(conn).await
        })
        .map_err($crate::storage::sqlx_error::db)
    };
}

/// First column of every row.
macro_rules! scalar_all_sql {
    ($ty:ty, $pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_pool!($pool, |conn| {
            sqlx::query_scalar::<_, $ty>($sql)$(.bind($bind))*.fetch_all(conn).await
        })
        .map_err($crate::storage::sqlx_error::db)
    };
}

/// One row decoded as `$ty` (a tuple or a `FromRow` type), or `None`.
macro_rules! row_optional_sql {
    ($ty:ty, $pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_pool!($pool, |conn| {
            sqlx::query_as::<_, $ty>($sql)$(.bind($bind))*.fetch_optional(conn).await
        })
        .map_err($crate::storage::sqlx_error::db)
    };
}

/// Every row decoded as `$ty`.
macro_rules! row_all_sql {
    ($ty:ty, $pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {
        on_sql_pool!($pool, |conn| {
            sqlx::query_as::<_, $ty>($sql)$(.bind($bind))*.fetch_all(conn).await
        })
        .map_err($crate::storage::sqlx_error::db)
    };
}

/// Every row decoded as `$ty`, for a statement whose last placeholders are one
/// fixed-size `IN` list (#104). `$bind`s go first, then each of `$values`.
macro_rules! rows_in_list_sql {
    ($ty:ty, $pool:expr, $sql:expr, [$($bind:expr),*], $values:expr) => {
        on_sql_pool!($pool, |conn| {
            let mut query = sqlx::query_as::<_, $ty>($sql)$(.bind($bind))*;
            for value in $values {
                query = query.bind(value);
            }
            query.fetch_all(conn).await
        })
        .map_err($crate::storage::sqlx_error::db)
    };
}
