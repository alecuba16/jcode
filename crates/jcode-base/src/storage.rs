#![cfg_attr(test, allow(clippy::items_after_test_module))]

pub use jcode_storage::*;

use anyhow::Result;
use serde::de::DeserializeOwned;
use std::path::Path;

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    jcode_storage::read_json_with_recovery_handler(path, |event| match event {
        jcode_storage::StorageRecoveryEvent::CorruptPrimary { path, error } => {
            crate::logging::warn(&format!(
                "Corrupt JSON at {}, trying backup: {}",
                path.display(),
                error
            ));
        }
        jcode_storage::StorageRecoveryEvent::RecoveredFromBackup { backup_path } => {
            crate::logging::info(&format!("Recovered from backup: {}", backup_path.display()));
        }
    })
}

#[cfg(any(test, feature = "test-support"))]
use std::sync::{Mutex, MutexGuard, OnceLock};

#[cfg(any(test, feature = "test-support"))]
pub fn test_env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// The real mutex guard is parked here while any same-thread nesting is
    /// alive, so the lock stays held until the last `TestEnvGuard` drops,
    /// regardless of drop order.
    static TEST_ENV_LOCK_GUARD: std::cell::RefCell<Option<MutexGuard<'static, ()>>> =
        const { std::cell::RefCell::new(None) };
    /// Same-thread `lock_test_env` nesting depth. Zero means this thread does
    /// not hold the lock.
    static TEST_ENV_LOCK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Reentrant guard for the shared test env lock.
///
/// Same-thread nesting parks the single real guard in a thread-local and
/// releases it only when the last same-thread guard drops, so out-of-order
/// drops cannot leave shared environment values unprotected. Cross-thread
/// callers still serialize on the same mutex. Guards must stay on the
/// acquiring thread (`!Send`), matching `std::sync::MutexGuard`.
#[cfg(any(test, feature = "test-support"))]
pub struct TestEnvGuard {
    /// Keeps this guard `!Send`, like the `MutexGuard` it replaces.
    _not_send: std::marker::PhantomData<*mut ()>,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        let release = TEST_ENV_LOCK_DEPTH.with(|depth| {
            let next = depth.get().saturating_sub(1);
            depth.set(next);
            next == 0
        });
        if release {
            TEST_ENV_LOCK_GUARD.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn lock_test_env() -> TestEnvGuard {
    TEST_ENV_LOCK_DEPTH.with(|depth| {
        if depth.get() == 0 {
            let guard = test_env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            TEST_ENV_LOCK_GUARD.with(|slot| *slot.borrow_mut() = Some(guard));
        }
        depth.set(depth.get() + 1);
    });
    install_shared_test_home_once();
    TestEnvGuard {
        _not_send: std::marker::PhantomData,
    }
}

/// Install the shared test home the first time any test takes the env lock,
/// before that test's body can run. After this point `JCODE_HOME` is always
/// set: scoped tests capture it as their previous value and restore it on
/// drop, so the variable never returns to the unset gap that racing
/// [`ensure_shared_test_jcode_home_if_unset`] calls could observe mid-body of
/// a victim test (the session-picker cache flake, #1784). The install shares
/// the install lock with the unlocked helper, so the two entry points cannot
/// interleave an unset check with a set that lands after a locked test
/// scoped its own home.
#[cfg(any(test, feature = "test-support"))]
fn install_shared_test_home_once() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        install_shared_test_home_if_unset();
    });
}

#[cfg(any(test, feature = "test-support"))]
fn install_shared_test_home_if_unset() {
    static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    if std::env::var_os("JCODE_HOME").is_some() {
        return;
    }
    let _install = INSTALL_LOCK
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if std::env::var_os("JCODE_HOME").is_none() {
        crate::env::set_var("JCODE_HOME", shared_test_jcode_home());
    }
}

/// Point JCODE_HOME at a per-process temp dir when the environment does not
/// already pin one, so tests never read the developer's real `~/.jcode` state.
/// Runs without the shared env lock; both entry points take the install lock
/// and re-check the variable inside it.
#[cfg(any(test, feature = "test-support"))]
pub fn ensure_shared_test_jcode_home_if_unset() {
    install_shared_test_home_if_unset();
}

/// The shared per-process test home that [`ensure_shared_test_jcode_home_if_unset`]
/// installs when no test has scoped its own `JCODE_HOME`.
#[cfg(any(test, feature = "test-support"))]
pub fn shared_test_jcode_home() -> &'static std::path::Path {
    static TEST_HOME: OnceLock<std::path::PathBuf> = OnceLock::new();
    TEST_HOME.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("jcode-test-home-{}", std::process::id()));
        if let Err(err) = std::fs::create_dir_all(&path) {
            crate::logging::debug(&format!(
                "failed to create shared test home {}: {}",
                path.display(),
                err
            ));
        }
        path
    })
}

#[cfg(test)]
mod tests;
