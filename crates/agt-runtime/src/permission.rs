//! Native-only permission stages. Policies cannot execute tools or bypass service authorization.
use crate::approval::{ApprovalState, ApprovalStore, DecisionOrigin, Delivery, Scope};
use crate::engine::check_canceled;
use crate::{Result, RuntimeError};
use agt_config::runtime::{ConfigStamp, PermissionConfig, PermissionMode, RuntimeConfig};
use agt_core::CancellationToken;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
mod model_review;
mod policy;
use policy::{Decision, PermissionPolicy};

/// Native requests wait on wake hints; this fallback bounds lease renewal, expiry and
/// review-timeout checks when no hint arrives.
const RECHECK: Duration = Duration::from_secs(1);
/// Cancellation is in-memory and checked at this cadence without touching the database.
const TICK: Duration = Duration::from_millis(100);

/// Holds the workspace permission snapshot for one Run. The snapshot is reloaded only
/// when `config.yaml` changes, so every request still applies the current policy.
pub(crate) struct PermissionLoop {
    home: PathBuf,
    config: PermissionConfig,
    stamp: Option<ConfigStamp>,
    reviewer: Option<crate::ResolvedAgent>,
}
impl PermissionLoop {
    pub(crate) fn load(home: &Path) -> Result<Self> {
        let stamp = RuntimeConfig::stamp(home);
        let config = load_config(home)?;
        Ok(Self {
            reviewer: model_review::load(home, &config).ok(),
            home: home.into(),
            config,
            stamp,
        })
    }
    pub(crate) fn config(&self) -> &PermissionConfig {
        &self.config
    }
    pub(crate) fn reviewer_fingerprint(&self) -> Option<&str> {
        self.reviewer.as_ref().map(|r| r.fingerprint.as_str())
    }
    /// Reload after the saved configuration changed. Returns whether the policy differs.
    fn refresh(&mut self) -> Result<bool> {
        let stamp = RuntimeConfig::stamp(&self.home);
        if stamp == self.stamp {
            return Ok(false);
        }
        let config = load_config(&self.home)?;
        self.stamp = stamp;
        if config == self.config {
            return Ok(false);
        }
        self.config = config;
        self.reviewer = model_review::load(&self.home, &self.config).ok();
        Ok(true)
    }
    /// Returns whether the call may run, and whether the policy snapshot changed so the
    /// caller can refresh its execution fingerprint.
    pub(crate) fn authorize(
        &mut self,
        request: Request<'_>,
        cancellation: &CancellationToken,
        fence: &dyn Fn() -> Result<()>,
    ) -> Result<(bool, bool)> {
        let mut changed = self.refresh()?;
        if self.config.mode == PermissionMode::ModelReview {
            // Edits to the reviewer Model resource apply to the next request.
            let reviewer = model_review::load(&self.home, &self.config).ok();
            changed |= reviewer.as_ref().map(|r| &r.fingerprint)
                != self.reviewer.as_ref().map(|r| &r.fingerprint);
            self.reviewer = reviewer;
        }
        let allowed = self
            .run(request, cancellation, fence, &mut changed)
            .map_err(|e| {
                e.downcast::<RuntimeError>().unwrap_or_else(|e| {
                    RuntimeError::Tool(crate::Reason::because(
                        "permission request failed; inspect approval records",
                        e,
                    ))
                })
            })?;
        Ok((allowed, changed))
    }
    fn start_policy(
        &self,
        store: &ApprovalStore,
        id: &str,
        context: Value,
    ) -> anyhow::Result<Box<dyn PermissionPolicy>> {
        store.native_policy(id, &self.config, self.reviewer_fingerprint())?;
        Ok(match self.config.mode {
            PermissionMode::Manual => Box::new(policy::Manual),
            PermissionMode::AutoAllow => Box::new(policy::AutoAllow),
            PermissionMode::ModelReview => Box::new(model_review::ModelReview::begin(
                &self.home,
                self.reviewer.clone(),
                &self.config,
                context,
            )),
        })
    }
    fn run(
        &mut self,
        request: Request<'_>,
        cancellation: &CancellationToken,
        fence: &dyn Fn() -> Result<()>,
        changed: &mut bool,
    ) -> anyhow::Result<bool> {
        let check = || -> Result<()> {
            check_canceled(cancellation)?;
            fence()
        };
        check()?;
        // Subscribe before creating the request so no decision hint is missed.
        let mut listener = agt_storage::wake::Listener::open(&self.home).ok();
        let store = ApprovalStore::open(&self.home)?;
        let Request {
            scope,
            call,
            tool,
            arguments: args,
            input,
        } = request;
        let request = json!({"tool":tool,"arguments":args,"policy":self.config,"reviewer_fingerprint":self.reviewer_fingerprint(),"options":[{"label":"批准本次调用","value":"allow"},{"label":"拒绝本次调用","value":"deny"},{"label":"取消执行","value":"cancel"}]});
        let item = store.create(
            scope,
            crate::approval::ApprovalSource::Native,
            call,
            request,
            u64::from(self.config.request_timeout_seconds),
        )?;
        struct Guard<'a>(&'a ApprovalStore, &'a str);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                let _ = self.0.invalidate_run(self.1);
            }
        }
        let _guard = Guard(&store, &scope.run_id);
        let context = json!({"tool":tool,"arguments":args,"task":input});
        let mut plugin = self.start_policy(&store, &item.id, context.clone())?;
        let mut hinted = true;
        let mut checked = Instant::now();
        loop {
            check_canceled(cancellation)?;
            if !hinted && checked.elapsed() < RECHECK {
                hinted = wait_hint(&mut listener);
                continue;
            }
            checked = Instant::now();
            check()?;
            let current = store.poll(&item.id)?;
            if current.delivery == Delivery::Invalidated
                || matches!(
                    current.state,
                    ApprovalState::Expired | ApprovalState::Canceled
                ) && current.decision.is_none()
            {
                return Err(RuntimeError::Stopped(crate::StopCode::CancellationRequested).into());
            }
            if let Some(decision) = current.decision {
                plugin.cancel();
                check()?;
                if decision == "cancel" {
                    return Err(
                        RuntimeError::Stopped(crate::StopCode::CancellationRequested).into(),
                    );
                }
                store.change(&item.id, None, Some(Delivery::Resolved))?;
                return Ok(decision == "allow");
            }
            let previous = self.config.clone();
            if self.refresh()? {
                *changed = true;
                if self.config.mode != previous.mode
                    || self.config.reviewer_model != previous.reviewer_model
                    || self.config.review_timeout_seconds != previous.review_timeout_seconds
                {
                    plugin.cancel();
                    plugin = self.start_policy(&store, &item.id, context.clone())?;
                    hinted = true;
                    continue;
                }
            }
            if current.state == ApprovalState::Reviewing
                || self.config.mode == PermissionMode::AutoAllow
            {
                match plugin.poll() {
                    decision @ (Decision::Allow(_) | Decision::Deny(_)) => {
                        let allow = matches!(&decision, Decision::Allow(_));
                        let (Decision::Allow(reason) | Decision::Deny(reason)) = decision else {
                            unreachable!()
                        };
                        if store
                            .decide_as(
                                &item.id,
                                current.revision,
                                json!(if allow { "allow" } else { "deny" }),
                                &reason,
                                None,
                                Some(if self.config.mode == PermissionMode::AutoAllow {
                                    DecisionOrigin::AutoAllow
                                } else {
                                    DecisionOrigin::ModelReview
                                }),
                            )
                            .is_err()
                        {
                            // Human decisions can win while the model is finishing.
                            if store.poll(&item.id)?.decision.is_none() {
                                return Err(anyhow::anyhow!("cannot save permission decision"));
                            }
                        }
                    }
                    Decision::Human(reason) => {
                        store.fallback(&item.id, current.revision, &reason)?;
                    }
                    Decision::Pending => {}
                }
            }
            hinted = wait_hint(&mut listener);
        }
    }
}

pub(crate) struct Request<'a> {
    pub scope: &'a Scope,
    pub call: &'a str,
    pub tool: &'a str,
    pub arguments: &'a Value,
    pub input: &'a str,
}

// Wake hints are optional; storage polling, expiry and cancellation still work
// when the socket cannot be created or later fails. Sleep to avoid a busy loop.
fn wait_hint(listener: &mut Option<agt_storage::wake::Listener>) -> bool {
    if let Some(active) = listener {
        match active.wait(TICK) {
            Ok(hinted) => return hinted,
            Err(_) => *listener = None,
        }
    }
    std::thread::sleep(TICK);
    false
}

fn load_config(home: &Path) -> Result<PermissionConfig> {
    Ok(RuntimeConfig::load(home)
        .map_err(|e| {
            RuntimeError::Configuration(crate::Reason::because("cannot load permission policy", e))
        })?
        .permissions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn home(mode: PermissionMode) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        agt_config::initialize_home(Some(home.path().into())).unwrap();
        let mut config = RuntimeConfig::load(home.path()).unwrap();
        config.permissions.mode = mode;
        config.save(home.path()).unwrap();
        home
    }

    #[cfg(unix)]
    fn without_wake(mode: PermissionMode) -> (tempfile::TempDir, Scope) {
        let home = home(mode);
        let (record, _) = crate::RunStore::open(home.path())
            .unwrap()
            .claim(&crate::RunRequest::new("a", "task"), None)
            .unwrap();
        // Deterministic socket failure without changing directory permissions.
        std::fs::write(home.path().join("runtime/wake"), b"unavailable").unwrap();
        assert!(agt_storage::wake::Listener::open(home.path()).is_err());
        let scope = Scope::new(&record, None);
        (home, scope)
    }

    #[cfg(unix)]
    fn request(scope: &Scope) -> Request<'_> {
        Request {
            scope,
            call: "call-1",
            tool: "framework_message_send",
            arguments: &Value::Null,
            input: "task",
        }
    }

    #[cfg(unix)]
    #[test]
    fn auto_allow_works_without_wake_socket() {
        let (home, scope) = without_wake(PermissionMode::AutoAllow);
        let (allowed, _) = PermissionLoop::load(home.path())
            .unwrap()
            .authorize(request(&scope), &CancellationToken::new(), &|| Ok(()))
            .unwrap();
        assert!(allowed);
        let item = ApprovalStore::open(home.path())
            .unwrap()
            .list(None, None)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(item.delivery, Delivery::Resolved);
    }

    #[cfg(unix)]
    #[test]
    fn manual_decisions_work_without_wake_socket() {
        for allow in [false, true] {
            let (home, scope) = without_wake(PermissionMode::Manual);
            let path = home.path().to_owned();
            let human = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let store = ApprovalStore::open(&path).unwrap();
                    if let Some(item) = store.list(None, None).unwrap().pop() {
                        store
                            .decide(
                                &item.id,
                                item.revision,
                                json!(if allow { "allow" } else { "deny" }),
                                "human",
                                None,
                            )
                            .unwrap();
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(20));
                }
            });
            let (allowed, _) = PermissionLoop::load(home.path())
                .unwrap()
                .authorize(request(&scope), &CancellationToken::new(), &|| Ok(()))
                .unwrap();
            human.join().unwrap();
            assert_eq!(allowed, allow);
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_and_expiry_work_without_wake_socket() {
        for cancel in [false, true] {
            let (home, scope) = without_wake(PermissionMode::Manual);
            let mut config = RuntimeConfig::load(home.path()).unwrap();
            config.permissions.request_timeout_seconds = if cancel { 30 } else { 1 };
            config.save(home.path()).unwrap();
            let token = CancellationToken::new();
            let child = token.clone();
            let handle = std::thread::spawn(move || {
                if cancel {
                    std::thread::sleep(Duration::from_millis(150));
                    child.cancel();
                }
            });
            let started = Instant::now();
            let result = PermissionLoop::load(home.path()).unwrap().authorize(
                request(&scope),
                &token,
                &|| Ok(()),
            );
            handle.join().unwrap();
            assert!(matches!(result, Err(RuntimeError::Stopped(_))));
            if cancel {
                assert!(started.elapsed() < Duration::from_secs(3));
            }
            let item = ApprovalStore::open(home.path())
                .unwrap()
                .list(None, None)
                .unwrap()
                .pop()
                .unwrap();
            assert!(item.decision.is_none());
            assert_eq!(item.delivery, Delivery::Invalidated);
            if !cancel {
                assert_eq!(item.state, ApprovalState::Expired);
            }
        }
    }

    #[test]
    fn snapshot_reloads_only_after_the_file_changes() {
        let home = home(PermissionMode::Manual);
        let mut permissions = PermissionLoop::load(home.path()).unwrap();
        assert!(!permissions.refresh().unwrap());
        let mut config = RuntimeConfig::load(home.path()).unwrap();
        config.permissions.mode = PermissionMode::AutoAllow;
        config.save(home.path()).unwrap();
        assert!(permissions.refresh().unwrap());
        assert_eq!(permissions.config().mode, PermissionMode::AutoAllow);
        assert!(!permissions.refresh().unwrap());
        // Saving identical content is not a policy change.
        config.save(home.path()).unwrap();
        assert!(!permissions.refresh().unwrap());
    }

    #[test]
    fn waiting_request_checks_storage_rarely_and_wakes_on_decision() {
        let home = home(PermissionMode::Manual);
        let (record, _) = crate::RunStore::open(home.path())
            .unwrap()
            .claim(&crate::RunRequest::new("a", "task"), None)
            .unwrap();
        let scope = Scope::new(&record, None);
        let path = home.path().to_owned();
        let decided = std::thread::spawn(move || {
            let store = ApprovalStore::open(&path).unwrap();
            let item = loop {
                if let Some(item) = store.list(None, None).unwrap().pop() {
                    break item;
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            std::thread::sleep(Duration::from_millis(2500));
            store
                .decide(&item.id, item.revision, json!("allow"), "ok", None)
                .unwrap();
            Instant::now()
        });
        let fences = Cell::new(0);
        let mut permissions = PermissionLoop::load(home.path()).unwrap();
        let (allowed, changed) = permissions
            .authorize(
                Request {
                    scope: &scope,
                    call: "call-1",
                    tool: "framework_message_send",
                    arguments: &json!({}),
                    input: "task",
                },
                &CancellationToken::new(),
                &|| {
                    fences.set(fences.get() + 1);
                    Ok(())
                },
            )
            .unwrap();
        let woke = Instant::now();
        let decided_at = decided.join().unwrap();
        assert!(allowed && !changed);
        assert!(woke.duration_since(decided_at) < Duration::from_millis(500));
        // About one storage check per second while idle, instead of one per 100 ms.
        assert!(fences.get() <= 8, "fence checks: {}", fences.get());
    }
}
