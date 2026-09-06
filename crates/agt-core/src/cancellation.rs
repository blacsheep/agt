use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
pub struct CancellationToken {
    canceled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) -> bool {
        !self.canceled.swap(true, Ordering::AcqRel)
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
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
    fn debug_contains_only_state() {
        let token = CancellationToken::new();
        assert_eq!(
            format!("{token:?}"),
            "CancellationToken { canceled: false }"
        );
    }
}
