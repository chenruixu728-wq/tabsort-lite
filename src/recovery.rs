use crate::bridge::valid_id;
use crate::protocol::Snapshot;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedLayout {
    version: u32,
    snapshot: Snapshot,
}

pub fn data_directory() -> Result<PathBuf> {
    if let Some(directory) = std::env::var_os("TABSORT_TEST_DATA") {
        return Ok(PathBuf::from(directory));
    }
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("找不到本地应用数据目录")?)
            .join("TabSort Lite"),
    )
}

fn recovery_path(profile: &str) -> Result<PathBuf> {
    if !valid_id(profile) {
        bail!("Edge 配置文件身份无效");
    }
    Ok(data_directory()?
        .join("recovery")
        .join(format!("{profile}.json")))
}

pub fn available(profile: &str) -> bool {
    recovery_path(profile).is_ok_and(|path| path.is_file())
}

pub fn save(snapshot: &Snapshot) -> Result<()> {
    write_snapshot(snapshot, &recovery_path(&snapshot.profile_id)?)
}

pub fn export(snapshot: &Snapshot) -> Result<PathBuf> {
    if !valid_id(&snapshot.profile_id) {
        bail!("Edge 配置文件身份无效");
    }
    let file = data_directory()?
        .join("exports")
        .join(format!("{}.json", snapshot.profile_id));
    write_snapshot(snapshot, &file)?;
    Ok(file)
}

fn write_snapshot(snapshot: &Snapshot, path: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("快照位置无效")?)?;
    let staging = path.with_extension("tmp");
    let mut file = File::create(&staging).context("无法创建本地快照")?;
    serde_json::to_writer_pretty(
        &mut file,
        &SavedLayout {
            version: 1,
            snapshot: snapshot.clone(),
        },
    )?;
    file.sync_all()?;
    drop(file);
    fs::rename(staging, path).context("无法写入本地快照")?;
    Ok(())
}

pub fn load(profile: &str) -> Result<Snapshot> {
    let saved: SavedLayout =
        serde_json::from_reader(File::open(recovery_path(profile)?).context("找不到整理前快照")?)?;
    if saved.version != 1 || saved.snapshot.profile_id != profile {
        bail!("快照版本或 Edge 配置文件不匹配");
    }
    Ok(saved.snapshot)
}

pub fn remove(profile: &str) -> Result<()> {
    fs::remove_file(recovery_path(profile)?).context("无法清除已恢复的快照")
}
