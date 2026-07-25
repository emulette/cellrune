//! Shared native-session ownership and close semantics for language bindings.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

use cellrune_interop::{InteropError, WorkbookSession};

/// Owns a workbook session shared by one language-binding object and its background tasks.
pub struct SharedWorkbookSession {
    closed: AtomicBool,
    session: Mutex<Option<WorkbookSession>>,
}

impl SharedWorkbookSession {
    /// Wraps an open interop session.
    pub fn new(session: WorkbookSession) -> Self {
        Self {
            closed: AtomicBool::new(false),
            session: Mutex::new(Some(session)),
        }
    }

    /// Returns whether close has begun.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Atomically prevents new work, cancels active calculation, and releases the session.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let mut guard = match self.session.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(session) = guard.as_mut() {
            session.cancel_calculation();
        }
        drop(guard.take());
    }

    /// Acquires an open session without waiting for concurrent work.
    ///
    /// # Errors
    ///
    /// Returns the stable closed or unavailable interop error.
    pub fn try_lock(&self) -> Result<WorkbookSessionGuard<'_>, InteropError> {
        self.require_open()?;
        let guard = match self.session.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {
                return Err(InteropError::session_busy());
            }
        };
        self.open_guard(guard)
    }

    /// Acquires an open session, waiting for concurrent work to finish.
    ///
    /// # Errors
    ///
    /// Returns the stable closed or unavailable interop error.
    pub fn lock(&self) -> Result<WorkbookSessionGuard<'_>, InteropError> {
        self.require_open()?;
        let guard = self
            .session
            .lock()
            .map_err(|_| InteropError::session_busy())?;
        self.open_guard(guard)
    }

    fn require_open(&self) -> Result<(), InteropError> {
        if self.is_closed() {
            Err(InteropError::session_closed())
        } else {
            Ok(())
        }
    }

    fn open_guard<'a>(
        &'a self,
        guard: MutexGuard<'a, Option<WorkbookSession>>,
    ) -> Result<WorkbookSessionGuard<'a>, InteropError> {
        self.require_open()?;
        if guard.is_none() {
            return Err(InteropError::session_closed());
        }
        Ok(WorkbookSessionGuard { guard })
    }
}

/// Locked access to an open workbook session.
pub struct WorkbookSessionGuard<'a> {
    guard: MutexGuard<'a, Option<WorkbookSession>>,
}

impl Deref for WorkbookSessionGuard<'_> {
    type Target = WorkbookSession;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("open session guard must contain a workbook session")
    }
}

impl DerefMut for WorkbookSessionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("open session guard must contain a workbook session")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_access_uses_stable_unavailable_error() {
        let shared = SharedWorkbookSession::new(WorkbookSession::create());
        let guard = shared.lock().expect("new session must be open");
        let error = match shared.try_lock() {
            Ok(_) => panic!("a second non-blocking lock must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "interop.session.unavailable");
        drop(guard);
        assert!(shared.try_lock().is_ok());
    }

    #[test]
    fn close_is_idempotent_and_permanently_releases_access() {
        let shared = SharedWorkbookSession::new(WorkbookSession::create());
        assert!(!shared.is_closed());

        shared.close();
        shared.close();

        assert!(shared.is_closed());
        let error = match shared.lock() {
            Ok(_) => panic!("a closed session must not be recoverable"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "interop.session.closed");
    }
}
