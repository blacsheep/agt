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
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerConfig {
    pub concurrency: Option<usize>,
    pub parallel_work: bool,
    pub replies_per_agent: Option<u32>,
    pub discussion_seconds: Option<u32>,
    pub model_steps: Option<u32>,
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
impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            concurrency: Some(3),
            parallel_work: false,
            replies_per_agent: Some(100),
            discussion_seconds: Some(1800),
            model_steps: None,
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
            || self.execution.timeout_seconds == Some(0)
        {
            bail!("limits must be positive; use null for unlimited");
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
        Ok(())
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
            {"key":"scheduler.parallel_work","default":false,"description":"允许双方显式声明 parallel 的独立工作任务在同一身份并行；普通聊天仍串行，共享资源声明仍互斥。模型需为会写入的同一资源使用一致的资源键。","effective":"调度器重启"},
            {"key":"scheduler.concurrency","default":3,"description":"同时执行的任务总数；不含保存检查点后的等待。增大可减少排队，但增加模型与本机负载。null 不限。","effective":"调度器重启；CLI --concurrency 可覆盖"},
            {"key":"scheduler.replies_per_agent","default":100,"description":"每个身份在同一讨论中允许进入回复执行的次数，不是工具调用次数。null 不限。","effective":"调度器重启；已用次数保留"},
            {"key":"scheduler.discussion_seconds","default":1800,"description":"一次讨论授权的有效秒数，子任务共享授权。到期会阻止新执行并撤销在途执行授权；长流程可设 null。","effective":"新授权或显式续期；不追改已有到期时间"},
            {"key":"scheduler.model_steps","default":null,"description":"原生执行器每个 Run 的模型步数上限；不限制 Codex 内部步骤。null 不限。","effective":"新 Run"},
            {"key":"memory.enabled","default":true,"description":"允许按当前身份搜索可见历史和保存带来源的私有主题摘要；不自动共享他人私聊。","effective":"下次工具调用"},
            {"key":"execution.enabled","default":false,"description":"允许挂载 local_execution 的原生 Agent 执行宿主机命令。工作目录不是沙箱，命令拥有服务账号的操作权限。","effective":"新 Run"},
            {"key":"execution.timeout_seconds","default":300,"description":"本机命令的总执行秒数，超时终止进程组；null 不限，仍可取消。","effective":"新命令"},
            {"key":"execution.output_bytes","default":65536,"description":"每个命令返回的 stdout/stderr 总字节上限；超出截断，不停止命令。","effective":"新命令"}
        ]})
    }
}
