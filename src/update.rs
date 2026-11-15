use crate::config::Config;
use crate::download::download_to;
use crate::error::AppError;
use crate::feed::{fetch_envelope_release, fetch_latest};
use crate::install::{self, InstallContext};
use crate::journal::{self, Journal};
use crate::protocol::{emit, State};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub enum Mode {
    /// 只检查是否有更新。`manual` = true 表示用户点"刷新"，强制联网；
    /// false 为启动时自动检查，受 24h 间隔与 auto_check 开关限制。
    Check { manual: bool },
    /// 检查 + 下载 + 校验 + 解压到 staging，停在 Ready，等待主程序弹"重启软件"按钮。
    Download,
    /// 复用已就位的 staging：等待旧进程退出 → 备份覆盖 → 写版本号 → 拉起新版 → 探测 → 清理。
    Apply,
}

/// 退出码约定：0 = 无更新 / 间隔内跳过，2 = 有更新或更新成功，1 = 失败。
pub fn run(cfg: &Config, mode: Mode, parent_pid: Option<u32>) -> Result<i32, AppError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("generic-updater/0.1")
        .build()
        .map_err(|e| AppError::Network(e.to_string()))?;

    match mode {
        Mode::Check { manual } => check(cfg, &client, manual),
        Mode::Download => download_phase(cfg, &client),
        Mode::Apply => apply_phase(cfg, parent_pid),
    }
}

/// 按配置选择更新源：配置了信封资产名则走签名信封验签源，否则走 Release API 源。
fn fetch_release(cfg: &Config, client: &reqwest::blocking::Client) -> Result<crate::feed::ReleaseInfo, AppError> {
    match &cfg.metadata_file {
        Some(metadata) => fetch_envelope_release(
            client,
            &cfg.repo,
            metadata,
            cfg.product(),
            &cfg.asset_name,
            cfg.feed_url.as_deref(),
        ),
        None => fetch_latest(client, &cfg.repo, &cfg.asset_name, cfg.feed_url.as_deref()),
    }
}

// ---------- 阶段一：检查 ----------

fn check(cfg: &Config, client: &reqwest::blocking::Client, manual: bool) -> Result<i32, AppError> {
    if !manual && !cfg.auto_check_enabled() {
        emit(State::Idle, "", "", 0, 0, None);
        return Ok(0);
    }

    if !manual {
        let state = cfg.read_state();
        if let Some(last) = state.last_check_at {
            let elapsed = (Config::now_unix() - last).max(0) as u64;
            if elapsed < cfg.interval_secs() {
                emit(State::Idle, "", "", 0, 0, None);
                return Ok(0);
            }
        }
    }

    emit(State::Checking, "", "", 0, 0, None);

    let release = fetch_release(cfg, client)?;
    let local = cfg.local_version()?;

    let mut state = cfg.read_state();
    state.last_check_at = Some(Config::now_unix());
    cfg.write_state(&state)?;

    if release.version <= local {
        emit(State::Idle, &release.version.to_string(), "", 0, 0, None);
        return Ok(0);
    }

    emit(
        State::Available,
        &release.version.to_string(),
        &release.notes,
        0,
        0,
        None,
    );
    Ok(2)
}

// ---------- 阶段二：下载 + 校验 + staging 就位 ----------

fn download_phase(cfg: &Config, client: &reqwest::blocking::Client) -> Result<i32, AppError> {
    emit(State::Checking, "", "", 0, 0, None);
    let release = fetch_release(cfg, client)?;
    let local = cfg.local_version()?;

    if release.version <= local {
        emit(State::Idle, &release.version.to_string(), "", 0, 0, None);
        return Ok(0);
    }

    let version = release.version.to_string();
    let notes = release.notes.clone();
    let asset = release.asset.clone();

    let cache = cfg.cache_dir();
    std::fs::create_dir_all(&cache)?;
    let dest = cache.join(&cfg.asset_name);

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_download = cancel.clone();

    emit(State::Available, &version, &notes, 0, 0, None);

    download_to(
        client,
        &asset.url,
        &dest,
        asset.sha256.as_deref(),
        asset.size,
        |received, total| emit(State::Downloading, &version, &notes, received, total, None),
        move || cancel_for_download.load(Ordering::Relaxed),
    )
    .map_err(|e| {
        emit(State::Failed, &version, &notes, 0, 0, Some(&e.to_string()));
        e
    })?;

    emit(State::Verifying, &version, &notes, 0, 0, None);

    let staging = cache.join("staging");
    install::prepare_staging(&dest, &staging)?;

    let mut state = cfg.read_state();
    state.pending_version = Some(version.clone());
    state.last_check_at = Some(Config::now_unix());
    cfg.write_state(&state)?;

    emit(State::Ready, &version, &notes, 1, 1, None);
    Ok(2)
}

// ---------- 阶段三：备份覆盖 + 重启 + 探测 + 清理 ----------

fn apply_phase(cfg: &Config, parent_pid: Option<u32>) -> Result<i32, AppError> {
    let cache = cfg.cache_dir();
    let staging = cache.join("staging");
    if !staging.exists() {
        return Err(AppError::Install(
            "暂存内容缺失，请先完成下载（--download）".to_string(),
        ));
    }

    // 0. 先恢复上次中断的未决事务（断电 / 崩溃残留）。
    recover_pending(cfg, &cache)?;

    let state = cfg.read_state();
    let pending = state
        .pending_version
        .clone()
        .unwrap_or_else(|| "0.0.0".to_string());
    let notes = String::new();

    // 等待旧程序退出，释放 exe 文件占用。
    wait_for_parent(parent_pid)?;

    let backup = cache.join("backup");
    emit(State::Applying, &pending, &notes, 0, 0, None);

    let (ctx, files) = match install::install(&staging, &cfg.install_dir(), &backup) {
        Ok(result) => result,
        Err(e) => {
            emit(State::Failed, &pending, &notes, 0, 0, Some(&e.to_string()));
            return Err(e);
        }
    };

    // 写事务日志：此时状态为 applying，崩溃后下次启动可据此回滚。
    let journal = Journal::new(pending.clone(), state.pending_version.clone());
    let mut journal = journal;
    journal.files = files;
    journal::save(&cache, &journal)?;

    // 写新版本号；失败则回滚。
    let ver = semver::Version::parse(&pending)
        .map_err(|e| AppError::Config(format!("待安装版本号无法解析 `{pending}`: {e}")))?;
    if let Err(e) = cfg.write_local_version(&ver) {
        rollback_and_cleanup(&ctx);
        finalize_failed(&cache, &pending);
        emit(State::Failed, &pending, &notes, 0, 0, Some(&e.to_string()));
        return Err(e);
    }

    // 写启动探测标记，再拉起新版；新版启动成功后会删除该标记。
    journal::write_marker(&cache, &pending)?;
    if let Err(e) = relaunch(cfg, &staging) {
        rollback_and_cleanup(&ctx);
        finalize_failed(&cache, &pending);
        emit(State::Failed, &pending, &notes, 0, 0, Some(&e.to_string()));
        return Err(e);
    }

    // 启动探测：等待标记被新版清除；超时未清除判定新版起不来，自动回滚。
    if !probe_launch(cfg, &cache) {
        rollback_and_cleanup(&ctx);
        finalize_failed(&cache, &pending);
        emit(
            State::Failed,
            &pending,
            &notes,
            0,
            0,
            Some("新版启动探测超时，已自动回滚到旧版本"),
        );
        return Err(AppError::Install("新版启动探测超时，已回滚".to_string()));
    }

    // 成功：清理暂存、备份、安装包、事务日志与标记。
    install::cleanup(&ctx);
    let _ = std::fs::remove_file(cache.join(&cfg.asset_name));
    journal::clear(&cache);
    journal::clear_marker(&cache);

    let mut state = state;
    state.pending_version = None;
    let _ = cfg.write_state(&state);

    emit(State::Ready, &pending, &notes, 1, 1, None);
    Ok(2)
}

/// 启动时若存在未决事务，按备份回滚并清理残留。
fn recover_pending(cfg: &Config, cache: &PathBuf) -> Result<(), AppError> {
    if !journal::has_pending(cache) {
        return Ok(());
    }
    let ctx = InstallContext {
        staging: cache.join("staging"),
        backup: cache.join("backup"),
        target_root: cfg.install_dir(),
    };
    install::rollback(&ctx)?;
    install::cleanup(&ctx);
    journal::clear(cache);
    journal::clear_marker(cache);
    Ok(())
}

/// 启动探测：轮询标记是否被新版清除。
fn probe_launch(cfg: &Config, cache: &PathBuf) -> bool {
    let deadline = Instant::now() + Duration::from_secs(cfg.launch_probe_secs());
    while Instant::now() < deadline {
        if !journal::marker_exists(cache) {
            return true;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    false
}

/// 失败收尾：保留一个 blocked 记录，清理事务日志与标记。
fn finalize_failed(cache: &PathBuf, version: &str) {
    let blocked = cache.join(format!("{}.blocked", version));
    let _ = std::fs::write(blocked, "新版启动失败，已回滚\n");
    journal::clear(cache);
    journal::clear_marker(cache);
}

fn rollback_and_cleanup(ctx: &InstallContext) {
    let _ = install::rollback(ctx);
    install::cleanup(ctx);
}

#[cfg(windows)]
fn wait_for_parent(pid: Option<u32>) -> Result<(), AppError> {
    use std::ffi::c_void;

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const INFINITE: u32 = u32::MAX;
    const WAIT_OBJECT_0: u32 = 0;
    const ERROR_INVALID_PARAMETER: u32 = 87;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }

    let Some(pid) = pid else {
        return Ok(());
    };

    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        if error == ERROR_INVALID_PARAMETER {
            return Ok(());
        }
        return Err(AppError::Install(format!(
            "无法等待主程序退出（PID {pid}）：{}",
            std::io::Error::from_raw_os_error(error as i32)
        )));
    }

    let wait_result = unsafe { WaitForSingleObject(handle, INFINITE) };
    let wait_error = (wait_result != WAIT_OBJECT_0).then(|| unsafe { GetLastError() });
    unsafe {
        CloseHandle(handle);
    }
    if let Some(error) = wait_error {
        return Err(AppError::Install(format!(
            "等待主程序退出失败（PID {pid}）：{}",
            std::io::Error::from_raw_os_error(error as i32)
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn wait_for_parent(_pid: Option<u32>) -> Result<(), AppError> {
    Ok(())
}

fn relaunch(cfg: &Config, staging: &PathBuf) -> Result<(), AppError> {
    let exe_name = cfg
        .exe_name
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or_else(|| {
            cfg.asset_name
                .strip_suffix(".zip")
                .unwrap_or(&cfg.asset_name)
        });

    let exe = cfg.install_dir().join(exe_name);
    if !exe.exists() {
        let staged = staging.join(exe_name);
        if staged.exists() {
            std::process::Command::new(&staged)
                .spawn()
                .map_err(|e| AppError::Install(format!("拉起新版失败: {e}")))?;
            return Ok(());
        }
        return Err(AppError::Install(format!("找不到新版程序: {exe_name}")));
    }

    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| AppError::Install(format!("拉起新版失败: {e}")))?;
    Ok(())
}
