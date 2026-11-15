use crate::download::sha256_file;
use crate::error::AppError;
use crate::journal::JournalFile;
use std::path::Path;
use walkdir::WalkDir;
use zip::ZipArchive;

/// 一次安装的上下文，回滚和清理都靠它。
pub struct InstallContext {
    pub staging: std::path::PathBuf,
    pub backup: std::path::PathBuf,
    pub target_root: std::path::PathBuf,
}

/// 把 zip 解压到 staging 目录；exe 资产则直接复制进去。
pub fn prepare_staging(
    asset_path: &Path,
    staging: &Path,
) -> Result<(), AppError> {
    std::fs::create_dir_all(staging)?;
    let ext = asset_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();

    match ext.as_str() {
        "zip" => {
            let file = std::fs::File::open(asset_path)?;
            let mut archive = ZipArchive::new(file)
                .map_err(|e| AppError::Install(format!("zip 打开失败: {e}")))?;
            for i in 0..archive.len() {
                let mut entry = archive
                    .by_index(i)
                    .map_err(|e| AppError::Install(format!("zip 条目读取失败: {e}")))?;
                let out = staging.join(entry.mangled_name());
                if entry.is_dir() {
                    std::fs::create_dir_all(&out)?;
                } else {
                    if let Some(p) = out.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    let mut out_file = std::fs::File::create(&out)?;
                    std::io::copy(&mut entry, &mut out_file)
                        .map_err(|e| AppError::Install(format!("解压写入失败: {e}")))?;
                }
            }
        }
        "exe" => {
            let name = asset_path
                .file_name()
                .ok_or_else(|| AppError::Install("exe 资产缺少文件名".to_string()))?;
            std::fs::copy(asset_path, staging.join(name))?;
        }
        other => {
            return Err(AppError::Install(format!("不支持的资产类型: .{other}")));
        }
    }
    Ok(())
}

/// 逐文件备份旧版本到 backup，再把 staging 覆盖到 target_root。
/// 返回安装上下文与本次涉及的文件清单（用于 journal 事务记录）。
pub fn install(
    staging: &Path,
    target_root: &Path,
    backup: &Path,
) -> Result<(InstallContext, Vec<JournalFile>), AppError> {
    std::fs::create_dir_all(target_root)?;
    std::fs::create_dir_all(backup)?;
    let running_exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok());

    let mut files: Vec<JournalFile> = Vec::new();

    for entry in WalkDir::new(staging) {
        let entry = entry.map_err(|e| AppError::Install(format!("遍历暂存目录失败: {e}")))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(staging)
            .map_err(|e| AppError::Install(e.to_string()))?;
        let target = target_root.join(rel);
        if running_exe
            .as_ref()
            .zip(target.canonicalize().ok().as_ref())
            .is_some_and(|(running, target)| running == target)
        {
            continue;
        }
        let backup_file = backup.join(rel);

        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p)?;
        }

        let existed = target.exists();
        let mut record = JournalFile {
            path: rel.to_string_lossy().to_string(),
            existed,
            size: None,
            sha256: None,
        };

        // 旧文件先备份，记录大小与哈希，再覆盖。
        if existed {
            if let Some(p) = backup_file.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(&target, &backup_file)?;
            record.size = std::fs::metadata(&target).map(|m| m.len()).ok();
            record.sha256 = sha256_file(&target).ok();
        }
        std::fs::copy(entry.path(), &target)?;
        files.push(record);
    }

    Ok((
        InstallContext {
            staging: staging.to_path_buf(),
            backup: backup.to_path_buf(),
            target_root: target_root.to_path_buf(),
        },
        files,
    ))
}

/// 按备份目录把旧文件还原回 target_root。
pub fn rollback(ctx: &InstallContext) -> Result<(), AppError> {
    for entry in WalkDir::new(&ctx.backup) {
        let entry = entry.map_err(|e| AppError::Install(format!("遍历备份目录失败: {e}")))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&ctx.backup)
            .map_err(|e| AppError::Install(e.to_string()))?;
        let target = ctx.target_root.join(rel);
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(entry.path(), &target)?;
    }
    Ok(())
}

/// 安装并启动成功后，清理暂存与备份。
pub fn cleanup(ctx: &InstallContext) {
    let _ = std::fs::remove_dir_all(&ctx.staging);
    let _ = std::fs::remove_dir_all(&ctx.backup);
}

#[cfg(test)]
mod tests {
    use super::install;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn install_skips_the_updater_executable_while_it_is_running() {
        let running_exe = std::env::current_exe().unwrap();
        let target_root = running_exe.parent().unwrap();
        let exe_name = running_exe.file_name().unwrap();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp = std::env::temp_dir().join(format!(
            "generic-updater-install-{}-{timestamp}",
            std::process::id()
        ));
        let staging = temp.join("staging");
        let backup = temp.join("backup");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join(exe_name), b"replacement").unwrap();

        let original = std::fs::read(&running_exe).unwrap();
        let (_, files) = install(&staging, target_root, &backup).unwrap();

        assert!(files.is_empty());
        assert_eq!(std::fs::read(&running_exe).unwrap(), original);
        std::fs::remove_dir_all(temp).unwrap();
    }
}
