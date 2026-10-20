//! Workspace policy, separate from model and Agent resource definitions.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub scheduler: SchedulerConfig,
    pub memory: MemoryConfig,
    pub execution: ExecutionConfig,
    pub permissions: PermissionConfig,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerConfig {
    pub concurrency: Option<usize>,
    pub parallel_work: bool,
    pub replies_per_agent: Option<u32>,
    pub discussion_seconds: Option<u32>,
    pub model_steps: Option<u32>,
    /// IANA zone for model-facing times and timers; `None` uses the host's zone.
    pub timezone: Option<String>,
    /// How long the workspace must be quiet before owners review their open goals;
    /// `None` turns automatic reviews off.
    pub goal_idle_seconds: Option<u32>,
    /// Reviews in a row without anyone else acting before a goal waits for a person;
    /// `None` keeps reviewing (up to a fixed maximum).
    pub goal_stalled_reviews: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionConfig {
    pub enabled: bool,
    pub timeout_seconds: Option<u32>,
    pub output_bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigStamp {
    inode: u64,
    modified: Option<std::time::SystemTime>,
    len: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    ModelReview,
    Manual,
    AutoAllow,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionConfig {
    pub harness_mode: HarnessPermissionMode,
    pub mode: PermissionMode,
    pub reviewer_model: Option<String>,
    pub review_timeout_seconds: u32,
    pub request_timeout_seconds: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessPermissionMode {
    #[default]
    Manual,
    AutoAllow,
}
impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            mode: PermissionMode::ModelReview,
            harness_mode: HarnessPermissionMode::Manual,
            reviewer_model: None,
            review_timeout_seconds: 30,
            request_timeout_seconds: 600,
        }
    }
}
impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            concurrency: Some(3),
            parallel_work: false,
            replies_per_agent: Some(100),
            discussion_seconds: Some(1800),
            model_steps: None,
            timezone: None,
            goal_idle_seconds: Some(120),
            goal_stalled_reviews: Some(3),
        }
    }
}
impl Default for MemoryConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_seconds: Some(300),
            output_bytes: 65536,
        }
    }
}
impl RuntimeConfig {
    pub fn load(home: &Path) -> Result<Self> {
        let file = home.join("config.yaml");
        agt_storage::check_no_symlinks_under(home, &file)?;
        let config: Self = match std::fs::read_to_string(&file) {
            Ok(text)
                if !text
                    .lines()
                    .all(|l| l.trim().is_empty() || l.trim_start().starts_with('#')) =>
            {
                serde_yaml::from_str(&text).context("invalid config.yaml")?
            }
            Ok(_) => Self::default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.scheduler.concurrency == Some(0)
            || self.scheduler.replies_per_agent == Some(0)
            || self.scheduler.discussion_seconds == Some(0)
            || self.scheduler.model_steps == Some(0)
            || self.scheduler.goal_idle_seconds == Some(0)
            || self.scheduler.goal_stalled_reviews == Some(0)
            || self.execution.timeout_seconds == Some(0)
        {
            bail!("limits must be positive; use null for unlimited");
        }
        if let Some(zone) = &self.scheduler.timezone
            && jiff::tz::TimeZone::get(zone).is_err()
        {
            bail!("scheduler.timezone must be an IANA zone such as Asia/Shanghai");
        }
        if !(1..=120).contains(&self.permissions.review_timeout_seconds)
            || !(1..=86400).contains(&self.permissions.request_timeout_seconds)
        {
            bail!("permission review timeout must be 1..120 and request timeout 1..86400 seconds");
        }
        if self
            .permissions
            .reviewer_model
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
        {
            bail!("reviewer_model must not be empty")
        }
        if !(1024..=1048576).contains(&self.execution.output_bytes) {
            bail!("execution.output_bytes must be 1024..1048576");
        }
        Ok(())
    }
    pub fn save(&self, home: &Path) -> Result<()> {
        self.save_if(home, None)
    }
    pub fn save_if(&self, home: &Path, expected: Option<&Self>) -> Result<()> {
        self.validate()?;
        agt_storage::ensure_dir_under(home, home)?;
        let path = home.join("config.lock");
        agt_storage::check_no_symlinks_under(home, &path)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        if let Some(expected) = expected
            && &Self::load(home)? != expected
        {
            bail!("configuration changed; refresh before saving");
        }
        agt_storage::write_atomic_under(
            home,
            home.join("config.yaml"),
            serde_yaml::to_string(self)?.as_bytes(),
        )?;
        drop(lock);
        // Running hosts hold a snapshot and reload only when woken or the stamp changes.
        agt_storage::wake::notify(home);
        Ok(())
    }

    /// Cheap identity of the saved file. Saves replace the file atomically, so any
    /// change yields a different stamp without parsing YAML.
    pub fn stamp(home: &Path) -> Option<ConfigStamp> {
        let metadata = std::fs::metadata(home.join("config.yaml")).ok()?;
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&metadata);
        #[cfg(not(unix))]
        let inode = 0;
        Some(ConfigStamp {
            inode,
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
    pub fn expires_at(&self, now: i64) -> i64 {
        // Exact JS-safe sentinel; the public API separately identifies unlimited policy.
        self.scheduler
            .discussion_seconds
            .map_or(9_007_199_254_740_991, |s| {
                now.saturating_add(i64::from(s) * 1000)
            })
    }
    pub fn describe() -> Value {
        json!({"unlimited":"null 表示不限，0 无效；取消、权限与会话互斥始终有效。","fields":[
            {"key":"permissions.harness_mode","default":"manual","description":"Harness 原生请求处理：manual / auto_allow；不改变引擎沙箱。","effective":"当前待审批及后续请求"},
            {"key":"permissions.mode","default":"model_review","description":"Native 工具权限策略：model_review / manual / auto_allow；不影响 Harness 原生审批。","effective":"当前待审批及后续请求"},
            {"key":"permissions.reviewer_model","default":null,"description":"独立审核 Model 资源；未配置或审核异常转人工。","effective":"当前待审批及后续请求"},
            {"key":"permissions.review_timeout_seconds","default":30,"description":"审核模型超时（1–120 秒），超时转人工。","effective":"当前待审批及后续请求"},
            {"key":"permissions.request_timeout_seconds","default":600,"description":"Native 审批总等待秒数，超时取消执行。","effective":"后续请求；不延长已有请求"},
            {"key":"scheduler.parallel_work","default":false,"description":"允许双方显式声明 parallel 的独立工作任务在同一身份并行；普通聊天仍串行，共享资源声明仍互斥。模型需为会写入的同一资源使用一致的资源键。","effective":"调度器重启"},
            {"key":"scheduler.concurrency","default":3,"description":"同时执行的任务总数；不含保存检查点后的等待。增大可减少排队，但增加模型与本机负载。null 不限。","effective":"调度器重启；CLI --concurrency 可覆盖"},
            {"key":"scheduler.replies_per_agent","default":100,"description":"每个身份在同一讨论中允许进入回复执行的次数，不是工具调用次数。null 不限。","effective":"调度器重启；已用次数保留"},
            {"key":"scheduler.discussion_seconds","default":1800,"description":"一次讨论授权的有效秒数，子任务共享授权。到期只阻止新执行（包括子任务、挂起恢复及通知处理），已经运行的任务可完成并投递结果；长流程可设 null。","effective":"新授权或显式续期；不追改已有到期时间"},
            {"key":"scheduler.timezone","default":null,"description":"模型看到的时间和定时任务使用的 IANA 时区，例如 Asia/Shanghai。null 使用本机时区。","effective":"新执行和新建的定时任务；已有定时任务保留创建时的时区"},
            {"key":"scheduler.goal_idle_seconds","default":120,"description":"工作空间连续安静多少秒后，让未完成目标的负责人审核一次：完成、疏通卡住的工作，或说明在等人。null 关闭自动审核。","effective":"下一次调度检查"},
            {"key":"scheduler.goal_stalled_reviews","default":3,"description":"连续多少次审核后除负责人外仍无人行动，就暂停该目标的自动审核并等人处理。null 不限（仍有总次数上限）。","effective":"下一次调度检查"},
            {"key":"scheduler.model_steps","default":null,"description":"原生执行器每个 Run 的模型步数上限；不限制 Codex 内部步骤。null 不限。","effective":"新 Run"},
            {"key":"memory.enabled","default":true,"description":"允许按当前身份搜索可见历史和保存带来源的私有主题摘要；不自动共享他人私聊。","effective":"关闭：下次调用即拒绝；开启：新 Run 才提供记忆工具"},
            {"key":"execution.enabled","default":false,"description":"允许挂载 local_execution 的原生 Agent 执行宿主机命令。工作目录不是沙箱，命令拥有服务账号的操作权限。","effective":"关闭：下一条命令即拒绝；开启：新 Run 才提供命令工具"},
            {"key":"execution.timeout_seconds","default":300,"description":"本机命令的总执行秒数，超时终止进程组；null 不限，仍可取消。","effective":"新命令"},
            {"key":"execution.output_bytes","default":65536,"description":"每个命令返回的 stdout/stderr 总字节上限；超出截断，不停止命令。","effective":"新命令"}
        ]})
    }
}
