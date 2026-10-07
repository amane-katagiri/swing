use std::ops::{Deref, DerefMut};

use tokio::sync::{Mutex, MutexGuard};

#[derive(Default)]
pub(super) struct ConfigWriteState {
    pub(super) setup_done: bool,
}

pub(super) struct PublishLock<'a> {
    _guard: MutexGuard<'a, ()>,
}

pub(super) struct MirrorWrites<'a> {
    _guard: MutexGuard<'a, ()>,
}

pub(super) struct ConfigWrites<'a> {
    guard: MutexGuard<'a, ConfigWriteState>,
}

impl Deref for ConfigWrites<'_> {
    type Target = ConfigWriteState;

    fn deref(&self) -> &ConfigWriteState {
        &self.guard
    }
}

impl DerefMut for ConfigWrites<'_> {
    fn deref_mut(&mut self) -> &mut ConfigWriteState {
        &mut self.guard
    }
}

#[derive(Default)]
pub(super) struct Locks {
    publish: Mutex<()>,
    mirror_writes: Mutex<()>,
    config_writes: Mutex<ConfigWriteState>,
}

impl Locks {
    pub(super) fn try_publish(&self) -> Option<PublishLock<'_>> {
        self.publish
            .try_lock()
            .ok()
            .map(|guard| PublishLock { _guard: guard })
    }

    pub(super) async fn mirror_writes(&self) -> MirrorWrites<'_> {
        MirrorWrites {
            _guard: self.mirror_writes.lock().await,
        }
    }

    pub(super) async fn config_writes(&self) -> ConfigWrites<'_> {
        ConfigWrites {
            guard: self.config_writes.lock().await,
        }
    }
}
