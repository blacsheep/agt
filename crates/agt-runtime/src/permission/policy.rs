pub(super) enum Decision {
    Allow(String),
    Deny(String),
    Human(String),
    Pending,
}
pub(super) trait PermissionPolicy {
    fn poll(&mut self) -> Decision;
    fn cancel(&mut self) {}
}
pub(super) struct Manual;
impl PermissionPolicy for Manual {
    fn poll(&mut self) -> Decision {
        Decision::Pending
    }
}
pub(super) struct AutoAllow;
impl PermissionPolicy for AutoAllow {
    fn poll(&mut self) -> Decision {
        Decision::Allow("Explicit native auto_allow policy".into())
    }
}
