//! Minimal blocking HTTP client built on WinHTTP.
//!
//! WinHTTP gives us the system proxy configuration, transparent redirects and
//! TLS without pulling a Rust TLS stack into the Windows-only build. Everything
//! runs on a dedicated worker thread; the caller passes a progress callback and
//! a cancellation flag.

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use octos_core::community::{HttpUrl, UrlScheme, parse_http_url};
use thiserror::Error;
use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows::Win32::Networking::WinHttp::*;
use windows::core::{PCWSTR, w};

#[derive(Debug, Error)]
pub enum NetError {
    #[error("download cancelled")]
    Cancelled,
    #[error("HTTP status {status} for {url}")]
    Status { status: u16, url: String },
    #[error("HTTP rate limited (status {status})")]
    RateLimited {
        status: u16,
        retry_after: Option<u64>,
    },
    #[error("WinHTTP error: {0}")]
    WinHttp(String),
    #[error("proxy error: {0}")]
    Proxy(String),
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("response is too large: {bytes} bytes (limit {limit})")]
    TooLarge { bytes: u64, limit: u64 },
    #[error("invalid URL: {0}")]
    Url(String),
}

#[derive(Debug, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub destination: Option<PathBuf>,
    pub bytes: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_length: Option<u64>,
    pub final_url: String,
}

/// Blocking GET. `destination` writes the body to a file (created/truncated);
/// otherwise the body is returned in memory. `progress` receives
/// `(received, total)` and is throttled to roughly 1% or 200 ms.
pub fn http_execute(
    url: &str,
    user_agent: &str,
    extra_headers: &[(String, String)],
    destination: Option<&Path>,
    max_bytes: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<HttpResponse, NetError> {
    let parsed = parse_http_url(url).map_err(|error| NetError::Url(error.to_string()))?;
    let proxy = proxy_setup(url, &parsed);

    let agent = wide(user_agent);
    let session = unsafe {
        WinHttpOpen(
            PCWSTR::from_raw(agent.as_ptr()),
            proxy.access,
            proxy.proxy_ptr(),
            proxy.bypass_ptr(),
            0,
        )
    };
    if session.is_null() {
        return Err(NetError::WinHttp("WinHttpOpen failed".into()));
    }
    let session = Handle(session);
    unsafe {
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 60_000).map_err(win_http)?;
        let policy = WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS.to_le_bytes();
        let _ = WinHttpSetOption(
            Some(session.0.cast_const()),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&policy),
        );
    }

    let host = wide(&parsed.host);
    let connect =
        unsafe { WinHttpConnect(session.0, PCWSTR::from_raw(host.as_ptr()), parsed.port, 0) };
    if connect.is_null() {
        return Err(NetError::WinHttp("WinHttpConnect failed".into()));
    }
    let connect = Handle(connect);

    let mut flag_bits = WINHTTP_FLAG_REFRESH.0;
    if parsed.scheme == UrlScheme::Https {
        flag_bits |= WINHTTP_FLAG_SECURE.0;
    }
    let path = wide(&parsed.path_and_query);
    let request = unsafe {
        WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            PCWSTR::from_raw(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_OPEN_REQUEST_FLAGS(flag_bits),
        )
    };
    if request.is_null() {
        return Err(NetError::WinHttp("WinHttpOpenRequest failed".into()));
    }
    let request = Handle(request);

    if !extra_headers.is_empty() {
        let mut block = String::new();
        for (name, value) in extra_headers {
            block.push_str(name);
            block.push_str(": ");
            block.push_str(value);
            block.push_str("\r\n");
        }
        let block: Vec<u16> = block.encode_utf16().collect();
        unsafe {
            WinHttpAddRequestHeaders(
                request.0,
                &block,
                WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
            )
            .map_err(win_http)?;
        }
    }

    unsafe {
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(win_http)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(win_http)?;
    }

    let status = unsafe { query_u32(request.0, WINHTTP_QUERY_STATUS_CODE) }.unwrap_or(0) as u16;
    let etag = unsafe { query_string(request.0, WINHTTP_QUERY_ETAG) };
    let last_modified = unsafe { query_string(request.0, WINHTTP_QUERY_LAST_MODIFIED) };
    let content_length = unsafe { query_string(request.0, WINHTTP_QUERY_CONTENT_LENGTH) }
        .and_then(|value| value.trim().parse::<u64>().ok());

    if status == 304 {
        return Ok(HttpResponse {
            status,
            etag,
            last_modified,
            content_length,
            final_url: url.to_string(),
            ..Default::default()
        });
    }
    if !(200..300).contains(&status) {
        if status == 403 || status == 429 {
            let retry_after = unsafe { query_string(request.0, WINHTTP_QUERY_RETRY_AFTER) }
                .and_then(|value| value.trim().parse::<u64>().ok());
            return Err(NetError::RateLimited {
                status,
                retry_after,
            });
        }
        return Err(NetError::Status {
            status,
            url: url.to_string(),
        });
    }

    let mut file = match destination {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Some(File::create(path)?)
        }
        None => None,
    };
    let mut body = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut last_percent = u64::MAX;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(NetError::Cancelled);
        }
        let mut available = 0u32;
        unsafe { WinHttpQueryDataAvailable(request.0, &mut available).map_err(win_http)? };
        if available == 0 {
            break;
        }
        let chunk = (available as usize).min(buffer.len());
        let mut read = 0u32;
        unsafe {
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                chunk as u32,
                &mut read,
            )
            .map_err(win_http)?;
        }
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes {
            return Err(NetError::TooLarge {
                bytes: total,
                limit: max_bytes,
            });
        }
        let slice = &buffer[..read as usize];
        match file.as_mut() {
            Some(file) => file.write_all(slice)?,
            None => body.extend_from_slice(slice),
        }
        let percent = content_length
            .filter(|length| *length > 0)
            .map(|length| total.saturating_mul(100) / length)
            .unwrap_or(0);
        if percent != last_percent || last_emit.elapsed() >= Duration::from_millis(200) {
            progress(total, content_length);
            last_percent = percent;
            last_emit = Instant::now();
        }
    }
    if let Some(file) = file.as_mut() {
        file.flush()?;
    }
    progress(total, content_length);

    Ok(HttpResponse {
        status,
        body,
        destination: destination.map(Path::to_path_buf),
        bytes: total,
        etag,
        last_modified,
        content_length,
        final_url: url.to_string(),
    })
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

struct ProxySetup {
    access: WINHTTP_ACCESS_TYPE,
    proxy: Vec<u16>,
    bypass: Vec<u16>,
}

impl ProxySetup {
    fn proxy_ptr(&self) -> PCWSTR {
        if self.proxy.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR::from_raw(self.proxy.as_ptr())
        }
    }

    fn bypass_ptr(&self) -> PCWSTR {
        if self.bypass.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR::from_raw(self.bypass.as_ptr())
        }
    }
}

fn proxy_setup(url: &str, parsed: &HttpUrl) -> ProxySetup {
    if parsed.is_loopback() {
        return ProxySetup {
            access: WINHTTP_ACCESS_TYPE_NO_PROXY,
            proxy: Vec::new(),
            bypass: Vec::new(),
        };
    }
    if let Ok(value) = std::env::var("OCTOS_COMMUNITY_PROXY") {
        let value = value.trim();
        if !value.is_empty() {
            return ProxySetup {
                access: WINHTTP_ACCESS_TYPE_NAMED_PROXY,
                proxy: wide(&normalize_proxy(value)),
                bypass: Vec::new(),
            };
        }
    }

    let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) }.is_ok() {
        let pac = unsafe { take_pwstr(config.lpszAutoConfigUrl) };
        let proxy = unsafe { take_pwstr(config.lpszProxy) };
        let bypass = unsafe { take_pwstr(config.lpszProxyBypass) };
        if !pac.is_empty() {
            tracing::debug!("system PAC configured at {pac}; using WinHTTP automatic proxy");
        }
        if let Some(selected) = proxy_from_ie(&proxy, parsed.scheme) {
            return ProxySetup {
                access: WINHTTP_ACCESS_TYPE_NAMED_PROXY,
                proxy: wide(&selected),
                bypass: if bypass.is_empty() {
                    Vec::new()
                } else {
                    wide(&bypass)
                },
            };
        }
    }
    tracing::debug!("no explicit proxy for {url}; using WinHTTP automatic proxy");
    ProxySetup {
        access: WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        proxy: Vec::new(),
        bypass: Vec::new(),
    }
}

fn proxy_from_ie(raw: &str, scheme: UrlScheme) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let scheme_name = match scheme {
        UrlScheme::Https => "https",
        UrlScheme::Http => "http",
    };
    let mut fallback = None;
    for entry in raw.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match entry.split_once('=') {
            Some((key, value)) => {
                if key.trim().eq_ignore_ascii_case(scheme_name) {
                    return Some(normalize_proxy(value));
                }
            }
            None => fallback = Some(normalize_proxy(entry)),
        }
    }
    fallback
}

fn normalize_proxy(value: &str) -> String {
    let value = value.trim();
    let value = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .unwrap_or(value);
    value.trim_end_matches('/').to_string()
}

/// Copy a WinHTTP-allocated wide string and release it.
unsafe fn take_pwstr(value: windows::core::PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    let text = unsafe { value.to_string() }.unwrap_or_default();
    unsafe {
        let _ = GlobalFree(Some(HGLOBAL(value.0.cast())));
    }
    text
}

unsafe fn query_u32(request: *mut c_void, header: u32) -> Option<u32> {
    let mut value = 0u32;
    let mut length = std::mem::size_of::<u32>() as u32;
    unsafe {
        WinHttpQueryHeaders(
            request,
            header | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut value as *mut u32).cast()),
            &mut length,
            std::ptr::null_mut(),
        )
        .ok()?;
    }
    Some(value)
}

unsafe fn query_string(request: *mut c_void, header: u32) -> Option<String> {
    let mut length = 0u32;
    unsafe {
        let _ = WinHttpQueryHeaders(
            request,
            header,
            PCWSTR::null(),
            None,
            &mut length,
            std::ptr::null_mut(),
        );
    }
    if length == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (length as usize).div_ceil(2) + 1];
    let mut length = (buffer.len() * 2) as u32;
    unsafe {
        WinHttpQueryHeaders(
            request,
            header,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr().cast()),
            &mut length,
            std::ptr::null_mut(),
        )
        .ok()?;
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end]).ok()
}

fn win_http(error: windows::core::Error) -> NetError {
    NetError::WinHttp(error.to_string())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn serve(responses: Vec<&'static str>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    fn get(url: &str) -> Result<HttpResponse, NetError> {
        let cancel = AtomicBool::new(false);
        http_execute(
            url,
            "Octocaris/test",
            &[],
            None,
            1024 * 1024,
            &cancel,
            &mut |_, _| {},
        )
    }

    #[test]
    fn fetches_a_body_and_headers() {
        let (base, server) = serve(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nETag: \"abc\"\r\nConnection: close\r\n\r\nhello",
        ]);
        let response = get(&format!("{base}/index.json")).unwrap();
        server.join().unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        assert_eq!(response.bytes, 5);
        assert_eq!(response.etag.as_deref(), Some("\"abc\""));
        assert_eq!(response.content_length, Some(5));
    }

    #[test]
    fn reports_status_codes_and_304() {
        let (base, server) = serve(vec![
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 304 Not Modified\r\nETag: \"x\"\r\nConnection: close\r\n\r\n",
        ]);
        let error = get(&format!("{base}/missing")).unwrap_err();
        assert!(matches!(error, NetError::Status { status: 404, .. }));
        let response = get(&format!("{base}/cached")).unwrap();
        server.join().unwrap();
        assert_eq!(response.status, 304);
        assert!(response.body.is_empty());
    }

    #[test]
    fn enforces_the_size_limit_and_cancellation() {
        let (base, server) = serve(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789",
        ]);
        let cancel = AtomicBool::new(false);
        let error = http_execute(
            &format!("{base}/big"),
            "Octocaris/test",
            &[],
            None,
            4,
            &cancel,
            &mut |_, _| {},
        )
        .unwrap_err();
        server.join().unwrap();
        assert!(matches!(error, NetError::TooLarge { .. }));

        let (base, server) = serve(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789",
        ]);
        let cancel = AtomicBool::new(true);
        let error = http_execute(
            &format!("{base}/cancelled"),
            "Octocaris/test",
            &[],
            None,
            1024,
            &cancel,
            &mut |_, _| {},
        )
        .unwrap_err();
        server.join().unwrap();
        assert!(matches!(error, NetError::Cancelled));
    }

    #[test]
    fn follows_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let target = format!("http://127.0.0.1:{port}/final");
        let handle = thread::spawn(move || {
            for index in 0..2 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let response = if index == 0 {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                        .to_string()
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let response = get(&format!("http://127.0.0.1:{port}/start")).unwrap();
        handle.join().unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"ok");
    }
}
