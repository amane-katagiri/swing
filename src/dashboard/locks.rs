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

#[derive(Default)]
pub(super) struct Locks {
    publish: Mutex<()>,
    mirror_writes: Mutex<()>,
    config_writes: Mutex<ConfigWriteState>,
    token_rotation: Mutex<()>,
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

    pub(super) async fn token_rotation(&self) -> MutexGuard<'_, ()> {
        self.token_rotation.lock().await
    }

    pub(super) async fn config_writes(&self) -> MutexGuard<'_, ConfigWriteState> {
        self.config_writes.lock().await
    }
}
