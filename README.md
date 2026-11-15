# generic-updater：外置通用更新器（Rust）

一个被任意语言主程序调用的独立更新器：基于 GitHub Release 做版本检查、下载、SHA256 校验、旁置替换、失败回滚。架构对标 snow-shot 的外置 updater。

## 0. 先回答最容易困惑的问题：我电脑没装 Rust，能开发/编译吗？

**分清两个角色：**

- **最终用户电脑**：永远不需要 Rust。updater 是开发者提前编译好的 `generic-updater.exe`，随软件一起分发，用户只运行这个成品 exe。
- **你（开发者）电脑**：要把 Rust 源码变成 exe，必须经过一次编译。但编译不一定要在你本地做，有两条路。

### 路径 A：本地装 Rust 工具链编译（一次性安装）

1. 装 Rust 工具链（rustup）：https://rustup.rs ，安装时选默认 stable 工具链；
2. Windows 还需要 **MSVC C++ 生成工具 + Windows SDK**：装 Visual Studio Installer，勾选"使用 C++ 的桌面开发"（rustup 的 msvc 目标依赖它）；
3. 打开终端，在项目目录执行：

```bash
cargo build --release
```

4. 产物在：

```
target\release\generic-updater.exe
```

> 想明确指定 Windows 目标：`cargo build --release --target x86_64-pc-windows-msvc`，产物在 `target\x86_64-pc-windows-msvc\release\`。

### 路径 B：GitHub Actions 云端编译（本地零环境，推荐不想装工具链时用）

仓库已带 `.github/workflows/build.yml`：

- 推一个 `v*` tag；或在 Actions 页面手动运行，并在 `tag_name` 中填写要构建发布的 tag（例如 `v0.1.0`）；
- GitHub 的 `windows-latest` 云端机器自带 VS 工具链和 Rust 环境；
- 自动 `cargo build --release`，把 `generic-updater.exe` 挂到对应 Release 资产。

你本地只需要写代码、push，不用装 Rust、不用装 VS。

### 当前验证状态

- 本项目已在 Linux 环境 `cargo build --release` 通过，二进制可运行；
- Windows `.exe` 在 Windows runner（路径 B）或本地 MSVC 工具链（路径 A）产出；代码里 Windows 专属行为用标准库实现，未引入额外系统依赖。

## 1. 项目结构

```
rust-updater/
├── Cargo.toml              # 依赖与 release 优化配置
├── config.example.json      # 每软件一份的配置模板
├── .github/workflows/build.yml
├── docs/RELEASE-PUBLISHING.md  # 发布页/资产命名/.sha256/latest-version.json 信封规范
├── scripts/
│   ├── build-metadata.sh    # 汇总产物哈希、生成 .sha256 与签名信封
│   └── gen-keys.sh          # 生成 RSA 密钥对，导出可内置的公钥片段
├── demo-app/                # egui 极简测试主程序（独立 crate，子进程对接 updater）
└── src/
    ├── main.rs             # 入口：解析命令行参数
    ├── config.rs           # 配置读取、本地版本号读写
    ├── protocol.rs         # ↔ 主程序的 JSON 帧协议（状态/进度/版本/变更说明）
    ├── feed.rs             # Release API 源 + 签名信封源拉取、版本匹配、.sha256 兜底
    ├── download.rs         # 流式下载 .part、断点续传、取消、SHA256 校验、进度回调
    ├── install.rs          # 解压 staging、逐文件备份覆盖、回滚、清理
    ├── journal.rs          # 事务日志、断电恢复、启动探测标记
    ├── signature.rs         # RSA-PSS 信封验签、产品标识比对
    ├── update.rs           # 编排：检查 → 下载 → 校验 → 安装 → 探测 → 重启清理
    └── error.rs            # 统一错误类型
```

## 2. 命令行用法（三阶段，对应主程序的按钮流程）

```bash
# 自动检查：受 24h 间隔与 auto_check 开关限制，间隔内不联网
generic-updater.exe --check   app.json

# 手动刷新：用户点"刷新"，强制联网检查，不受间隔限制
generic-updater.exe --manual  app.json

# 下载阶段：检查 → 下载 → 校验 → 解压到 staging，停在 Ready，等主程序弹"重启软件"按钮
generic-updater.exe --download app.json

# 安装阶段：复用 staging，等待旧进程退出 → 备份覆盖 → 写版本号 → 拉起新版 → 清理
generic-updater.exe --apply   app.json --parent-pid 12345
```

| 参数 | 含义 |
|---|---|
| `--check` | 自动检查（间隔判断后联网） |
| `--manual` | 手动刷新检查（强制联网） |
| `--download` | 下载 + 校验 + staging 就位，停在 Ready |
| `--apply` | 安装覆盖 + 重启 + 清理 |
| `<config.json>` | 本软件的配置文件路径 |
| `--parent-pid <pid>` | 主程序 PID，用于等待旧进程退出（Windows 生产版接 OpenProcess 等待） |

**退出码**：`0` = 无更新 / 间隔内跳过；`2` = 有更新 / 更新成功；`1` = 失败。

### 关于报错 "GitHub Releases requires a tag"

这个报错来自"按 tag 取 Release"的接口（`/releases/tags/{tag}`），当 tag 变量为空时抛出。本项目**全程不拼 tag 接口**：先打 `/releases/latest`（不需要 tag），失败再回退 `/releases` 列表取首个；tag 名（如 `v1.5.7_snow-shot`）只用于提取语义版本号。若你在别的生成代码里看到这个报错，检查：① 是否误用了 `/releases/tags/` 端点；② 配置里 `repo` / `asset_name` 是否为空；③ 是否把 release 标签名当成了下载资产名。

## 3. 配置文件字段

```json
{
  "repo": "tahhans/update",
  "asset_name": "generic-updater.exe",
  "version_file": "version.txt",
  "install_dir": ".",
  "exe_name": "generic-updater.exe",

  "feed_url": null,
  "check_interval_secs": 86400,
  "auto_check": true,
  "state_file": null,
  "product": "generic-updater",
  "metadata_file": null,
  "launch_probe_secs": 10
}
```

| 字段 | 含义 |
|---|---|
| `repo` | 仓库 `owner/name`（GitHub 或 Gitee） |
| `asset_name` | 本平台资产文件名（`.zip` 目录包或 `.exe` 单文件） |
| `version_file` | 本地版本号文件，一行 `v1.2.3` |
| `install_dir` | 安装根目录，建议主程序传绝对路径 |
| `exe_name` | 单 exe 场景的目标程序名；zip 包用于定位主程序 |
| `feed_url` | 可选，更新源 API 基址；留空默认 GitHub，改 Gitee 填 `https://gitee.com/api/v5/repos` |
| `check_interval_secs` | 自动检查最小间隔（秒），默认 86400 = 24 小时 |
| `auto_check` | 是否允许启动时自动检查；手动刷新不受此开关限制 |
| `state_file` | 可选，检查状态文件路径，默认 `<install_dir>/.app-update/state.json` |
| `product` | 可选，产品标识；走信封源时与 payload.product 比对，防串包 |
| `metadata_file` | 可选，信封资产名（如 `latest-version.json`）；留空走 Release API 源 |
| `launch_probe_secs` | 可选，新版启动探测等待时长（秒），默认 10；超时判定启动失败并回滚 |

> **迁移到新项目**：updater 二进制不用改，只换一份 `app.json`（仓库、资产名、安装目录、程序名、间隔）即可。上次检查时间存在 `state.json`，跨启动仍遵守 24h 间隔。

## 4. 和主程序怎么通信（跨语言契约）

updater 启动后，**stdout 每行一条 JSON 状态帧**；主程序按行读取、解析后刷新自己的 UI：

```json
{"protocol":2,"type":"status","state":"Checking","version":"","notes":"","received":0,"total":0,"error":null}
{"protocol":2,"type":"status","state":"Available","version":"1.5.7","notes":"### 新功能\n- ...","received":0,"total":0,"error":null}
{"protocol":2,"type":"status","state":"Downloading","version":"1.5.7","notes":"...","received":1234567,"total":37764164,"error":null}
{"protocol":2,"type":"status","state":"Verifying","version":"1.5.7","notes":"...","received":0,"total":0,"error":null}
{"protocol":2,"type":"status","state":"Applying","version":"1.5.7","notes":"...","received":0,"total":0,"error":null}
{"protocol":2,"type":"status","state":"Ready","version":"1.5.7","notes":"...","received":1,"total":1,"error":null}
{"protocol":2,"type":"status","state":"Failed","version":"1.5.7","notes":"...","received":0,"total":0,"error":"失败原因"}
```

| 字段 | 用途 |
|---|---|
| `state` | 状态机阶段：Checking/Available/Downloading/Verifying/Ready/Applying/Failed |
| `version` | 检测到的新版本号 |
| `notes` | 变更说明（Release body，markdown），主程序渲染到更新弹窗 |
| `received` / `total` | 进度（下载阶段字节，安装阶段文件数） |
| `error` | 失败原因，成功为 null |

主程序侧只需要一个薄对接层：拉起进程 → 按行读 stdout JSON → 映射到自己的按钮/进度条/版本号/变更说明。

## 5. 主程序对接示例（任意语言伪代码）

```text
1. 主程序用自身路径 API 拿到安装目录（Windows: GetModuleFileNameW）
2. 写一份本软件的 app.json（repo / asset_name / install_dir / exe_name ...）
3. 启动时：拉起 --check；间隔内直接跳过，Available 才弹窗
4. 用户点"刷新"：拉起 --manual 强制检查
5. 用户点"立即更新"：拉起 --download；读 stdout：
   - Available   → 弹窗显示版本号 + notes（changelog）
   - Downloading → 进度条 received/total
   - Ready       → 出现"重启软件"按钮
6. 用户点"重启软件"：拉起 --apply --parent-pid <自己的PID>
   - Applying    → 备份覆盖
   - Ready       → 新版已拉起，临时文件已清理
   - Failed      → 显示 error，可重试；失败已自动回滚
```

## 5.1 测试主程序 demo-app（egui）

`demo-app/` 是一个独立 crate 的极简界面壳，演示任意语言主程序如何对接 updater：

- 界面：当前版本、「检查更新」按钮、变更说明区、下载进度条、「重启软件」按钮；
- 内部：后台拉起 `generic-updater` 子进程，逐行读 stdout 的 protocol:2 JSON 帧，刷新共享状态；
- 启动时删除 `.app-update/.launch-marker`（新版启动探测标记的清除方）；
- **不做代码级依赖**：主程序与 updater 是两个进程，只靠子进程 + JSON 帧通信，这正是外置架构的要求。

编译（需在带桌面环境的机器上；无头 Linux 缺 GUI 运行库时只做类型检查）：

```bash
cd demo-app
cargo build --release
# 产物 demo-app.exe；把编译好的 generic-updater.exe 放到同目录，
# 再放一份改好仓库/资产名的 app.json（模板见上级 config.example.json）。
```

## 6. 已实现 / 待扩展

**已实现**

- Release latest 检查 + semver 比较；latest 端点失败回退列表端点，全程不拼 tag 接口
- 自动检查 24h 间隔（state.json 记录上次检查时间）；手动刷新强制联网
- 三阶段编排：Check（检查）/ Download（下载校验就位）/ Apply（覆盖重启清理）
- 流式下载到 `.part`，大小 + SHA256 校验，不符丢弃
- 断点续传（Range/206 追加，200 覆盖）；下载取消删残片，不碰已装版本
- `.sha256` 同名资产兜底解析哈希；tag 名带后缀（`v1.5.7_snow-shot`）可提取版本
- zip / exe 两种资产；解压到 staging
- 逐文件备份到 backup，覆盖安装；失败按备份回滚
- 状态帧回传版本号、notes、进度、错误
- 退出码约定；detached 拉起新版；安装后清理 staging/backup/安装包
- RSA-PSS（SHA256，salt 32）信封验签：公钥按 keyId 内置，验签通过才解析 payload，并比对 product 防串包
- 事务 journal：安装前先恢复未决事务；记录文件清单；启动探测标记，新版未在窗口内清除标记则判定起不来、自动回滚并写 `.blocked`
- `demo-app/`：egui 极简测试主程序，独立 crate，子进程 + protocol:2 JSON 帧对接 updater

**待扩展（生产化时再补）**

- Windows 挂起启动探测（CREATE_SUSPENDED 区分策略拦截 / 文件占用）；当前 `wait_for_parent` 为非 Windows 空钩子
- 下载 stall 超时与退避重试
- 目录不可写时的 UAC 提权
- 单实例锁、卸载/启动项迁移
- 多源兜底（站点 feed → GitHub）

## 7. 发布侧配合

完整规范见 [`docs/RELEASE-PUBLISHING.md`](docs/RELEASE-PUBLISHING.md)，要点：

1. 打 tag `v1.2.3_product`，标题 `<产品名> 1.2.3`，正文分 `## 中文` / `## English` 写 changelog；
2. CI 编译产物，分别打包 installer exe / update zip / portable zip；
3. 每个资产算同名 `.sha256`（`<hash>  <filename>`）；
4. 用 `scripts/build-metadata.sh` 汇总哈希生成 `latest-version.json` 信封；
5. 全部资产 + 信封上传 Release；本地 `version.txt` 随包分发。
