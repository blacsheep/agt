use agt_config::RuntimeConfig;
#[test]
fn configuration_roundtrip_unlimited_and_validation() {
    let home = std::env::temp_dir().join(format!("agt-config-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let mut config = RuntimeConfig::default();
    config.scheduler.concurrency = None;
    config.scheduler.replies_per_agent = None;
    config.scheduler.discussion_seconds = None;
    config.save(&home).unwrap();
    let loaded = RuntimeConfig::load(&home).unwrap();
    assert!(loaded.scheduler.concurrency.is_none());
    assert_eq!(loaded.expires_at(100), 9_007_199_254_740_991);
    let mut changed = loaded.clone();
    changed.scheduler.concurrency = Some(8);
    changed.save_if(&home, Some(&loaded)).unwrap();
    assert!(loaded.save_if(&home, Some(&loaded)).is_err());
    config.scheduler.concurrency = Some(0);
    assert!(config.validate().is_err());
    std::fs::write(home.join("config.yaml"), "scheduler:\n  typo: 4\n").unwrap();
    assert!(RuntimeConfig::load(&home).is_err());
    std::fs::remove_dir_all(home).unwrap();
}
#[test]
fn timezone_must_be_an_iana_zone() {
    let mut config = RuntimeConfig::default();
    assert!(config.scheduler.timezone.is_none());
    config.scheduler.timezone = Some("Asia/Shanghai".into());
    config.validate().unwrap();
    config.scheduler.timezone = Some("Mars/Base".into());
    assert!(config.validate().is_err());
}
