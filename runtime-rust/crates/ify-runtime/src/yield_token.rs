//! Cooperative cancellation and yield primitive.
//!
//! [`YieldToken`] is a lightweight handle that every long-running async task
//! should accept and poll at safe suspension points.  It serves two purposes:
//!
//! 1. **Cancellation** — the caller sets the token; the task detects it and
//!    returns [`YieldError::Cancelled`].
//! 2. **Cooperative yield** — calling [`YieldToken::yield_now`] suspends the
//!    current async task for one scheduler tick, preventing CPU starvation of
//!    other concurrent tasks.
//!
//! ## Usage
//!
//! ```rust
//! use ify_runtime::yield_token::YieldToken;
//!
//! async fn long_computation(token: YieldToken) -> Result<u64, ify_runtime::yield_token::YieldError> {
//!     let mut acc = 0u64;
//!     for i in 0..1_000_000u64 {
//!         token.check_cancelled()?;          // fast path — no await needed
//!         if i % 1_000 == 0 {
//!             token.yield_now().await?;       // full cooperative yield
//!         }
//!         acc += i;
//!     }
//!     Ok(acc)
//! }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use thiserror::Error;

/// Errors produced by yield-token operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum YieldError {
    /// The token was cancelled; the task should return as soon as possible.
    #[error("task cancelled via YieldToken")]
    Cancelled,
}

/// Inner shared state for a `YieldToken` pair.
struct Inner {
    cancelled: AtomicBool,
}

/// Caller-side handle for signalling cancellation.
///
/// Create one [`YieldTokenSource`] per task and pass the derived
/// [`YieldToken`] into the task.
pub struct YieldTokenSource {
    inner: Arc<Inner>,
}

impl YieldTokenSource {
    /// Create a new source/token pair.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                cancelled: AtomicBool::new(false),
            }),
        }
    }

    /// Derive the [`YieldToken`] to hand into the task.
    pub fn token(&self) -> YieldToken {
        YieldToken {
            inner: self.inner.clone(),
        }
    }

    /// Signal cancellation.  Any subsequent call to [`YieldToken::check_cancelled`]
    /// or [`YieldToken::yield_now`] will return [`YieldError::Cancelled`].
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }

    /// Return `true` if cancellation has been signalled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }
}

impl Default for YieldTokenSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Task-side cooperative cancellation and yield handle.
///
/// Obtained via [`YieldTokenSource::token`].  Cheap to clone — all clones
/// share the same cancellation state.
#[derive(Clone)]
pub struct YieldToken {
    inner: Arc<Inner>,
}

impl YieldToken {
    /// Create an already-cancelled token (useful in tests).
    pub fn cancelled() -> Self {
        let src = YieldTokenSource::new();
        src.cancel();
        src.token()
    }

    /// Create a token that will never be cancelled (useful in tests / benchmarks
    /// where cancellation is not required).
    pub fn never_cancelled() -> Self {
        YieldTokenSource::new().token()
    }

    /// Check for cancellation **without** yielding to the scheduler.
    ///
    /// This is the cheapest check — suitable for tight inner loops.
    ///
    /// # Errors
    ///
    /// Returns [`YieldError::Cancelled`] if cancellation has been signalled.
    #[inline]
    pub fn check_cancelled(&self) -> Result<(), YieldError> {
        if self.inner.cancelled.load(Ordering::Acquire) {
            Err(YieldError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Yield to the async scheduler for one tick, then check cancellation.
    ///
    /// Use at natural suspension points in compute-heavy async tasks to allow
    /// other tasks to make progress without busy-waiting.
    ///
    /// # Errors
    ///
    /// Returns [`YieldError::Cancelled`] if cancellation has been signalled
    /// either before or after the yield.
    pub async fn yield_now(&self) -> Result<(), YieldError> {
        self.check_cancelled()?;
        tokio::task::yield_now().await;
        self.check_cancelled()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_cancelled_by_default() {
        let src = YieldTokenSource::new();
        let token = src.token();
        assert!(token.check_cancelled().is_ok());
    }

    #[test]
    fn cancel_propagates() {
        let src = YieldTokenSource::new();
        let token = src.token();
        src.cancel();
        assert!(matches!(token.check_cancelled(), Err(YieldError::Cancelled)));
    }

    #[test]
    fn clone_shares_state() {
        let src = YieldTokenSource::new();
        let t1 = src.token();
        let t2 = t1.clone();
        src.cancel();
        assert!(matches!(t1.check_cancelled(), Err(YieldError::Cancelled)));
        assert!(matches!(t2.check_cancelled(), Err(YieldError::Cancelled)));
    }

    #[test]
    fn never_cancelled_token() {
        let token = YieldToken::never_cancelled();
        assert!(token.check_cancelled().is_ok());
    }

    #[test]
    fn pre_cancelled_token() {
        let token = YieldToken::cancelled();
        assert!(matches!(token.check_cancelled(), Err(YieldError::Cancelled)));
    }

    #[tokio::test]
    async fn yield_now_detects_cancel() {
        let src = YieldTokenSource::new();
        let token = src.token();
        src.cancel();
        let result = token.yield_now().await;
        assert!(matches!(result, Err(YieldError::Cancelled)));
    }

    #[tokio::test]
    async fn yield_now_ok_when_not_cancelled() {
        let token = YieldToken::never_cancelled();
        assert!(token.yield_now().await.is_ok());
    }
}
