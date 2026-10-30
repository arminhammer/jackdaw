//! The built-in caches: in memory, Redb, SQLite and Postgres.
pub mod mem;
pub mod postgres;
pub mod redb;
pub mod sqlite;

#[allow(unused_imports)]
pub use self::postgres::PostgresCache;
pub use self::redb::RedbCache;
#[allow(unused_imports)]
pub use self::sqlite::SqliteCache;
