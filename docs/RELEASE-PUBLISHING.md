# 发布规范（对标 Snow Shot Release 结构）

> 本文档规定：在 GitHub / Gitee Release 页面怎么填标题、标签、更新说明，上传哪些资产，
> `latest-version.json` 信封长什么样，每个 `.sha256` 文件怎么来。updater 客户端按这套约定解析。

## 1. Release 页面填什么

### 1.1 标签（Tag）

格式：`v<版本号>_<产品名>`，示例：

```
v1.5.7_snow-shot
```

- 版本号用 semver：`主.次.补丁`，如 `1.5.7`；
- 预发布加后缀：`v1.6.0-rc.1_snow-shot`；
- updater 只从 tag 解析版本号做比较，产品名后缀仅用于标识。

### 1.2 标题（Release Title）

```
Snow Shot 1.5.7
```

规则：`<产品名> <版本号>`，不带 `v` 前缀。

### 1.3 更新说明（Description / Notes）

正文用 markdown，**分语言分段**，updater 读取后原样回传给主程序展示：

```markdown
## 中文

- 绘制或编辑直线时，端点会自动吸附到附近直线的端点。
- 跟随鼠标录屏时，取景框跟随得更及时。
- 截图失败后，应用能重新建立屏幕捕获会话。

## English

- Line endpoints snap to nearby line endpoints as you draw.
- The recording outline responds faster in mouse-follow mode.
```

- 这就是 updater 状态帧里的 `notes` 字段来源；
- 说明文本只用于展示，不参与安装决策；安装用哪个包、哈希多少，只认签名后的清单。

### 1.4 是否预发布

- 正式版：不勾选 Prerelease；
- RC / 测试版：勾选 Prerelease，对应预发布版本号，稳定频道不更新到这类版本。

## 2. 上传哪些资产（Windows x64 标准版为例）

| 类别 | 资产文件名 | 配套哈希文件 | 用途 |
|---|---|---|---|
| 安装包 online | `<product>-<version>-windows-x64-online.exe` | 同名 `.exe.sha256` | 全新安装 |
| 安装包 offline | `<product>-<version>-windows-x64-offline.exe` | 同名 `.exe.sha256` | 离线全新安装 |
| 更新包 online | `<product>-<version>-windows-x64-online-update.zip` | 同名 `.zip.sha256` | 已装用户增量/全量更新 |
| 更新包 offline | `<product>-<version>-windows-x64-offline-update.zip` | 同名 `.zip.sha256` | 离线更新 |
| 便携包 | `<product>-<version>-windows-x64-portable.zip` | 同名 `.zip.sha256` | 绿色免安装 |
| 元数据信封 | `latest-version.json` | — | updater 检查更新的入口清单 |
| （arm64） | 同上，把 `x64` 换成 `arm64`，信封 `latest-version-windows-arm64.json` | — | 另一架构 |

真实示例（Snow Shot 1.5.7，x64）：

```
snow-shot-1.5.7-windows-x64-online.exe
snow-shot-1.5.7-windows-x64-online.exe.sha256
snow-shot-1.5.7-windows-x64-online-update.zip
snow-shot-1.5.7-windows-x64-online-update.zip.sha256
snow-shot-1.5.7-windows-x64-portable.zip
snow-shot-1.5.7-windows-x64-portable.zip.sha256
latest-version.json
```

> 源码包 `v1.5.7_snow-shot.zip / .tar.gz` 是平台自动生成的，不用手动上传。

### 2.1 本仓库 demo-app 的 CI 资产

本仓库的 Windows demo 工作流（`.github/workflows/build.yml`）发布：

```
demo-app-windows-x64-demo.zip
demo-app-windows-x64-demo.zip.sha256
latest-version.json
```

`demo-app-windows-x64-demo.zip` 是可直接运行的演示包，包含 GUI 主程序、外置 updater、`app.json` 和 `version.txt`。安装时主程序会退出释放自身文件，updater 等待主程序退出，并跳过覆盖自身正在运行的 updater exe。Release 只上传此 zip、其 SHA256 文件和 `latest-version.json`；清单将 zip 标记为 portable 演示包。Release 正文按 `## 中文` / `## English` 分段。`latest-version.json` 由 `scripts/build-metadata.sh` 生成但不签名；因为当前 demo 配置使用 Release API 源且没有内置可信公钥，不能将它配置为强制验签的 metadata 源。此 demo 工作流不生成真实 installer、offline 包或独立更新包。

## 3. `.sha256` 文件格式

每个二进制/压缩包旁边放一个同名 `.sha256`，内容一行：

```
<c64位小写哈希>  <文件名>
```

示例 `snow-shot-1.5.7-windows-x64-portable.zip.sha256`：

```
c2474bce2e30eb73243e15c689720d47e31455396b7d1a1d15efdd974097bd1d  snow-shot-1.5.7-windows-x64-portable.zip
```

生成命令（Linux/macOS/Git Bash）：

```bash
sha256sum snow-shot-1.5.7-windows-x64-portable.zip > \
          snow-shot-1.5.7-windows-x64-portable.zip.sha256
```

## 4. `latest-version.json` 信封结构

这是 updater 检查更新时下载的第一个文件。外层是**签名信封**，内层 payload 是 base64 编码的清单 JSON。

### 4.1 外层

```json
{
  "schema": 1,
  "keyId": "snow-shot-2026-01",
  "payload": "<base64: 下面的 payload>",
  "signature": "<base64: 私钥对 payload 的签名>"
}
```

| 字段 | 含义 |
|---|---|
| `schema` | 信封格式版本，当前 1 |
| `keyId` | 签名密钥标识，客户端按它在内置公钥列表里找对应公钥验签 |
| `payload` | base64 编码的清单 JSON（见 4.2） |
| `signature` | RSA-PSS 签名：先对**解码后的 payload 明文字节**做 SHA256 摘要，再 PSS（salt 长度 32）签名，base64 编码；无签名阶段可留空，客户端走 Release API 源、仅做哈希校验 |

### 4.2 payload（base64 解码后）

```json
{
  "schema": 1,
  "product": "snow-shot",
  "version": "1.5.7",
  "publishedAt": "2026-11-13T16:48:11Z",
  "platform": "windows-x64",
  "packages": [
    {
      "variant": "online",
      "kind": "installer",
      "path": "setup/snow-shot_windows-x64-online.exe",
      "size": 25137773,
      "sha256": "c477c55b..."
    },
    {
      "variant": "online",
      "kind": "update",
      "path": "setup/snow-shot_windows-x64-online-update.zip",
      "size": 37764164,
      "sha256": "c2474bce...",
      "files": [
        { "path": "bin/snow_shot.exe", "size": 83513344, "sha256": "af651121..." },
        { "path": "bin/assets/ocr/asset-manifest.json", "size": 4371, "sha256": "087f0f23..." }
      ]
    },
    {
      "variant": "portable",
      "kind": "portable",
      "path": "setup/snow-shot_windows-x64-portable.zip",
      "size": 75429901,
      "sha256": "..."
    }
  ]
}
```

| 字段 | 含义 |
|---|---|
| `product` | 产品标识，必须和客户端编译期身份一致，防止装错产品 |
| `version` | 新版本 semver |
| `publishedAt` | 发布时间 RFC3339 |
| `platform` | `windows-x64` / `windows-arm64` / `macos-arm64` ... |
| `packages[].variant` | `online` / `offline` / `portable` |
| `packages[].kind` | `installer`（安装包）/ `update`（更新包）/ `portable`（便携包） |
| `packages[].path` | 资产在 Release 里的相对路径 / 文件名 |
| `packages[].size` | 字节数 |
| `packages[].sha256` | 整包哈希 |
| `packages[].files[]` | 可选：更新包内逐文件清单（路径/大小/哈希）。本方案走**整包覆盖**，默认不生成、不做逐文件增量 |

### 4.3 签名与公钥托管

- 私钥只留在发布 CI secret / 离线保管，**绝不进客户端仓库或安装包**；
- 公钥（模数 + 指数，big-endian base64）编译期嵌入 updater 的 `TRUSTED_KEYS`；
- `keyId` 轮换时，客户端内置新旧两把公钥做平滑过渡。

**一次性生成密钥对**（`scripts/gen-keys.sh`）：

```bash
KEY_ID=prod-2026 KEY_DIR=./keys ./scripts/gen-keys.sh
# 产物：keys/private.pem（离线保管）、keys/public.pem（备查）
# 终端打印一段 Rust 片段，填入 src/signature.rs 的 TRUSTED_KEYS
```

**每次发布签名**（`scripts/build-metadata.sh`，同时生成各资产 `.sha256` 与信封）：

```bash
PRODUCT=my-shot VERSION=1.5.7 PLATFORM=windows-x64 \
  ARTIFACT_DIR=./dist OUT_DIR=./dist \
  KEY_ID=prod-2026 PRIVATE_KEY_PEM=./keys/private.pem \
  ./scripts/build-metadata.sh
```

未提供 `PRIVATE_KEY_PEM` 时信封 `signature` 留空，开发期 updater 配置不指定 `metadata_file`，走 Release API 源，不强制验签。

### 4.4 多平台 / 多版本元数据文件命名

同一 Release 下按架构、版本线放多份信封，updater 配置里用 `metadata_file` 指定本产品读取哪一份：

| 元数据文件 | 适用 |
|---|---|
| `latest-version.json` | Windows x64 标准版（默认） |
| `latest-version-mini.json` | Windows x64 精简版 |
| `latest-version-windows-arm64.json` | Windows ARM64 标准版 |
| `latest-version-mini-windows-arm64.json` | Windows ARM64 精简版 |

本阶段只做 Windows x64，发布时只需 `latest-version.json`。其他架构照上表复制一份脚本产物、改 `platform` 与资产名即可。

## 5. 安装目录与临时文件生命周期（客户端视角）

```
<install_dir>/
├── app.exe                     # 主程序（运行中被占用）
├── version.txt                 # 本地已装版本号
├── .app-update/                # 隐藏暂存目录（updater 自建）
│   ├── app.exe.new.part        # 下载中途的残片
│   ├── app.exe.new             # 下载校验完成的新包
│   ├── staging/                # 解压展开的新文件
│   └── backup/                 # 被替换旧文件的备份（回滚用）
├── app.exe.old                 # 旁置的上一版本（单 exe 场景）
└── app.exe.blocked             # 被策略拦截/起不来的版本，保留备查
```

生命周期规则：

| 场景 | 处理 |
|---|---|
| 下载中断/取消 | 删除 `.part` 残片，不碰安装目录 |
| 哈希不符 | 删除 `.new`，报错退出 |
| 安装中途失败 | 从 `backup/` 逐文件还原；清理 `staging/` |
| 新版启动成功 | 删除 `backup/`、`staging/`、`.old` |
| 新版起不来 | `.old` 移回原位，新文件改 `.blocked`，记录 blocked |

## 6. 发布步骤

### 6.1 手动发布

1. 编译各平台产物；
2. 对每个 exe/zip 算 `.sha256`；
3. 汇总清单生成 payload JSON，base64 编码，私钥签名后组装 `latest-version.json`（脚本见 `scripts/build-metadata.sh`）；
4. 在 Release 页面填 tag、标题、中英文 notes；
5. 上传所有 exe/zip、`.sha256`、`latest-version.json`；
6. 发布后用 updater `--check` 验证能识别新版本。

### 6.2 CI 自动发布（推荐）

`.github/workflows/release.yml` 流程：

1. 触发：push `v*` tag；
2. 编译 Windows x64 / arm64 产物；
3. 打包 installer / update zip / portable zip；
4. 计算每个资产 SHA256，生成 `.sha256`；
5. 生成 `latest-version.json` 信封（签名私钥从 secrets 取）；
6. 用 Release Action 上传全部资产；
7. Release 标题、正文从 tag 和 changelog 文件自动生成。

## 7. 版本号规则

- 新功能兼容升级：次版本 +1（`1.5.7 → 1.6.0`）；
- 缺陷修复：补丁 +1（`1.5.7 → 1.5.8`）；
- 不兼容变更：主版本 +1（`1.5.7 → 2.0.0`）；
- updater 只在远端版本 **严格大于** 本地版本时提示更新。
