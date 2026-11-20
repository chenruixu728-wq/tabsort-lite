#![windows_subsystem = "windows"]

mod ai;
mod app;
mod bridge;
mod intent;
mod pipe;
mod planner;
mod protocol;
mod recovery;
mod semantic;
mod theme;
mod winhttp;

use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::{CreateMutexW, CREATE_NO_WINDOW};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR};

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if let Some(origin) = arguments
        .first()
        .filter(|arg| arg.starts_with("chrome-extension://"))
    {
        let _ = pipe::host(origin);
        return;
    }
    if arguments.first().is_some_and(|arg| arg == "--benchmark") {
        std::process::exit(if benchmark(&arguments).is_ok() { 0 } else { 1 });
    }
    if !single_instance() {
        return;
    }
    if let Err(error) = app::run() {
        let title: Vec<_> = "TabSort Lite".encode_utf16().chain([0]).collect();
        let text: Vec<_> = format!("无法打开 TabSort Lite。\n{error}")
            .encode_utf16()
            .chain([0])
            .collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_ICONERROR,
            );
        }
    }
}

fn single_instance() -> bool {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let name: Vec<_> = format!(r"Local\TabSortLite-{user}")
        .encode_utf16()
        .chain([0])
        .collect();
    // The handle is held until this desktop process exits.
    unsafe {
        !CreateMutexW(std::ptr::null(), 0, name.as_ptr()).is_null()
            && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

pub fn register_edge() -> Result<()> {
    let directory = recovery::data_directory()?;
    std::fs::create_dir_all(&directory)?;
    let manifest = directory.join("edge-native-host.json");
    let exe = std::env::current_exe()?;
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": protocol::HOST,
            "description": "TabSort Lite local Edge connector",
            "path": exe,
            "type": "stdio",
            "allowed_origins": [pipe::extension_origin()],
        }))?,
    )?;
    let key = format!(
        r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\{}",
        protocol::HOST
    );
    let output = Command::new("reg.exe")
        .args(["add", &key, "/ve", "/t", "REG_SZ", "/d"])
        .arg(manifest)
        .arg("/f")
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    anyhow::ensure!(output.status.success(), "无法在当前用户下注册 Edge 连接");
    Ok(())
}

fn benchmark(arguments: &[String]) -> Result<()> {
    let input = arguments.get(1).context("缺少测试快照")?;
    let output = arguments.get(2).context("缺少结果位置")?;
    let windows = arguments
        .get(3)
        .map_or(Ok(2_usize), |argument| argument.parse())?;
    anyhow::ensure!((1..=6).contains(&windows), "目标窗口数量必须为 1–6");
    let result = (|| -> Result<_> {
        let snapshot: protocol::Snapshot = serde_json::from_reader(std::fs::File::open(input)?)?;
        let started = std::time::Instant::now();
        let directory = std::env::var_os("TABSORT_BENCH_MODELS")
            .map_or_else(model_directory, |value| Ok(value.into()))?;
        let mut model = semantic::Model::load(&directory)?;
        let plan = planner::preview(&snapshot, &mut model, windows, planner::Detail::Standard)?;
        let cold_ms = started.elapsed().as_millis();
        let started = std::time::Instant::now();
        let _ = planner::preview(&snapshot, &mut model, windows, planner::Detail::Standard)?;
        Ok(
            serde_json::json!({"tabs": snapshot.tabs.len(), "coldMs": cold_ms, "warmMs": started.elapsed().as_millis(), "plan": plan}),
        )
    })();
    match result {
        Ok(report) => std::fs::write(output, serde_json::to_vec_pretty(&report)?)?,
        Err(error) => {
            std::fs::write(output, format!("{error:#}"))?;
            return Err(error);
        }
    }
    Ok(())
}

pub fn model_directory() -> Result<std::path::PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .context("找不到程序目录")?
        .join("models"))
}
