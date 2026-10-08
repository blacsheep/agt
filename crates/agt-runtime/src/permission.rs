//! Native-only permission stages. Policies cannot execute tools or bypass service authorization.
use crate::approval::{ApprovalStore, Scope};
use crate::{Result, RuntimeError};
use agt_config::runtime::{PermissionConfig, PermissionMode};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
mod model_review;
mod policy;
use policy::{Decision, PermissionPolicy};

pub(crate) struct PermissionLoop {
    home: PathBuf,
    config: PermissionConfig,
    reviewer: Option<crate::ResolvedAgent>,
}
impl PermissionLoop {
    pub(crate) fn new(home: &Path, config: PermissionConfig) -> Self {
        Self {
            reviewer: model_review::load(home, &config).ok(),
            home: home.into(),
            config,
        }
    }
    pub(crate) fn reviewer_fingerprint(&self) -> Option<&str> {
        self.reviewer.as_ref().map(|r| r.fingerprint.as_str())
    }
    pub(crate) fn authorize(
        &self,
        scope: &Scope,
        call: &str,
        tool: &str,
        args: &Value,
        input: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<bool> {
        let config = agt_config::runtime::RuntimeConfig::load(&self.home)
            .map_err(|_| RuntimeError::Configuration("cannot load permission policy"))?
            .permissions;
        Self::new(&self.home, config)
            .run(scope, call, tool, args, input, check)
            .map_err(|e| {
                e.downcast::<RuntimeError>().unwrap_or_else(|_| {
                    RuntimeError::Tool("permission request failed; inspect approval records")
                })
            })
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
                self.reviewer.clone(),
                &self.config,
                context,
            )),
        })
    }
    fn run(
        &self,
        scope: &Scope,
        call: &str,
        tool: &str,
        args: &Value,
        input: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> anyhow::Result<bool> {
        check()?;
        let store = ApprovalStore::open(&self.home)?;
        let request = json!({"tool":tool,"arguments":args,"policy":self.config,"reviewer_fingerprint":self.reviewer_fingerprint(),"options":[{"label":"批准本次调用","value":"allow"},{"label":"拒绝本次调用","value":"deny"},{"label":"取消执行","value":"cancel"}]});
        let item = store.create(
            scope,
            "native",
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
        let mut active = Self::new(&self.home, self.config.clone());
        let context = json!({"tool":tool,"arguments":args,"task":input});
        let mut plugin = active.start_policy(&store, &item.id, context.clone())?;
        loop {
            check()?;
            let current = store.poll(&item.id)?;
            if current.delivery == "invalidated"
                || matches!(current.state.as_str(), "expired" | "canceled")
                    && current.decision.is_none()
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
                store.change(&item.id, None, Some("resolved"))?;
                return Ok(decision == "allow");
            }
            let config = agt_config::runtime::RuntimeConfig::load(&self.home)?.permissions;
            if config.mode != active.config.mode
                || config.reviewer_model != active.config.reviewer_model
                || config.review_timeout_seconds != active.config.review_timeout_seconds
            {
                plugin.cancel();
                active = Self::new(&self.home, config);
                plugin = active.start_policy(&store, &item.id, context.clone())?;
                continue;
            }
            if current.state == "reviewing" || active.config.mode == PermissionMode::AutoAllow {
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
                                Some(if active.config.mode == PermissionMode::AutoAllow {
                                    "auto_allow"
                                } else {
                                    "model_review"
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
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}
