use anyhow::{bail, Context, Result};
use std::ffi::c_void;
use std::io;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use url::Url;
use windows_sys::Win32::Networking::WinHttp::*;

struct Handle(*mut c_void);

impl Handle {
    fn checked(value: *mut c_void) -> io::Result<Self> {
        if value.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(value))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}

fn checked(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

pub fn post_json(endpoint: &Url, local: bool, key: &str, body: &[u8]) -> Result<Vec<u8>> {
    let host = wide(endpoint.host_str().context("AI 地址缺少主机")?);
    let port = endpoint
        .port_or_known_default()
        .context("AI 地址缺少端口")?;
    let path = wide(endpoint.path());
    let headers = if key.is_empty() {
        "Content-Type: application/json\r\nAccept: application/json\r\n".to_owned()
    } else {
        format!("Content-Type: application/json\r\nAccept: application/json\r\nAuthorization: Bearer {key}\r\n")
    };
    let headers = wide(&headers);
    let length = u32::try_from(body.len())?;
    let proxy = if local {
        WINHTTP_ACCESS_TYPE_NO_PROXY
    } else {
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY
    };
    let tls = if endpoint.scheme() == "https" {
        WINHTTP_FLAG_SECURE
    } else {
        0
    };

    let response = unsafe {
        let session = Handle::checked(WinHttpOpen(
            wide(concat!("TabSort Lite/", env!("CARGO_PKG_VERSION"))).as_ptr(),
            proxy,
            null(),
            null(),
            0,
        ))?;
        checked(WinHttpSetTimeouts(
            session.0, 8_000, 10_000, 15_000, 110_000,
        ))?;
        let connection = Handle::checked(WinHttpConnect(session.0, host.as_ptr(), port, 0))?;
        let request = Handle::checked(WinHttpOpenRequest(
            connection.0,
            wide("POST").as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            tls,
        ))?;
        // Keep the request and credentials on the exact endpoint the user chose.
        let disabled =
            WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION;
        checked(WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_DISABLE_FEATURE,
            (&disabled as *const u32).cast(),
            4,
        ))?;
        let started = Instant::now();
        checked(WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            body.as_ptr().cast(),
            length,
            length,
            0,
        ))
        .context("无法连接 AI 服务")?;
        checked(WinHttpReceiveResponse(request.0, null_mut())).context("AI 服务未返回响应")?;

        let mut status = 0_u32;
        let mut size = 4_u32;
        checked(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            null_mut(),
        ))?;
        if status != 200 {
            match status {
                401 | 403 => bail!("AI 服务拒绝访问（HTTP {status}）。请检查 API 密钥和权限。"),
                404 => bail!("AI 接口或模型不存在（HTTP 404）。请检查地址和模型名称。"),
                429 => bail!("AI 服务限流或额度不足（HTTP 429）。"),
                _ => bail!("AI 服务返回 HTTP {status}。请检查接口地址及模型配置。"),
            }
        }

        let mut response = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let mut count = 0;
            checked(WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut count,
            ))
            .context("无法读取 AI 分析结果")?;
            if count == 0 {
                break;
            }
            if started.elapsed() > Duration::from_secs(150) {
                bail!("AI 分析超时");
            }
            if response.len() + count as usize > 1024 * 1024 {
                bail!("AI 返回的内容超出大小限制");
            }
            response.extend_from_slice(&buffer[..count as usize]);
        }
        response
    };
    Ok(response)
}
