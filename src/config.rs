use std::path::PathBuf;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use serde::Deserialize;
use crate::error::AppError;

/// 每个被更新软件一份的配置，由主程序在拉起 updater 时传入路径。
/// 迁移到新项目时只改这份 JSON，无需改动 updater 二进制。
#[derive(Debug, Deserialize)]
pub struct Config {
    /// 仓库，格式 `owner/name`（GitHub 或 Gitee）。
    pub repo: String,
    /// 本平台要下载的资产文件名，例如 `myapp-windows-x64-online-update.zip`。
    pub asset_name: String,
    /// 本地版本号文件路径（一行 `v1.2.3` 或 `1.2.3`）。
    pub version_file: String,
    /// 安装根目录（建议主程序传绝对路径）。
    pub install_dir: String,
    /// 单 exe 资产时，目标 exe 文件名；zip 资产可留空。
    #[serde(default)]
    pub exe_name: Option<String>,

    /// 可选：更新源 API 基址。留空默认 GitHub `https://api.github.com/repos`；
    /// 改 Gitee 兜底填 `https://gitee.com/api/v5/repos`。
    #[serde(default)]
    pub feed_url: Option<String>,
    /// 可选：自动检查最小间隔（秒），默认 86400（24 小时）。
    #[serde(default)]
    pub check_interval_secs: Option<u64>,
    /// 可选：是否允许启动时自动检查，默认 true；手动刷新不受此开关限制。
    #[serde(default)]
    pub auto_check: Option<bool>,
    /// 可选：检查状态文件路径，默认 `<install_dir>/.app-update/state.json`。
    #[serde(default)]
    pub state_file: Option<String>,
    /// 可选：产品标识，验签后与信封 payload.product 比对，防串包。
    #[serde(default)]
    pub product: Option<String>,
    /// 可选：签名信封资产名（如 `latest-version.json`）；留空走 Release API 源。
    #[serde(default)]
    pub metadata_file: Option<String>,
    /// 可选：新版启动探测等待时长（秒），默认 10。
    #[serde(default)]
    pub launch_probe_secs: Option<u64>,
}

/// 本地检查状态：记录上次联网检查时间，用于 24h 间隔判断。
#[derive(Debug, Deserialize, serde::Serialize, Default)]
pub struct UpdateState {
    #[serde(default)]
    pub last_check_at: Option<i64>,
    /// Download 阶段记录待安装的远端版本号，Apply 阶段写回本地版本文件。
    #[serde(default)]
    pub pending_version: Option<String>,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, AppError> {
        let text = fs::read_to_string(path)
            .map_err(|e| AppError::Config(format!("读取配置失败 {path}: {e}")))?;
        let cfg: Config = serde_json::from_str(&text)
            .map_err(|e| AppError::Config(format!("配置 JSON 解析失败: {e}")))?;
        Ok(cfg)
    }

    pub fn install_dir(&self) -> PathBuf {
        PathBuf::from(&self.install_dir)
    }

    /// 暂存目录：下载残片、staging、backup、state 都放这里。
    pub fn cache_dir(&self) -> PathBuf {
        self.install_dir().join(".app-update")
    }

    pub fn state_path(&self) -> PathBuf {
        self.state_file
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.cache_dir().join("state.json"))
    }

    /// 自动检查最小间隔（秒）。
    pub fn interval_secs(&self) -> u64 {
        self.check_interval_secs.unwrap_or(86400)
    }

    pub fn auto_check_enabled(&self) -> bool {
        self.auto_check.unwrap_or(true)
    }

    pub fn product(&self) -> &str {
        self.product.as_deref().unwrap_or("")
    }

    pub fn launch_probe_secs(&self) -> u64 {
        self.launch_probe_secs.unwrap_or(10)
    }

    /// 读取本地已安装版本；文件不存在时按 0.0.0 处理（首次安装场景）。
    pub fn local_version(&self) -> Result<semver::Version, AppError> {
        let raw = fs::read_to_string(&self.version_file).unwrap_or_default();
        let trimmed = raw.trim().trim_start_matches('v').to_string();
        if trimmed.is_empty() {
            return Ok(semver::Version::new(0, 0, 0));
        }
        semver::Version::parse(&trimmed)
            .map_err(|e| AppError::Config(format!("本地版本号无法解析 `{trimmed}`: {e}")))
    }

    pub fn write_local_version(&self, version: &semver::Version) -> Result<(), AppError> {
        fs::write(&self.version_file, format!("{version}\n"))
            .map_err(|e| AppError::Io(e))
    }

    /// 读取上次联网检查的 Unix 时间戳（秒）。
    pub fn read_state(&self) -> UpdateState {
        let raw = fs::read_to_string(self.state_path()).unwrap_or_default();
        serde_json::from_str::<UpdateState>(&raw).unwrap_or_default()
    }

    /// 记录本次联网检查时间。
    pub fn write_state(&self, state: &UpdateState) -> Result<(), AppError> {
        if let Some(parent) = self.state_path().parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(state)
            .map_err(|e| AppError::Config(format!("状态序列化失败: {e}")))?;
        fs::write(&self.state_path(), text).map_err(AppError::Io)
    }

    /// 当前 Unix 秒。
    pub fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}
