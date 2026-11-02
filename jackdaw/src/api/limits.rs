//! Named limits on how many calls of a task run at once (see [`EngineBuilder::limit`](super::EngineBuilder::limit)).
//!
//! A limit is a semaphore. A call takes a permit before it starts and gives it back when it finishes, so a call that
//! waits holds no thread. The permit moves into the call itself: a blocking call cannot be stopped, so a cancelled
//! workflow keeps the permit taken until the call's thread is done.
use super::Error;
use super::task::NativeTask;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The limits an engine was built with.
#[derive(Default)]
pub(crate) struct Limits {
    named: HashMap<String, Arc<Semaphore>>,
    /// Applies to blocking tasks that name no limit of their own.
    blocking: Option<Arc<Semaphore>>,
}

/// The name the default limit for blocking tasks reports under.
const BLOCKING: &str = "blocking";

fn semaphore(name: &str, permits: usize) -> Result<Arc<Semaphore>, Error> {
    if permits == 0 || permits > Semaphore::MAX_PERMITS {
        return Err(Error::invalid(format!(
            "the limit `{name}` needs between 1 and {} permits, not {permits}",
            Semaphore::MAX_PERMITS
        )));
    }
    Ok(Arc::new(Semaphore::new(permits)))
}

impl Limits {
    pub(crate) fn new(named: Vec<(String, usize)>, blocking: Option<usize>) -> Result<Self, Error> {
        let mut limits = Self {
            named: HashMap::new(),
            blocking: blocking
                .map(|permits| semaphore(BLOCKING, permits))
                .transpose()?,
        };
        for (name, permits) in named {
            let semaphore = semaphore(&name, permits)?;
            limits.named.insert(name, semaphore);
        }
        Ok(limits)
    }

    /// Fails when `task` names a limit the engine does not define.
    pub(crate) fn check(&self, task: &NativeTask) -> Result<(), Error> {
        match task.limit {
            Some(name) if !self.named.contains_key(name) => Err(Error::invalid(format!(
                "task `{}` uses the limit `{name}`, which the engine does not define; add `.limit(\"{name}\", n)` to the builder",
                task.name
            ))),
            Some(_) | None => Ok(()),
        }
    }

    fn for_task(&self, task: &NativeTask) -> Option<(&str, &Arc<Semaphore>)> {
        match task.limit {
            Some(name) => self.named.get(name).map(|semaphore| (name, semaphore)),
            None if task.blocking => self
                .blocking
                .as_ref()
                .map(|semaphore| (BLOCKING, semaphore)),
            None => None,
        }
    }

    /// Waits for a permit to run `task`; `None` when no limit applies to it.
    pub(crate) async fn acquire(&self, task: &NativeTask) -> Option<OwnedSemaphorePermit> {
        let (name, semaphore) = self.for_task(task)?;
        if let Ok(permit) = Arc::clone(semaphore).try_acquire_owned() {
            return Some(permit);
        }
        tracing::debug!(
            task = task.name,
            limit = name,
            "the task waits for its limit"
        );
        let _waiting = super::observe::limit_waiting(name);
        // The semaphore is never closed, so this only ends with a permit.
        Arc::clone(semaphore).acquire_owned().await.ok()
    }
}
