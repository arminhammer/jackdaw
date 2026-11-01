//! The built-in caches: in memory, Redb, SQLite and Postgres.
pub mod mem;
#[cfg(feature = "postgres")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres")))]
pub mod postgres;
#[cfg(feature = "redb")]
#[cfg_attr(docsrs, doc(cfg(feature = "redb")))]
pub mod redb;
#[cfg(feature = "sqlite")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite")))]
pub mod sqlite;

#[allow(unused_imports)]
#[cfg(feature = "postgres")]
pub use self::postgres::PostgresCache;
#[cfg(feature = "redb")]
pub use self::redb::RedbCache;
#[allow(unused_imports)]
#[cfg(feature = "sqlite")]
pub use self::sqlite::SqliteCache;
