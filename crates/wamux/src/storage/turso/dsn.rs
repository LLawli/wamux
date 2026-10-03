//! The `turso://<path>` DSN (#106), shaped like `sqlite://<path>` and stricter:
//! turso has no connection options, so a query string is a mistake to refuse,
//! not to ignore (an operator who wrote `?mode=ro` must not get a writable store).

use wacore::store::error::StoreError;

const PREFIX: &str = "turso://";

/// The file path of a `turso://<path>` DSN: `turso:///abs/x.db` is
/// `/abs/x.db`, `turso://x.db` is `x.db`. Anything else, a query string
/// included, is `InvalidConfig` quoting the DSN.
pub(crate) fn turso_path(database_url: &str) -> Result<&str, StoreError> {
    let refuse = |why: &str| {
        StoreError::InvalidConfig(format!(
            "invalid turso DSN '{database_url}': {why}; expected turso://<path>, \
             e.g. turso:///var/lib/wamux/wamux.db"
        ))
    };
    let path = database_url
        .strip_prefix(PREFIX)
        .ok_or_else(|| refuse("it does not start with turso://"))?;
    if path.contains('?') {
        return Err(refuse("turso takes no options, so no query string"));
    }
    if path.is_empty() || path == "/" {
        return Err(refuse("the path is empty"));
    }
    Ok(path)
}
