use std::fmt;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Default)]
pub struct CancellationToken {
    reason: Arc<OnceLock<&'static str>>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) -> bool {
        self.cancel_with_reason("cancellation_requested")
    }

    /// First cause wins across all token clones; later cleanup cannot overwrite it.
    pub fn cancel_with_reason(&self, reason: &'static str) -> bool {
        self.reason.set(reason).is_ok()
    }

    pub fn reason(&self) -> Option<&'static str> {
        self.reason.get().copied()
    }

    pub fn is_canceled(&self) -> bool {
        self.reason.get().is_some()
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("canceled", &self.is_canceled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn clones_observe_the_same_idempotent_cancel() {
        let token = CancellationToken::new();
        let clone = token.clone();

        assert!(token.cancel());
        assert!(!clone.cancel());
        assert!(clone.is_canceled());
    }

    #[test]
    fn concurrent_cancel_has_one_first_caller() {
        let token = CancellationToken::new();
        let handles = (0..8)
            .map(|_| {
                let token = token.clone();
                thread::spawn(move || token.cancel())
            })
            .collect::<Vec<_>>();

        let first_callers = handles
            .into_iter()
            .map(|handle| handle.join().expect("cancel thread"))
            .filter(|first| *first)
            .count();

        assert_eq!(first_callers, 1);
        assert!(token.is_canceled());
    }

    #[test]
    fn first_reason_survives_later_cleanup() {
        let token = CancellationToken::new();
        assert!(token.cancel_with_reason("scheduler_lease_lost"));
        assert!(!token.clone().cancel_with_reason("service_shutdown"));
        assert_eq!(token.reason(), Some("scheduler_lease_lost"));
    }

    #[test]
    fn debug_contains_only_state() {
        let token = CancellationToken::new();
        assert_eq!(
            format!("{token:?}"),
            "CancellationToken { canceled: false }"
        );
    }
}
