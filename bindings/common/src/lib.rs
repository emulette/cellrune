//! Shared native-session ownership and close semantics for language bindings.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

use cellrune_interop::{CancellationToken, InteropError, WorkbookSession};

/// Owns a workbook session shared by one language-binding object and its background tasks.
pub struct SharedWorkbookSession {
    closed: AtomicBool,
    session: Mutex<Option<WorkbookSession>>,
    cancellable_operations: Mutex<Vec<Arc<CancellationToken>>>,
}

impl SharedWorkbookSession {
    /// Wraps an open interop session.
    pub fn new(session: WorkbookSession) -> Self {
        Self {
            closed: AtomicBool::new(false),
            session: Mutex::new(Some(session)),
            cancellable_operations: Mutex::new(Vec::new()),
        }
    }

    /// Returns whether close has begun.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Atomically prevents new work, cancels active calculation, and releases the session.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let operations = match self.cancellable_operations.lock() {
            Ok(operations) => operations,
            Err(poisoned) => poisoned.into_inner(),
        };
        for cancellation in operations.iter() {
            cancellation.cancel();
        }
        drop(operations);
        let mut guard = match self.session.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(session) = guard.as_mut() {
            session.cancel_calculation();
        }
        drop(guard.take());
    }

    /// Registers cancellation that remains reachable while an operation runs off-lock.
    ///
    /// # Errors
    ///
    /// Returns the stable closed or unavailable interop error.
    pub fn cancellable_operation(&self) -> Result<CancellableOperation<'_>, InteropError> {
        self.require_open()?;
        let cancellation = Arc::new(CancellationToken::new());
        let mut operations = self
            .cancellable_operations
            .lock()
            .map_err(|_| InteropError::session_busy())?;
        self.require_open()?;
        operations.push(Arc::clone(&cancellation));
        Ok(CancellableOperation {
            owner: self,
            cancellation,
        })
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

/// One binding operation whose cancellation remains reachable without the workbook mutex.
pub struct CancellableOperation<'a> {
    owner: &'a SharedWorkbookSession,
    cancellation: Arc<CancellationToken>,
}

impl CancellableOperation<'_> {
    /// Returns the token passed through interop and core preparation/run work.
    pub fn token(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl Drop for CancellableOperation<'_> {
    fn drop(&mut self) {
        let mut operations = match self.owner.cancellable_operations.lock() {
            Ok(operations) => operations,
            Err(poisoned) => poisoned.into_inner(),
        };
        operations.retain(|candidate| !Arc::ptr_eq(candidate, &self.cancellation));
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
    use std::thread;
    use std::time::{Duration, Instant};

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

    #[test]
    fn close_cancels_registered_work_before_waiting_for_the_session_lock() {
        let shared = Arc::new(SharedWorkbookSession::new(WorkbookSession::create()));
        let operation = shared
            .cancellable_operation()
            .expect("new session accepts cancellable work");
        let guard = shared.lock().expect("new session is lockable");
        let closer = Arc::clone(&shared);
        let close_thread = thread::spawn(move || closer.close());

        let deadline = Instant::now() + Duration::from_secs(1);
        while !operation.token().is_cancelled() && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            operation.token().is_cancelled(),
            "close must reach preparation cancellation before the workbook mutex"
        );

        drop(guard);
        close_thread.join().expect("close thread completes");
        assert!(shared.is_closed());
    }

    #[test]
    fn closed_session_rejects_new_cancellable_work() {
        let shared = SharedWorkbookSession::new(WorkbookSession::create());
        shared.close();
        let error = match shared.cancellable_operation() {
            Ok(_) => panic!("closed session must reject cancellable work"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "interop.session.closed");
    }
}
