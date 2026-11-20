use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};

pub const HOST: &str = "com.tabsort_lite.connector";
pub const PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_COMMAND: usize = 1024 * 1024;

pub fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        bail!("消息长度无效");
    }
    let mut frame = vec![0; length];
    reader.read_exact(&mut frame)?;
    Ok(frame)
}

pub fn write_frame(writer: &mut impl Write, frame: &[u8]) -> Result<()> {
    if frame.is_empty() || frame.len() > MAX_FRAME {
        bail!("消息长度无效");
    }
    writer.write_all(&(frame.len() as u32).to_le_bytes())?;
    writer.write_all(frame)?;
    writer.flush()?;
    Ok(())
}

pub fn send<T: Serialize>(writer: &mut impl Write, message: &T) -> Result<()> {
    let frame = serde_json::to_vec(message)?;
    if frame.len() > MAX_COMMAND {
        bail!("单条浏览器指令超过 1 MB");
    }
    write_frame(writer, &frame)
}

pub fn receive<T: for<'a> Deserialize<'a>>(reader: &mut impl Read) -> Result<T> {
    serde_json::from_slice(&read_frame(reader)?).context("浏览器消息格式无效")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    pub profile_id: String,
    pub session_id: String,
    #[serde(default)]
    pub group_split: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub profile_id: String,
    pub session_id: String,
    pub revision: String,
    pub windows: Vec<Window>,
    pub tabs: Vec<Tab>,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub id: i32,
    pub focused: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: i32,
    pub window_id: i32,
    pub index: i32,
    pub title: String,
    pub url: String,
    pub pinned: bool,
    pub active: bool,
    pub group_id: i32,
    pub protection: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: i32,
    pub window_id: i32,
    pub title: String,
    pub color: String,
    pub collapsed: bool,
    pub shared: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub revision: String,
    pub windows: Vec<TargetWindow>,
    pub protected_count: usize,
    pub duplicate_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split_group_ids: Vec<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TargetWindow {
    #[serde(default)]
    pub title: String,
    pub groups: Vec<TargetGroup>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetGroup {
    pub title: String,
    pub color: String,
    pub tab_ids: Vec<i32>,
    pub existing_group_id: Option<i32>,
}
