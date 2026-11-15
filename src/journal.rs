//! 事务日志（journal）与新版启动探测标记。
//!
//! - journal 记录本次替换涉及的文件、旧版备份信息与事务状态；
//!   若程序在"备份已做、替换未完成"之间断电/崩溃，下次启动读到
//!   `state = applying` 的未决事务，按备份回滚。
//! - 启动探测标记：替换完成、拉起新版后写入标记；新版主程序启动成功后
//!   删除该标记。updater 等待一段时间检测标记是否被清除，超时未清除
//!   判定新版起不来，自动回滚。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::AppError;

const JOURNAL_NAME: &str = "journal.json";
const MARKER_NAME: &str = ".launch-marker";

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JournalFile {
    pub path: String,
    /// 替换前该文件是否已存在于安装目录。
    pub existed: bool,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Journal {
    pub schema: u32,
    /// `applying` = 事务进行中（未决，需恢复）；`done` = 成功；`rolled_back` = 已回滚。
    pub state: String,
    pub version: String,
    pub previous_version: Option<String>,
    pub files: Vec<JournalFile>,
}

impl Journal {
    pub fn new(version: String, previous_version: Option<String>) -> Self {
        Journal {
            schema: 1,
            state: "applying".to_string(),
            version,
            previous_version,
            files: Vec::new(),
        }
    }
}

pub fn journal_path(cache: &PathBuf) -> PathBuf {
    cache.join(JOURNAL_NAME)
}

pub fn marker_path(cache: &PathBuf) -> PathBuf {
    cache.join(MARKER_NAME)
}

pub fn load(cache: &PathBuf) -> Option<Journal> {
    let raw = std::fs::read_to_string(journal_path(cache)).ok()?;
    serde_json::from_str::<Journal>(&raw).ok()
}

pub fn save(cache: &PathBuf, journal: &Journal) -> Result<(), AppError> {
    if let Some(parent) = journal_path(cache).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(journal)
        .map_err(|e| AppError::Config(format!("journal 序列化失败: {e}")))?;
    std::fs::write(journal_path(cache), text).map_err(AppError::Io)
}

pub fn clear(cache: &PathBuf) {
    let _ = std::fs::remove_file(journal_path(cache));
}

/// 是否存在未决事务（上次中断在 applying 阶段）。
pub fn has_pending(cache: &PathBuf) -> bool {
    matches!(load(cache), Some(j) if j.state == "applying")
}

pub fn write_marker(cache: &PathBuf, version: &str) -> Result<(), AppError> {
    std::fs::write(marker_path(cache), format!("{version}\n")).map_err(AppError::Io)
}

pub fn marker_exists(cache: &PathBuf) -> bool {
    marker_path(cache).exists()
}

pub fn clear_marker(cache: &PathBuf) {
    let _ = std::fs::remove_file(marker_path(cache));
}
