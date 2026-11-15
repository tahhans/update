#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! egui 极简测试主程序（对标 Snow Shot 主程序对接层）。
//!
//! 架构：本程序与外置 `generic-updater` 是两个进程。主程序只负责：
//! 1. 拉起 updater 子进程（`--check/--manual/--download/--apply`）；
//! 2. 按行读取子进程 stdout 的 protocol:2 JSON 状态帧；
//! 3. 把版本号、变更说明、进度、错误渲染到界面；
//! 4. 新版启动成功后删除启动探测标记（updater 据此判定新版起来了）。
//!
//! 编译：在带桌面环境的机器上 `cargo build --release`；
//! 运行时把编译好的 `generic-updater(.exe)` 放在本程序同目录。

use std::collections::VecDeque;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------- 状态帧（updater → 主程序） ----------

#[derive(Debug, Deserialize, Default)]
struct StatusFrame {
    #[serde(default)]
    state: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    received: u64,
    #[serde(default)]
    total: u64,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AppConfig {
    #[serde(default)]
    repo: String,
    #[serde(default)]
    asset_name: String,
    #[serde(default)]
    version_file: String,
    #[serde(default)]
    install_dir: String,
    #[serde(default)]
    exe_name: Option<String>,
    #[serde(default)]
    product: Option<String>,
}

struct Shared {
    current_version: String,
    latest_version: String,
    notes: String,
    received: u64,
    total: u64,
    phase: String,
    status_text: String,
    busy: bool,
    log: VecDeque<String>,
    config: AppConfig,
}

impl Shared {
    fn new(config: AppConfig) -> Self {
        let current = std::fs::read_to_string(&config.version_file)
            .unwrap_or_default()
            .trim()
            .trim_start_matches('v')
            .to_string();
        Shared {
            current_version: if current.is_empty() { "0.0.0".to_string() } else { current },
            latest_version: String::new(),
            notes: String::new(),
            received: 0,
            total: 0,
            phase: "Idle".to_string(),
            status_text: "就绪。点击「检查更新」。".to_string(),
            busy: false,
            log: VecDeque::new(),
            config,
        }
    }

    fn push_log(&mut self, line: String) {
        self.log.push_back(line);
        if self.log.len() > 200 {
            self.log.pop_front();
        }
    }

    fn apply_frame(&mut self, frame: StatusFrame) {
        if !frame.version.is_empty() {
            self.latest_version = frame.version.clone();
        }
        if !frame.notes.is_empty() {
            self.notes = frame.notes;
        }
        self.received = frame.received;
        self.total = frame.total;
        self.phase = frame.state.clone();

        match frame.state.as_str() {
            "Checking" => {
                self.busy = true;
                self.status_text = "正在检查更新…".to_string();
            }
            "Available" => {
                self.busy = false;
                self.status_text = format!("发现新版本 {}，可点击「立即更新」。", self.latest_version);
            }
            "Downloading" => {
                self.busy = true;
                self.status_text = "正在下载更新包…".to_string();
            }
            "Verifying" => {
                self.busy = true;
                self.status_text = "正在校验完整性…".to_string();
            }
            "Ready" => {
                self.busy = false;
                self.status_text = "下载校验完成，可点击「重启软件」完成更新。".to_string();
            }
            "Applying" => {
                self.busy = true;
                self.status_text = "正在备份并覆盖安装…".to_string();
            }
            "Failed" => {
                self.busy = false;
                self.status_text = format!("更新失败：{}", frame.error.unwrap_or_default());
            }
            _ => {}
        }
    }
}

struct DemoApp {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    updater_exe: PathBuf,
    config_path: PathBuf,
}

/// 取 updater 可执行文件路径：与本程序同目录，Windows 自动加 `.exe`。
fn updater_path() -> PathBuf {
    let mut base = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    base.pop();
    let name = if cfg!(windows) { "generic-updater.exe" } else { "generic-updater" };
    base.join(name)
}

fn load_config() -> AppConfig {
    let mut cfg_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    cfg_path.pop();
    cfg_path.push("app.json");
    if let Ok(text) = std::fs::read_to_string(&cfg_path) {
        if let Ok(cfg) = serde_json::from_str::<AppConfig>(&text) {
            return cfg;
        }
    }
    AppConfig::default()
}

#[cfg(windows)]
fn install_windows_cjk_font(ctx: &egui::Context) -> Result<(), String> {
    let fonts_dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .map(|path| path.join("Fonts"))
        .ok_or_else(|| "WINDIR is not set".to_string())?;
    let font_path = ["simhei.ttf", "Deng.ttf", "msyh.ttf"]
        .iter()
        .map(|name| fonts_dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "No supported Chinese font found (tried SimHei, DengXian, and YaHei)".to_string()
        })?;
    let font_bytes = std::fs::read(&font_path)
        .map_err(|error| format!("Unable to read {}: {error}", font_path.display()))?;

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "windows_cjk".to_string(),
        egui::FontData::from_owned(font_bytes),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .get_mut(&family)
            .expect("default font family exists")
            .push("windows_cjk".to_string());
    }
    ctx.set_fonts(fonts);
    Ok(())
}

/// 拉起 updater 子进程，后台线程逐行读取 stdout 状态帧。
fn spawn_updater(
    shared: &Arc<Mutex<Shared>>,
    updater_exe: &PathBuf,
    config_path: &PathBuf,
    args: &[&str],
) -> bool {
    let applying = args.contains(&"--apply");
    let mut cmd = std::process::Command::new(updater_exe);
    cmd.args(args.iter().copied())
        .arg(config_path)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(
            updater_exe
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")),
        )
        .stdout(if applying {
            std::process::Stdio::null()
        } else {
            std::process::Stdio::piped()
        })
        .stderr(if applying {
            std::process::Stdio::null()
        } else {
            std::process::Stdio::piped()
        });

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let mut s = shared.lock().unwrap();
            s.busy = false;
            s.status_text = format!("无法启动 updater：{e}");
            s.push_log(format!("spawn 失败: {e}"));
            return false;
        }
    };

    if applying {
        drop(child);
        return true;
    }

    // stdout 帧线程
    let shared_out = shared.clone();
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(frame) = serde_json::from_str::<StatusFrame>(&line) {
                    shared_out.lock().unwrap().apply_frame(frame);
                } else {
                    shared_out.lock().unwrap().push_log(format!("stdout: {line}"));
                }
            }
        });
    }

    // stderr 日志线程
    let shared_err = shared.clone();
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                shared_err.lock().unwrap().push_log(format!("stderr: {line}"));
            }
        });
    }

    // 回收子进程，避免僵尸
    let shared_wait = shared.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        shared_wait.lock().unwrap().busy = false;
    });
    true
}

impl DemoApp {
    fn new(ctx: &egui::Context) -> Self {
        let config = load_config();
        let shared = Arc::new(Mutex::new(Shared::new(config)));

        // 新版启动成功后清除启动探测标记（updater 探测到标记消失即认定新版起来了）。
        let install_dir = shared.lock().unwrap().config.install_dir.clone();
        let cache_dir = PathBuf::from(if install_dir.is_empty() { "." } else { &install_dir })
            .join(".app-update");
        let marker = cache_dir.join(".launch-marker");
        let _ = std::fs::remove_file(marker);

        DemoApp {
            shared,
            ctx: ctx.clone(),
            updater_exe: updater_path(),
            config_path: std::env::current_exe()
                .map(|mut p| { p.pop(); p.push("app.json"); p })
                .unwrap_or_else(|_| PathBuf::from("app.json")),
        }
    }

    fn run_stage(&self, args: &[&str]) -> bool {
        {
            let mut s = self.shared.lock().unwrap();
            s.busy = true;
        }
        let started = spawn_updater(&self.shared, &self.updater_exe, &self.config_path, args);
        self.ctx.request_repaint_after(Duration::from_millis(100));
        started
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 先取状态快照并释放锁，避免锁守卫跨 UI 闭包移动。
        let snap = {
            let s = self.shared.lock().unwrap();
            Snapshot {
                current: s.current_version.clone(),
                latest: s.latest_version.clone(),
                notes: s.notes.clone(),
                received: s.received,
                total: s.total,
                phase: s.phase.clone(),
                status: s.status_text.clone(),
                busy: s.busy,
                log: s.log.iter().cloned().collect(),
            }
        };

        let mut action: Option<&[&str]> = None;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("MyShot 更新测试壳");
            ui.label(egui::RichText::new("外置 updater 对接演示（protocol:2 JSON 帧）").weak());
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("当前版本：");
                ui.label(egui::RichText::new(&snap.current).strong());
                if !snap.latest.is_empty() {
                    ui.label(format!("→ 最新版本：{}", snap.latest));
                }
            });

            ui.add_space(8.0);

            // 按钮区：按状态机控制可用性，点击只记录动作，渲染结束后再拉起子进程。
            ui.horizontal(|ui| {
                let check_enabled = !snap.busy;
                if ui.add_enabled(check_enabled, egui::Button::new("检查更新")).clicked() {
                    action = Some(&["--manual"]);
                }

                let update_enabled = !snap.busy && !snap.latest.is_empty() && snap.phase == "Available";
                if ui.add_enabled(update_enabled, egui::Button::new("立即更新")).clicked() {
                    action = Some(&["--download"]);
                }

                let restart_enabled = !snap.busy && snap.phase == "Ready";
                if ui.add_enabled(restart_enabled, egui::Button::new("重启软件")).clicked() {
                    action = Some(&["--apply"]);
                }
            });

            ui.add_space(8.0);
            ui.label(&snap.status);

            // 进度条
            if snap.total > 0 {
                let ratio = (snap.received as f32 / snap.total as f32).clamp(0.0, 1.0);
                ui.add(egui::ProgressBar::new(ratio).show_percentage());
                ui.label(format!("{} / {} 字节", snap.received, snap.total));
            }

            // 变更说明
            if !snap.notes.is_empty() {
                ui.add_space(8.0);
                ui.label("变更说明：");
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .show(ui, |ui| {
                        ui.label(&snap.notes);
                    });
            }

            ui.separator();
            ui.label("运行日志");
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for line in &snap.log {
                        ui.label(line);
                    }
                });
        });

        if let Some(args) = action {
            let applying = args.contains(&"--apply");
            if self.run_stage(args) && applying {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        // 子进程帧到达后持续重绘
        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

struct Snapshot {
    current: String,
    latest: String,
    notes: String,
    received: u64,
    total: u64,
    phase: String,
    status: String,
    busy: bool,
    log: Vec<String>,
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([520.0, 640.0])
            .with_title("MyShot 更新测试壳"),
        ..Default::default()
    };
    eframe::run_native(
        "demo-app",
        options,
        Box::new(|cc| {
            #[cfg(windows)]
            let font_error = install_windows_cjk_font(&cc.egui_ctx).err();
            #[cfg(not(windows))]
            let font_error: Option<String> = None;
            let app = DemoApp::new(&cc.egui_ctx);
            if let Some(error) = font_error {
                app.shared
                    .lock()
                    .unwrap()
                    .push_log(format!("Chinese font unavailable: {error}"));
            }
            Ok(Box::new(app))
        }),
    )
}
