//! The `prefs` key/value table (SPEC 7).
//!
//! Small, typed accessors rather than a settings object: preferences are read
//! at the moment they are needed and written the moment they change, so a
//! crash can never lose more than the one value that was in flight.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

/// Bitmap cache budget in mebibytes (SPEC 9's "configurable").
pub const CACHE_BUDGET_MB: &str = "render.cache_budget_mb";

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM prefs WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO prefs (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Reads a preference that should hold a number, ignoring one that does not.
///
/// A corrupt value falls back to the caller's default instead of refusing to
/// start: a preferences table is not worth an unusable application.
pub fn get_u64(conn: &Connection, key: &str) -> Result<Option<u64>> {
    Ok(get(conn, key)?.and_then(|v| v.trim().parse::<u64>().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, Which};

    fn conn() -> Connection {
        open_memory(Which::App).expect("open")
    }

    #[test]
    fn a_value_round_trips() {
        let c = conn();
        assert_eq!(get(&c, CACHE_BUDGET_MB).expect("get"), None);
        set(&c, CACHE_BUDGET_MB, "512").expect("set");
        assert_eq!(get_u64(&c, CACHE_BUDGET_MB).expect("get"), Some(512));
    }

    #[test]
    fn writing_twice_updates_in_place() {
        let c = conn();
        set(&c, CACHE_BUDGET_MB, "512").expect("set");
        set(&c, CACHE_BUDGET_MB, "1024").expect("set");
        assert_eq!(get_u64(&c, CACHE_BUDGET_MB).expect("get"), Some(1024));
        let n: i64 = c
            .query_row("SELECT count(*) FROM prefs", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 1);
    }

    #[test]
    fn a_value_that_is_not_a_number_reads_as_absent() {
        let c = conn();
        set(&c, CACHE_BUDGET_MB, "banyak").expect("set");
        assert_eq!(get_u64(&c, CACHE_BUDGET_MB).expect("get"), None);
    }
}
