use serde::Deserialize;
use crate::error::AppError;
use crate::signature::{expect_product, verify_envelope};

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: semver::Version,
    pub notes: String,
    pub asset: Asset,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

const DEFAULT_GITHUB: &str = "https://api.github.com/repos";

/// 请求最新 Release 并匹配指定资产名。
///
/// 源顺序：先打 `{base}/repos/{repo}/releases/latest`；该端点不可用（404 / 限流 /
/// 无 latest）时回退 `{base}/repos/{repo}/releases` 列表取首个。
/// 全程不拼 `/releases/tags/{tag}`，因此不会触发 "GitHub Releases requires a tag"。
/// 哈希优先取 API digest，缺失时兜底下载同名 `.sha256` 资产解析。
pub fn fetch_latest(
    client: &reqwest::blocking::Client,
    repo: &str,
    asset_name: &str,
    feed_base: Option<&str>,
) -> Result<ReleaseInfo, AppError> {
    let base = feed_base.unwrap_or(DEFAULT_GITHUB).trim_end_matches('/');

    // 路径一：latest 端点（不需要 tag）。
    let latest_url = format!("{base}/{repo}/releases/latest");
    if let Ok(release) = get_release_json(client, &latest_url) {
        return build_release(client, release, asset_name);
    }

    // 路径二：列表端点，取第一个（按发布时间倒序）。
    let list_url = format!("{base}/{repo}/releases?per_page=10");
    let list: Vec<ApiRelease> = client
        .get(&list_url)
        .header("Accept", "application/json")
        .header("User-Agent", "generic-updater/0.1")
        .send()
        .map_err(|e| AppError::Network(format!("请求 Release 列表失败: {e}")))?
        .json()
        .map_err(|e| AppError::Network(format!("Release 列表 JSON 解析失败: {e}")))?;

    let release = list
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Verify("仓库没有任何已发布 Release".to_string()))?;

    build_release(client, release, asset_name)
}

fn get_release_json(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<ApiRelease, AppError> {
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", "generic-updater/0.1")
        .send()
        .map_err(|e| AppError::Network(format!("请求 Release 失败: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Network(format!(
            "Release 接口返回状态 {}",
            resp.status()
        )));
    }
    resp.json()
        .map_err(|e| AppError::Network(format!("Release JSON 解析失败: {e}")))
}

fn build_release(
    client: &reqwest::blocking::Client,
    release: ApiRelease,
    asset_name: &str,
) -> Result<ReleaseInfo, AppError> {
    let version = parse_tag(&release.tag_name)?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            AppError::Verify(format!(
                "Release 中找不到资产 `{asset_name}`；请核对发布页资产名与配置 asset_name 是否一致"
            ))
        })?;

    let mut sha256 = asset
        .digest
        .as_ref()
        .and_then(|d| d.strip_prefix("sha256:").map(|s| s.to_string()));

    // digest 缺失：下载同名 `<asset>.sha256` 解析，内容格式 `<hash>  <filename>`。
    if sha256.is_none() {
        let sha_name = format!("{asset_name}.sha256");
        if let Some(sha_asset) = release.assets.iter().find(|a| a.name == sha_name) {
            let text = client
                .get(&sha_asset.browser_download_url)
                .header("User-Agent", "generic-updater/0.1")
                .send()
                .map_err(|e| AppError::Network(format!("读取 {sha_name} 失败: {e}")))?
                .text()
                .map_err(|e| AppError::Network(format!("{sha_name} 解析失败: {e}")))?;
            sha256 = parse_sha256_line(&text);
        }
    }

    Ok(ReleaseInfo {
        version,
        notes: release.body.unwrap_or_default(),
        asset: Asset {
            name: asset.name.clone(),
            url: asset.browser_download_url.clone(),
            size: asset.size,
            sha256,
        },
    })
}

/// 从 tag 提取语义版本：`v1.5.7_snow-shot` / `1.5.7-offline` → `1.5.7`。
fn parse_tag(tag: &str) -> Result<semver::Version, AppError> {
    let t = tag.trim().trim_start_matches('v');
    let core = t.split(|c| c == '_' || c == '-').next().unwrap_or(t);
    semver::Version::parse(core)
        .map_err(|e| AppError::Verify(format!("tag 版本号无法解析 `{tag}`: {e}")))
}

/// 签名信封源：拉取 `metadata_file` 信封 → RSA-PSS 验签 → 比对产品 → 取版本与资产。
///
/// 信封外层结构 `{schema,keyId,payload(base64),signature}`；payload 明文为
/// `{product,version,packages:[{kind,path,size,sha256,...}]}`。
pub fn fetch_envelope_release(
    client: &reqwest::blocking::Client,
    repo: &str,
    metadata_file: &str,
    product: &str,
    asset_name: &str,
    feed_base: Option<&str>,
) -> Result<ReleaseInfo, AppError> {
    let base = feed_base.unwrap_or(DEFAULT_GITHUB).trim_end_matches('/');

    // 先取 latest Release，用于定位信封资产与资产下载地址。
    let release = latest_api_release(client, repo, base)?;

    let envelope_asset = release
        .assets
        .iter()
        .find(|a| a.name == metadata_file)
        .ok_or_else(|| {
            AppError::Verify(format!("Release 中找不到信封资产 `{metadata_file}`"))
        })?;

    let envelope_bytes = client
        .get(&envelope_asset.browser_download_url)
        .header("User-Agent", "generic-updater/0.1")
        .send()
        .map_err(|e| AppError::Network(format!("下载信封失败: {e}")))?
        .bytes()
        .map_err(|e| AppError::Network(format!("信封读取失败: {e}")))?;

    let signed = verify_envelope(&envelope_bytes)?;
    expect_product(&signed, product)?;

    let version_raw = signed
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Verify("信封缺少 version 字段".to_string()))?;
    let version = semver::Version::parse(version_raw)
        .map_err(|e| AppError::Verify(format!("信封版本号无法解析 `{version_raw}`: {e}")))?;

    let packages = signed
        .get("packages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AppError::Verify("信封缺少 packages 清单".to_string()))?;

    // 选目标包：优先文件名命中 asset_name，其次取第一个 kind=update 的包。
    let pkg = packages
        .iter()
        .find(|p| {
            p.get("path")
                .and_then(|v| v.as_str())
                .map(|s| std::path::Path::new(s).file_name().and_then(|f| f.to_str()) == Some(asset_name))
                .unwrap_or(false)
        })
        .or_else(|| packages.iter().find(|p| p.get("kind").and_then(|v| v.as_str()) == Some("update")))
        .ok_or_else(|| AppError::Verify("信封 packages 中没有可用更新包".to_string()))?;

    let pkg_path = pkg
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Verify("更新包缺少 path 字段".to_string()))?;
    let pkg_name = std::path::Path::new(pkg_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(pkg_path)
        .to_string();
    let size = pkg.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    let sha256 = pkg.get("sha256").and_then(|v| v.as_str()).map(|s| s.to_string());

    // 资产下载地址从 Release assets 按文件名匹配。
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == pkg_name)
        .ok_or_else(|| {
            AppError::Verify(format!("Release 中找不到更新资产 `{pkg_name}`"))
        })?;

    Ok(ReleaseInfo {
        version,
        notes: release.body.unwrap_or_default(),
        asset: Asset {
            name: asset.name.clone(),
            url: asset.browser_download_url.clone(),
            size,
            sha256,
        },
    })
}

/// 取 latest Release，失败回退列表首个。
fn latest_api_release(
    client: &reqwest::blocking::Client,
    repo: &str,
    base: &str,
) -> Result<ApiRelease, AppError> {
    let latest_url = format!("{base}/{repo}/releases/latest");
    if let Ok(r) = get_release_json(client, &latest_url) {
        return Ok(r);
    }
    let list_url = format!("{base}/{repo}/releases?per_page=10");
    let list: Vec<ApiRelease> = client
        .get(&list_url)
        .header("Accept", "application/json")
        .header("User-Agent", "generic-updater/0.1")
        .send()
        .map_err(|e| AppError::Network(format!("请求 Release 列表失败: {e}")))?
        .json()
        .map_err(|e| AppError::Network(format!("Release 列表 JSON 解析失败: {e}")))?;
    list.into_iter()
        .next()
        .ok_or_else(|| AppError::Verify("仓库没有任何已发布 Release".to_string()))
}

/// 解析 `.sha256` 文件一行：`<hash>[空格/两个空格]filename`。
fn parse_sha256_line(text: &str) -> Option<String> {
    let first = text.lines().find(|l| !l.trim().is_empty())?;
    let hash = first.split_whitespace().next()?;
    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hash.to_string())
    } else {
        None
    }
}
