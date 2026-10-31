//! A store of your own: implement `PersistenceProvider` (four methods; the rest have defaults) and hand it to the
//! builder. This one keeps everything in maps; a real one would write to your database.
//!
//! ```text
//! cargo run -p jackdaw-wf --example custom_persistence
//! ```
use async_trait::async_trait;
use jackdaw::persistence::{PersistenceProvider, Result as StoreResult};
use jackdaw::prelude::*;
use jackdaw::workflow::{WorkflowCheckpoint, WorkflowEvent};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Default)]
struct MapStore {
    events: Mutex<HashMap<String, Vec<WorkflowEvent>>>,
    checkpoints: Mutex<HashMap<String, WorkflowCheckpoint>>,
}

#[async_trait]
impl PersistenceProvider for MapStore {
    async fn save_event(&self, event: WorkflowEvent) -> StoreResult<()> {
        lock(&self.events)
            .entry(event.instance_id().to_string())
            .or_default()
            .push(event);
        Ok(())
    }

    async fn get_events(&self, instance_id: &str) -> StoreResult<Vec<WorkflowEvent>> {
        Ok(lock(&self.events)
            .get(instance_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn save_checkpoint(&self, checkpoint: WorkflowCheckpoint) -> StoreResult<()> {
        lock(&self.checkpoints).insert(checkpoint.instance_id.clone(), checkpoint);
        Ok(())
    }

    async fn get_checkpoint(&self, instance_id: &str) -> StoreResult<Option<WorkflowCheckpoint>> {
        Ok(lock(&self.checkpoints).get(instance_id).cloned())
    }

    // Engines that share one store must not start the same instance twice. A store meant for that overrides
    // `create_checkpoint` with an atomic "insert if absent"; this one is used by a single engine.
}

#[task]
async fn double(n: u64) -> Result<u64, TaskError> {
    Ok(n * 2)
}

#[workflow(namespace = "examples", name = "doubles", version = "1.0.0")]
pub struct Doubles;

#[workflow_methods]
impl Doubles {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, n: u64) -> WorkflowResult<u64> {
        let once = ctx.call(Double, n).await?;
        ctx.call(Double, once).await
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let store = Arc::new(MapStore::default());
    let engine = Engine::builder()
        .persistence(Arc::clone(&store) as Arc<dyn PersistenceProvider>)
        .task(Double)
        .workflow::<Doubles>()
        .build()?;

    let run = engine.execute(Doubles::run, 21).await?;
    let id = run.id().to_string();
    let answer = run.await?;

    let events = store
        .get_events(&id)
        .await
        .map_err(|e| Error::Engine(e.to_string()))?;
    println!("{answer} after {} stored events", events.len());
    assert_eq!(answer, 84);
    assert!(
        store
            .get_checkpoint(&id)
            .await
            .map_err(|e| Error::Engine(e.to_string()))?
            .is_some()
    );
    Ok(())
}
