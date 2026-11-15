mod config;
mod download;
mod error;
mod feed;
mod install;
mod journal;
mod protocol;
mod signature;
mod update;

use std::env;
use update::{Mode, run};

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut mode: Mode = Mode::Check { manual: false };
    let mut cfg_path: Option<String> = None;
    let mut parent_pid: Option<u32> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => mode = Mode::Check { manual: false },
            "--manual" => mode = Mode::Check { manual: true },
            "--download" => mode = Mode::Download,
            "--apply" => mode = Mode::Apply,
            "--parent-pid" => {
                i += 1;
                parent_pid = args.get(i).and_then(|v| v.parse().ok());
            }
            "--config" | "-c" => {
                i += 1;
                cfg_path = args.get(i).cloned();
            }
            s if s.ends_with(".json") => cfg_path = Some(s.to_string()),
            _ => {}
        }
        i += 1;
    }

    let cfg_path = match cfg_path {
        Some(p) => p,
        None => {
            eprintln!("用法:");
            eprintln!("  generic-updater --check   <config.json>  自动检查（受 24h 间隔限制）");
            eprintln!("  generic-updater --manual  <config.json>  手动刷新检查（强制联网）");
            eprintln!("  generic-updater --download <config.json> 检查+下载+校验，停在 Ready");
            eprintln!("  generic-updater --apply   <config.json> [--parent-pid <pid>] 安装并重启");
            std::process::exit(1);
        }
    };

    let cfg = match config::Config::load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    match run(&cfg, mode, parent_pid) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
