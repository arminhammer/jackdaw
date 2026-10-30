// The curated set is reachable from the crate root, without knowing the module layout.
use jackdaw::{CacheProvider, Engine, EngineBuilder, Error, ExecutionHandle, Executor, PersistenceProvider};

fn names(
    _engine: Option<Engine>,
    _builder: Option<EngineBuilder>,
    _handle: Option<ExecutionHandle<()>>,
    _store: Option<&dyn PersistenceProvider>,
    _cache: Option<&dyn CacheProvider>,
    _executor: Option<&dyn Executor>,
    _error: Option<Error>,
) {
}

fn main() {
    let _ = names;
}
