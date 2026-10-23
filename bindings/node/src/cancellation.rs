//! Per-call cooperative cancellation that a JavaScript `AbortSignal` drives.

use cellrune_interop::CancellationToken;
use napi_derive::napi;

/// Cancels the one calculation it is passed to, through the session's cooperative cancellation.
#[napi]
#[derive(Default)]
pub struct NativeCancellation {
    token: CancellationToken,
}

#[napi]
impl NativeCancellation {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation of the calculation this handle was passed to.
    #[napi(catch_unwind)]
    pub fn cancel(&self) {
        self.token.cancel();
    }
}

/// Returns the token of an optional handle, or a token nothing cancels but the session.
pub(crate) fn token(cancellation: Option<&NativeCancellation>) -> CancellationToken {
    cancellation.map_or_else(CancellationToken::new, |handle| handle.token.clone())
}
