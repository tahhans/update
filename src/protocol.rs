use serde::{Deserialize, Serialize};
use std::io::{self, Write};

pub const PROTOCOL: u32 = 2;

/// updater → 主程序的状态机阶段。
#[derive(Debug, Clone, Copy)]
pub enum State {
    Idle,
    Checking,
    Available,
    Downloading,
    Verifying,
    Ready,
    Applying,
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "Idle",
            State::Checking => "Checking",
            State::Available => "Available",
            State::Downloading => "Downloading",
            State::Verifying => "Verifying",
            State::Ready => "Ready",
            State::Applying => "Applying",
            State::Failed => "Failed",
        }
    }
}

/// 主程序 → updater 的命令帧（stdin 一行一条 JSON）。第一版按需解析。
#[derive(Debug, Deserialize, Default)]
pub struct Command {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub proceed: Option<bool>,
}

/// updater → 主程序的状态帧（stdout 一行一条 JSON）。
#[derive(Serialize)]
struct StatusEvent {
    protocol: u32,
    #[serde(rename = "type")]
    event_type: &'static str,
    state: String,
    version: String,
    notes: String,
    received: u64,
    total: u64,
    error: Option<String>,
}

/// 向主程序推一条状态帧并立刻 flush，保证界面实时收到。
pub fn emit(
    state: State,
    version: &str,
    notes: &str,
    received: u64,
    total: u64,
    error: Option<&str>,
) {
    let ev = StatusEvent {
        protocol: PROTOCOL,
        event_type: "status",
        state: state.as_str().to_string(),
        version: version.to_string(),
        notes: notes.to_string(),
        received,
        total,
        error: error.map(|s| s.to_string()),
    };
    // 帧序列化失败不应中断更新；降级为纯文本行。
    match serde_json::to_string(&ev) {
        Ok(line) => println!("{line}"),
        Err(_) => println!("{} {state:?}", state.as_str()),
    }
    let _ = io::stdout().flush();
}
