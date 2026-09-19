use crate::{OutputMode, emit};
use agt_config::RuntimeConfig;
use anyhow::Result;
use clap::Subcommand;
use std::path::Path;
#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// 查看最终生效的工作空间默认配置（不含 CLI 临时覆盖）
    Show,
    /// 使用读取时的配置校验冲突，再保存完整 JSON 配置
    Update {
        #[arg(long)]
        value: String,
        #[arg(long)]
        expected: String,
    },
    /// 查看各配置用途、默认值、单位与生效时机
    Describe,
    /// 从 YAML 文件校验并替换工作空间配置
    Apply { file: std::path::PathBuf },
}
pub(crate) fn dispatch(command: ConfigCommand, home: &Path, output: OutputMode) -> Result<()> {
    match command {
        ConfigCommand::Update { value, expected } => {
            let value: RuntimeConfig = serde_json::from_str(&value)?;
            let expected: RuntimeConfig = serde_json::from_str(&expected)?;
            value.save_if(home, Some(&expected))?;
            emit(&value, output)
        }
        ConfigCommand::Show => emit(&RuntimeConfig::load(home)?, output),
        ConfigCommand::Describe => emit(&RuntimeConfig::describe(), output),
        ConfigCommand::Apply { file } => {
            let config: RuntimeConfig = serde_yaml::from_str(&std::fs::read_to_string(file)?)?;
            config.save(home)?;
            emit(&config, output)
        }
    }
}
