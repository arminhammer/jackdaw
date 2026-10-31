//! The built-in stores: in memory, Redb, SQLite and Postgres. All of them support leases.
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

pub use self::mem::InMemoryPersistence;
#[allow(unused_imports)]
#[cfg(feature = "postgres")]
pub use self::postgres::PostgresPersistence;
#[cfg(feature = "redb")]
pub use self::redb::RedbPersistence;
#[allow(unused_imports)]
#[cfg(feature = "sqlite")]
pub use self::sqlite::SqlitePersistence;
