use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ring::digest;
use serde_json::json;
use solaris_core_ir::{
    BrandTokens, CaptionBlock, CaptionPreset, Clip, ClipKind, Project, SavedCodeAsset, TrackKind,
};
use uuid::Uuid;

use crate::error::MediaError;
use crate::ffmpeg::MediaToolchain;

const CHROMIUM_CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);
const CHROMIUM_CDP_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_ANIMATED_FRAMES: usize = 180;
const MAX_WS_FRAME_BYTES: usize = 128 * 1024 * 1024;
const GSAP_MIN_JS: &str = include_str!("../vendor/gsap.min.js");
const SORA_WOFF2_BYTES: &[u8] = include_bytes!("../vendor/sora-latin-wght-normal.woff2");

#[derive(Debug, Clone, PartialEq)]
pub struct BrandFontData {
    pub family: String,
    pub mime: String,
    pub data_base64: String,
}

pub type BrandFontFace = BrandFontData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayRenderProgress {
    pub completed_frames: usize,
    pub total_frames: usize,
    pub stage_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayImageSpec {
    pub path: PathBuf,
    pub start_sec: f64,
    pub end_sec: f64,
    pub is_sequence: bool,
    pub fps: Option<f64>,
}

impl OverlayImageSpec {
    pub fn single(path: PathBuf, start_sec: f64, end_sec: f64) -> Self {
        Self {
            path,
            start_sec,
            end_sec,
            is_sequence: false,
            fps: None,
        }
    }

    pub fn sequence(pattern: PathBuf, start_sec: f64, end_sec: f64, fps: f64) -> Self {
        Self {
            path: pattern,
            start_sec,
            end_sec,
            is_sequence: true,
            fps: Some(fps),
        }
    }
}

/// RAII guard ensuring spawned Chromium processes are always killed & reaped on drop.
struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn as_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }

    fn mark_reaped(&mut self) {
        self.child = None;
    }

    fn kill_and_wait(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Minimal, self-contained RFC 6455 WebSocket client over `TcpStream` for local CDP.
struct CdpWebSocket {
    stream: TcpStream,
    read_buf: Vec<u8>,
}

impl CdpWebSocket {
    fn connect(port: u16, ws_path: &str) -> Result<Self, MediaError> {
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .map_err(|e| MediaError::ProcessFailed(format!("CDP TCP connect failed: {e}")))?;
        let _ = stream.set_read_timeout(Some(CHROMIUM_CAPTURE_TIMEOUT));
        let _ = stream.set_write_timeout(Some(CHROMIUM_CAPTURE_TIMEOUT));
        let _ = stream.set_nodelay(true);

        let ws_key = BASE64_STANDARD.encode(Uuid::new_v4().into_bytes());
        let mut accept_in = String::with_capacity(ws_key.len() + 36);
        accept_in.push_str(&ws_key);
        accept_in.push_str("258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        let expected_accept = BASE64_STANDARD.encode(
            digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, accept_in.as_bytes()).as_ref(),
        );

        let req = format!(
            "GET {ws_path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {ws_key}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).map_err(|e| {
            MediaError::ProcessFailed(format!("CDP WS handshake write failed: {e}"))
        })?;
        let _ = stream.flush();

        let mut buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let n = stream.read(&mut chunk).map_err(|e| {
                MediaError::ProcessFailed(format!("CDP WS handshake read failed: {e}"))
            })?;
            if n == 0 {
                return Err(MediaError::ProcessFailed(
                    "CDP WS closed during handshake".to_string(),
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            if buf.len() > 64 * 1024 {
                return Err(MediaError::ProcessFailed(
                    "CDP WS handshake response headers too large".to_string(),
                ));
            }
        };

        let header_str = String::from_utf8_lossy(&buf[..header_end]);
        let mut lines = header_str.lines();
        let status_line = lines.next().unwrap_or("");
        if !status_line.contains("101") {
            return Err(MediaError::ProcessFailed(format!(
                "CDP WS upgrade rejected: {status_line}"
            )));
        }
        let accept_ok = lines.any(|l| {
            l.split_once(':').is_some_and(|(k, v)| {
                k.eq_ignore_ascii_case("sec-websocket-accept") && v.trim() == expected_accept
            })
        });
        if !accept_ok {
            return Err(MediaError::ProcessFailed(
                "CDP WS Sec-WebSocket-Accept mismatch".to_string(),
            ));
        }

        let trailing = buf[header_end..].to_vec();
        Ok(Self {
            stream,
            read_buf: trailing,
        })
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), MediaError> {
        let mut frame = Vec::with_capacity(14 + payload.len());
        frame.push(0x80 | (opcode & 0x0f));
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | (len as u8));
        } else if len <= 65_535 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask_uuid = Uuid::new_v4();
        let mask = &mask_uuid.as_bytes()[..4];
        frame.extend_from_slice(mask);
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i & 3]);
        }
        self.stream
            .write_all(&frame)
            .map_err(|e| MediaError::ProcessFailed(format!("CDP WS send failed: {e}")))?;
        let _ = self.stream.flush();
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<(), MediaError> {
        self.send_frame(0x1, text.as_bytes())
    }

    fn read_exact_bytes(&mut self, count: usize) -> Result<Vec<u8>, MediaError> {
        while self.read_buf.len() < count {
            let mut tmp = [0u8; 8192];
            let n = self
                .stream
                .read(&mut tmp)
                .map_err(|e| MediaError::ProcessFailed(format!("CDP WS read failed: {e}")))?;
            if n == 0 {
                return Err(MediaError::ProcessFailed(
                    "CDP WS connection closed unexpectedly".to_string(),
                ));
            }
            self.read_buf.extend_from_slice(&tmp[..n]);
        }
        let out: Vec<u8> = self.read_buf.drain(..count).collect();
        Ok(out)
    }

    fn read_text_message(&mut self) -> Result<String, MediaError> {
        let mut assembled = Vec::new();
        loop {
            let hdr = self.read_exact_bytes(2)?;
            let fin = (hdr[0] & 0x80) != 0;
            let opcode = hdr[0] & 0x0f;
            let masked = (hdr[1] & 0x80) != 0;
            let len7 = (hdr[1] & 0x7f) as u64;

            let payload_len = if len7 == 126 {
                let ext = self.read_exact_bytes(2)?;
                u16::from_be_bytes([ext[0], ext[1]]) as usize
            } else if len7 == 127 {
                let ext = self.read_exact_bytes(8)?;
                let len64 = u64::from_be_bytes([
                    ext[0], ext[1], ext[2], ext[3], ext[4], ext[5], ext[6], ext[7],
                ]);
                if len64 > MAX_WS_FRAME_BYTES as u64 {
                    return Err(MediaError::ProcessFailed(format!(
                        "CDP WS frame exceeds maximum allowed size ({len64} bytes)"
                    )));
                }
                len64 as usize
            } else {
                len7 as usize
            };

            let mask_key = if masked {
                let m = self.read_exact_bytes(4)?;
                Some([m[0], m[1], m[2], m[3]])
            } else {
                None
            };

            let mut payload = if payload_len > 0 {
                self.read_exact_bytes(payload_len)?
            } else {
                Vec::new()
            };
            if let Some(m) = mask_key {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= m[i & 3];
                }
            }

            match opcode {
                0x9 => {
                    // Ping -> reply with Pong
                    self.send_frame(0xA, &payload)?;
                    continue;
                }
                0xA => continue, // Pong
                0x8 => {
                    return Err(MediaError::ProcessFailed(
                        "CDP WS peer sent close frame".to_string(),
                    ));
                }
                0x1 | 0x0 => {
                    assembled.extend_from_slice(&payload);
                    if fin {
                        return String::from_utf8(assembled).map_err(|e| {
                            MediaError::ParseError(format!("invalid UTF-8 in CDP message: {e}"))
                        });
                    }
                }
                _ => {
                    if fin && !assembled.is_empty() {
                        return String::from_utf8(assembled).map_err(|e| {
                            MediaError::ParseError(format!("invalid UTF-8 in CDP message: {e}"))
                        });
                    }
                }
            }
        }
    }
}

/// Persistent headless Chromium CDP session reused across all clips/frames in a job.
/// Drop order: close WebSocket first, kill & reap Chromium second, remove profile dir last.
struct ChromiumCdpSession {
    ws: CdpWebSocket,
    _child: ChildGuard,
    _profile_dir: TempDirGuard,
    next_id: u64,
    viewport: (u32, u32, f64),
}

impl ChromiumCdpSession {
    fn launch(chrome: &Path, width: u32, height: u32, temp_dir: &Path) -> Result<Self, MediaError> {
        let profile_path = temp_dir.join(format!("chrome-profile-{}", Uuid::new_v4()));
        fs::create_dir_all(&profile_path)?;
        let profile_guard = TempDirGuard(profile_path.clone());

        let user_data_arg = format!("--user-data-dir={}", profile_path.to_string_lossy());
        let size_arg = format!("--window-size={width},{height}");

        let mut cmd = Command::new(chrome);
        cmd.args([
            "--headless=new",
            &user_data_arg,
            "--remote-debugging-port=0",
            "--disable-gpu",
            "--hide-scrollbars",
            "--disable-dev-shm-usage",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-extensions",
            "--disable-sync",
            "--metrics-recording-only",
            "--mute-audio",
            "--no-first-run",
            "--no-default-browser-check",
            "--no-pings",
            "--proxy-server=http://127.0.0.1:1",
            "--proxy-bypass-list=<-loopback>",
            "--host-resolver-rules=MAP * ~NOTFOUND",
            &size_arg,
        ]);
        if should_use_chrome_no_sandbox() {
            cmd.arg("--no-sandbox");
        }
        cmd.arg("about:blank");
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            MediaError::ProcessFailed(format!(
                "failed to spawn headless Chromium '{}': {e}",
                chrome.display()
            ))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            MediaError::ProcessFailed("headless Chromium stderr pipe unavailable".to_string())
        })?;
        let child_guard = ChildGuard::new(child);

        // Drain stderr continuously on a background thread while extracting the DevTools WS URL
        let (tx, rx) = mpsc::channel::<Result<u16, String>>();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            let mut sent = false;
            let mut tail = String::new();
            for line in reader.lines().map_while(Result::ok) {
                if !sent && let Some(port) = parse_devtools_port(&line) {
                    let _ = tx.send(Ok(port));
                    sent = true;
                }
                if tail.len() < 4096 {
                    tail.push_str(&line);
                    tail.push('\n');
                }
            }
            if !sent {
                let _ = tx.send(Err(tail));
            }
        });

        let port = match rx.recv_timeout(CHROMIUM_CDP_STARTUP_TIMEOUT) {
            Ok(Ok(p)) => p,
            Ok(Err(stderr_log)) => {
                return Err(MediaError::ProcessFailed(format!(
                    "headless Chromium exited before DevTools port opened: {}",
                    stderr_log.trim()
                )));
            }
            Err(_) => {
                return Err(MediaError::ProcessFailed(
                    "timed out waiting for headless Chromium DevTools port".to_string(),
                ));
            }
        };

        let page_ws_path = discover_page_ws_path(port)?;
        let ws = CdpWebSocket::connect(port, &page_ws_path)?;

        let mut session = Self {
            _child: child_guard,
            _profile_dir: profile_guard,
            ws,
            next_id: 1,
            viewport: (width, height, 1.0),
        };

        // Configure offline network isolation, transparent RGBA background, and exact viewport
        session.send_command("Page.enable", json!({}))?;
        session.send_command("Network.enable", json!({}))?;
        session.send_command(
            "Network.setBlockedURLs",
            json!({
                "urls": ["http://*/*", "https://*/*", "ws://*/*", "wss://*/*"]
            }),
        )?;
        session.send_command(
            "Emulation.setDefaultBackgroundColorOverride",
            json!({
                "color": { "r": 0, "g": 0, "b": 0, "a": 0 }
            }),
        )?;
        session.send_command(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": 1,
                "mobile": false
            }),
        )?;

        Ok(session)
    }

    fn set_viewport(&mut self, css_w: u32, css_h: u32, dpr: f64) -> Result<(), MediaError> {
        if self.viewport.0 == css_w
            && self.viewport.1 == css_h
            && (self.viewport.2 - dpr).abs() < 1e-4
        {
            return Ok(());
        }
        self.send_command(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": css_w,
                "height": css_h,
                "deviceScaleFactor": dpr,
                "mobile": false
            }),
        )?;
        self.viewport = (css_w, css_h, dpr);
        Ok(())
    }

    fn send_command(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, MediaError> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string();
        self.ws.send_text(&payload)?;

        let deadline = Instant::now() + CHROMIUM_CAPTURE_TIMEOUT;
        while Instant::now() < deadline {
            let msg = self.ws.read_text_message()?;
            let val: serde_json::Value = serde_json::from_str(&msg)
                .map_err(|e| MediaError::ParseError(format!("invalid CDP JSON: {e}")))?;
            if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = val.get("error") {
                    return Err(MediaError::ProcessFailed(format!(
                        "CDP {method} failed: {err}"
                    )));
                }
                return Ok(val
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null));
            }
        }
        Err(MediaError::ProcessFailed(format!(
            "timed out waiting for CDP response to {method}"
        )))
    }

    fn navigate_and_wait(&mut self, html_path: &Path) -> Result<(), MediaError> {
        let file_url = path_to_file_uri(html_path).map_err(|e| {
            MediaError::StorageError(format!(
                "failed to canonicalize overlay HTML '{}': {e}",
                html_path.display()
            ))
        })?;

        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "id": id,
            "method": "Page.navigate",
            "params": { "url": file_url },
        })
        .to_string();
        self.ws.send_text(&payload)?;

        let mut got_reply = false;
        let mut got_load = false;
        let deadline = Instant::now() + CHROMIUM_CAPTURE_TIMEOUT;
        while !(got_reply && got_load) && Instant::now() < deadline {
            let msg = self.ws.read_text_message()?;
            let val: serde_json::Value = serde_json::from_str(&msg)
                .map_err(|e| MediaError::ParseError(format!("invalid CDP JSON: {e}")))?;
            if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = val.get("error") {
                    return Err(MediaError::ProcessFailed(format!(
                        "CDP Page.navigate failed: {err}"
                    )));
                }
                if let Some(err_text) = val
                    .get("result")
                    .and_then(|r| r.get("errorText"))
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                {
                    return Err(MediaError::ProcessFailed(format!(
                        "CDP Page.navigate errorText: {err_text}"
                    )));
                }
                got_reply = true;
            } else if val.get("method").and_then(|m| m.as_str()) == Some("Page.loadEventFired") {
                got_load = true;
            }
        }
        if !(got_reply && got_load) {
            return Err(MediaError::ProcessFailed(format!(
                "timed out waiting for Page.loadEventFired on '{}'",
                html_path.display()
            )));
        }

        // Ensure bundled Sora & brand @font-face fonts have finished decoding before capture
        let _ = self.evaluate_js(
            "document.fonts ? document.fonts.ready.then(function(){ return true; }) : true",
        );
        Ok(())
    }

    fn evaluate_js(&mut self, expression: &str) -> Result<serde_json::Value, MediaError> {
        let res = self.send_command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )?;
        if let Some(exc) = res.get("exceptionDetails") {
            return Err(MediaError::ProcessFailed(format!(
                "CDP Runtime.evaluate exception: {exc}"
            )));
        }
        Ok(res)
    }

    fn capture_png(&mut self, png_path: &Path) -> Result<(), MediaError> {
        let res = self.send_command(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "fromSurface": true,
                "optimizeForSpeed": true,
            }),
        )?;
        let b64 = res
            .get("data")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                MediaError::ProcessFailed("CDP Page.captureScreenshot returned empty data".into())
            })?;
        let png_bytes = BASE64_STANDARD
            .decode(b64)
            .map_err(|e| MediaError::ParseError(format!("failed to decode CDP PNG base64: {e}")))?;
        if png_bytes.is_empty() {
            return Err(MediaError::ProcessFailed(
                "CDP Page.captureScreenshot decoded to 0 bytes".to_string(),
            ));
        }
        fs::write(png_path, &png_bytes)?;
        Ok(())
    }
}

fn parse_devtools_port(line: &str) -> Option<u16> {
    let idx = line.find("DevTools listening on ws://")?;
    let rest = &line[idx + "DevTools listening on ws://".len()..];
    let authority = rest.split('/').next()?;
    let (_host, port_str) = authority.rsplit_once(':')?;
    port_str.trim().parse::<u16>().ok()
}

const MAX_CDP_HTTP_BODY_BYTES: usize = 8 * 1024 * 1024;

fn decode_chunked_body(raw: &[u8]) -> Result<Vec<u8>, MediaError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let Some(rel_crlf) = raw[pos..].windows(2).position(|w| w == b"\r\n") else {
            return Err(MediaError::ParseError(
                "truncated chunked CDP HTTP response".to_string(),
            ));
        };
        let size_line = std::str::from_utf8(&raw[pos..pos + rel_crlf])
            .map_err(|e| MediaError::ParseError(format!("invalid chunk size line: {e}")))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let chunk_len = usize::from_str_radix(size_hex, 16)
            .map_err(|e| MediaError::ParseError(format!("invalid chunk size '{size_hex}': {e}")))?;
        pos += rel_crlf + 2;
        if chunk_len == 0 {
            return Ok(out);
        }
        if pos + chunk_len + 2 > raw.len() {
            return Err(MediaError::ParseError(
                "incomplete chunk payload in CDP HTTP response".to_string(),
            ));
        }
        if out.len() + chunk_len > MAX_CDP_HTTP_BODY_BYTES {
            return Err(MediaError::ParseError(
                "chunked CDP HTTP response exceeded size limit".to_string(),
            ));
        }
        out.extend_from_slice(&raw[pos..pos + chunk_len]);
        pos += chunk_len + 2;
    }
}

/// Reads a single HTTP/1.x response respecting `Content-Length` and `Transfer-Encoding: chunked`
/// without waiting for TCP EOF (Chromium's DevTools HTTP server ignores `Connection: close` and
/// keeps the socket open, so `read_to_end` always stalls until read timeout).
fn read_http_response_body(stream: &mut impl Read) -> Result<(u16, Vec<u8>), MediaError> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = stream
            .read(&mut tmp)
            .map_err(|e| MediaError::ProcessFailed(format!("CDP HTTP read failed: {e}")))?;
        if n == 0 {
            return Err(MediaError::ParseError(
                "malformed CDP HTTP response (EOF before headers)".to_string(),
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 64 * 1024 {
            return Err(MediaError::ParseError(
                "CDP HTTP response headers exceeded 64 KiB".to_string(),
            ));
        }
    };

    let header_str = std::str::from_utf8(&buf[..header_end - 4])
        .map_err(|e| MediaError::ParseError(format!("non-UTF8 CDP HTTP headers: {e}")))?;
    let mut lines = header_str.lines();
    let status_line = lines.next().unwrap_or("");
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| {
            MediaError::ParseError(format!("invalid CDP HTTP status line: {status_line}"))
        })?;

    let mut content_length: Option<usize> = None;
    let mut is_chunked = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().ok();
            } else if k.trim().eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
            {
                is_chunked = true;
            }
        }
    }

    let mut body = buf[header_end..].to_vec();
    if let Some(expected_len) = content_length {
        if expected_len > MAX_CDP_HTTP_BODY_BYTES {
            return Err(MediaError::ParseError(format!(
                "CDP HTTP body exceeds max size ({expected_len} bytes)"
            )));
        }
        while body.len() < expected_len {
            let need = (expected_len - body.len()).min(tmp.len());
            let n = stream.read(&mut tmp[..need]).map_err(|e| {
                MediaError::ProcessFailed(format!("CDP HTTP body read failed: {e}"))
            })?;
            if n == 0 {
                return Err(MediaError::ParseError(format!(
                    "truncated CDP HTTP body (got {} of {expected_len} bytes)",
                    body.len()
                )));
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(expected_len);
        return Ok((status_code, body));
    }

    if is_chunked {
        loop {
            if let Ok(decoded) = decode_chunked_body(&body) {
                return Ok((status_code, decoded));
            }
            let n = stream.read(&mut tmp).map_err(|e| {
                MediaError::ProcessFailed(format!("CDP HTTP chunk read failed: {e}"))
            })?;
            if n == 0 {
                return Ok((status_code, decode_chunked_body(&body)?));
            }
            body.extend_from_slice(&tmp[..n]);
            if body.len() > MAX_CDP_HTTP_BODY_BYTES {
                return Err(MediaError::ParseError(
                    "chunked CDP HTTP response exceeded size limit".to_string(),
                ));
            }
        }
    }

    stream
        .read_to_end(&mut body)
        .map_err(|e| MediaError::ProcessFailed(format!("CDP HTTP read failed: {e}")))?;
    Ok((status_code, body))
}

fn cdp_http_json(port: u16, method: &str, path: &str) -> Result<serde_json::Value, MediaError> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| MediaError::ProcessFailed(format!("CDP HTTP connect failed: {e}")))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let req =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| MediaError::ProcessFailed(format!("CDP HTTP write failed: {e}")))?;
    let _ = stream.flush();
    let (status_code, body_bytes) = read_http_response_body(&mut stream)?;
    if !(200..300).contains(&status_code) {
        return Err(MediaError::ProcessFailed(format!(
            "CDP HTTP {method} {path} returned status {status_code}"
        )));
    }
    let body = std::str::from_utf8(&body_bytes)
        .map_err(|e| MediaError::ParseError(format!("non-UTF8 CDP HTTP body: {e}")))?;
    serde_json::from_str(body)
        .map_err(|e| MediaError::ParseError(format!("invalid CDP JSON from {path}: {e}")))
}

fn extract_ws_path(ws_url: &str) -> Option<String> {
    let rest = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))?;
    let slash_idx = rest.find('/')?;
    Some(rest[slash_idx..].to_string())
}

fn discover_page_ws_path(port: u16) -> Result<String, MediaError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(list) = cdp_http_json(port, "GET", "/json/list")
            && let Some(arr) = list.as_array()
        {
            for item in arr {
                if item.get("type").and_then(|t| t.as_str()) == Some("page")
                    && let Some(ws_url) = item.get("webSocketDebuggerUrl").and_then(|u| u.as_str())
                    && let Some(path) = extract_ws_path(ws_url)
                {
                    return Ok(path);
                }
            }
        }
        if let Ok(created) = cdp_http_json(port, "PUT", "/json/new?about:blank")
            && let Some(ws_url) = created.get("webSocketDebuggerUrl").and_then(|u| u.as_str())
            && let Some(path) = extract_ws_path(ws_url)
        {
            return Ok(path);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(MediaError::ProcessFailed(
        "could not discover CDP page target from /json/list".to_string(),
    ))
}

fn bundled_sora_font_css() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| {
        let b64 = BASE64_STANDARD.encode(SORA_WOFF2_BYTES);
        format!(
            "@font-face {{\n  font-family: \"Sora\";\n  font-style: normal;\n  font-weight: 100 800;\n  font-display: block;\n  src: url(\"data:font/woff2;base64,{b64}\") format(\"woff2\");\n}}"
        )
    })
}

fn safe_gsap_script() -> &'static str {
    static SAFE: OnceLock<String> = OnceLock::new();
    SAFE.get_or_init(|| GSAP_MIN_JS.replace("</script", "<\\/script"))
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn sanitize_css_val(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .filter(|c| {
            !matches!(
                c,
                ';' | '{' | '}' | '<' | '>' | '\\' | '\n' | '\r' | '"' | '\''
            )
        })
        .collect();
    loop {
        let next = out.replace("/*", "").replace("*/", "");
        if next == out {
            break;
        }
        out = next;
    }
    out.trim().to_string()
}

/// Computes the CSS design viewport and device pixel ratio for `ClipKind::Gfx` overlays
/// so Chromium renders in the exact same logical coordinate space as `WebGLProgramStage.tsx`
/// while producing crisp full-resolution RGBA PNGs at `canvas_w x canvas_h`.
pub fn gfx_design_viewport(canvas_w: u32, canvas_h: u32) -> (u32, u32, f64) {
    let aspect = canvas_w as f64 / canvas_h.max(1) as f64;
    let design_w = if aspect < 0.85 {
        540
    } else if aspect > 1.2 {
        960
    } else {
        720
    };
    if canvas_w <= design_w {
        return (canvas_w, canvas_h, 1.0);
    }
    let dpr = canvas_w as f64 / design_w as f64;
    let design_h = ((canvas_h as f64 / dpr).round() as u32).max(2);
    (design_w, design_h, dpr)
}

fn is_valid_font_mime(mime: &str) -> bool {
    !mime.is_empty()
        && mime.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'/' | b'+' | b'.' | b'-')
        })
}

fn is_valid_base64_chars(b64: &str) -> bool {
    !b64.trim().is_empty()
        && b64.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=') || b.is_ascii_whitespace()
        })
}

fn build_brand_fonts_css(fonts: &[BrandFontData]) -> String {
    let mut css = String::new();
    for f in fonts {
        let fam = sanitize_css_val(&f.family).replace('"', "");
        let mime = f.mime.trim();
        if fam.is_empty() || !is_valid_font_mime(mime) || !is_valid_base64_chars(&f.data_base64) {
            continue;
        }
        let clean_b64: String = f
            .data_base64
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        if clean_b64.is_empty() {
            continue;
        }
        let fmt = if mime.contains("woff2") {
            "woff2"
        } else if mime.contains("woff") {
            "woff"
        } else if mime.contains("otf") || mime.contains("opentype") {
            "opentype"
        } else {
            "truetype"
        };
        css.push_str(&format!(
            "@font-face {{\n  font-family: \"{fam}\";\n  font-display: block;\n  src: url(\"data:{mime};base64,{clean_b64}\") format(\"{fmt}\");\n}}\n"
        ));
    }
    css
}

/// Converts a local filesystem path into a canonical, percent-encoded `file:///` URI.
pub fn path_to_file_uri(path: &Path) -> Result<String, std::io::Error> {
    let canon = fs::canonicalize(path)?;
    let raw = canon.to_string_lossy();
    let normalized = raw.replace('\\', "/");
    let trimmed = normalized.trim_start_matches('/');
    let mut encoded = String::with_capacity(trimmed.len() + 16);
    encoded.push_str("file:///");
    for (idx, seg) in trimmed.split('/').enumerate() {
        if idx > 0 {
            encoded.push('/');
        }
        // Preserve Windows drive colon (e.g. "C:") in first segment
        if idx == 0
            && seg.len() == 2
            && seg.as_bytes()[0].is_ascii_alphabetic()
            && seg.as_bytes()[1] == b':'
        {
            encoded.push_str(seg);
            continue;
        }
        for &b in seg.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                encoded.push(b as char);
            } else {
                encoded.push_str(&format!("%{b:02X}"));
            }
        }
    }
    Ok(encoded)
}

fn should_use_chrome_no_sandbox() -> bool {
    if let Ok(val) = std::env::var("SOLARIS_CHROME_NO_SANDBOX") {
        let lower = val.trim().to_ascii_lowercase();
        if !lower.is_empty() {
            return matches!(lower.as_str(), "1" | "true" | "yes");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists() {
            return true;
        }
    }
    false
}

/// Single-shot CLI screenshot fallback (isolated `--user-data-dir`, proxy block, and RAII `ChildGuard`).
fn run_chromium_screenshot(
    chrome: &Path,
    html_path: &Path,
    png_path: &Path,
    width: u32,
    height: u32,
    dpr: f64,
    temp_dir: &Path,
) -> Result<(), MediaError> {
    let file_url = path_to_file_uri(html_path).map_err(|e| {
        MediaError::StorageError(format!(
            "failed to canonicalize overlay HTML '{}': {e}",
            html_path.display()
        ))
    })?;
    let profile_path = temp_dir.join(format!("chrome-cli-profile-{}", Uuid::new_v4()));
    fs::create_dir_all(&profile_path)?;
    let _profile_guard = TempDirGuard(profile_path.clone());

    let user_data_arg = format!("--user-data-dir={}", profile_path.to_string_lossy());
    let screenshot_arg = format!("--screenshot={}", png_path.to_string_lossy());
    let size_arg = format!("--window-size={width},{height}");
    let dpr_arg = ((dpr - 1.0).abs() > 0.001).then(|| format!("--force-device-scale-factor={dpr}"));

    let mut cmd = Command::new(chrome);
    cmd.args([
        "--headless=new",
        &user_data_arg,
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--hide-scrollbars",
        "--no-first-run",
        "--no-default-browser-check",
        "--default-background-color=00000000",
        "--proxy-server=http://127.0.0.1:1",
        "--proxy-bypass-list=<-loopback>",
        "--host-resolver-rules=MAP * ~NOTFOUND",
        &size_arg,
        &screenshot_arg,
    ]);
    if let Some(arg) = dpr_arg.as_deref() {
        cmd.arg(arg);
    }
    if should_use_chrome_no_sandbox() {
        cmd.arg("--no-sandbox");
    }
    cmd.arg(&file_url);
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        MediaError::ProcessFailed(format!(
            "failed to spawn headless Chromium '{}': {e}",
            chrome.display()
        ))
    })?;

    let stderr_handle = child.stderr.take().map(|mut err_pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err_pipe.read_to_string(&mut buf);
            buf
        })
    });
    let mut guard = ChildGuard::new(child);

    let start = Instant::now();
    let status = loop {
        let Some(c) = guard.as_mut() else {
            break None;
        };
        match c.try_wait() {
            Ok(Some(st)) => {
                guard.mark_reaped();
                break Some(st);
            }
            Ok(None) => {
                if start.elapsed() >= CHROMIUM_CAPTURE_TIMEOUT {
                    guard.kill_and_wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                guard.kill_and_wait();
                return Err(MediaError::ProcessFailed(format!(
                    "error waiting for headless Chromium: {err}"
                )));
            }
        }
    };

    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    let Some(exit_status) = status else {
        return Err(MediaError::ProcessFailed(format!(
            "headless Chromium timed out after {}s rendering '{}': {}",
            CHROMIUM_CAPTURE_TIMEOUT.as_secs(),
            html_path.display(),
            stderr_text.trim()
        )));
    };

    if !exit_status.success() {
        return Err(MediaError::ProcessFailed(format!(
            "headless Chromium exited {:?} rendering '{}': {}",
            exit_status.code(),
            html_path.display(),
            stderr_text.trim()
        )));
    }

    let ok = png_path
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() > 0);
    if !ok {
        return Err(MediaError::ProcessFailed(format!(
            "headless Chromium exited 0 but did not produce a non-empty PNG at '{}': {}",
            png_path.display(),
            stderr_text.trim()
        )));
    }
    Ok(())
}

fn merge_json_props(
    defaults: &serde_json::Value,
    overrides: &serde_json::Value,
) -> serde_json::Value {
    match (defaults, overrides) {
        (serde_json::Value::Object(def_map), serde_json::Value::Object(ov_map)) => {
            let mut merged = def_map.clone();
            for (k, v) in ov_map {
                let next = match merged.get(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        merge_json_props(existing, v)
                    }
                    _ => v.clone(),
                };
                merged.insert(k.clone(), next);
            }
            serde_json::Value::Object(merged)
        }
        (_, serde_json::Value::Null) => defaults.clone(),
        (_, ov) => ov.clone(),
    }
}

fn resolve_prop_value<'a>(
    props: &'a serde_json::Value,
    key: &str,
) -> Option<&'a serde_json::Value> {
    if let Some(v) = props.get(key) {
        return Some(v);
    }
    if !key.contains('.') {
        return None;
    }
    let mut cur = props;
    for part in key.split('.') {
        cur = cur.get(part)?;
    }
    Some(cur)
}

fn is_valid_prop_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn interpolate_props(html_source: &str, props: &serde_json::Value) -> String {
    let mut out = String::with_capacity(html_source.len());
    let mut rest = html_source;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("}}") {
            let key = after[..end].trim();
            if is_valid_prop_key(key) {
                let val = resolve_prop_value(props, key).and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                });
                if let Some(s) = val {
                    out.push_str(&escape_html(&s));
                }
                rest = &after[end + 2..];
            } else {
                // Preserve JS object literals like `{{a: 1}}` untouched
                out.push_str("{{");
                rest = after;
            }
        } else {
            out.push_str("{{");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn has_dynamic_gfx_animation(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("<script")
        || lower.contains("@keyframes")
        || lower.contains("animation:")
        || lower.contains("animation-name")
        || lower.contains("gsap.")
        || lower.contains("__solarisonseek")
        || lower.contains("__timelines")
}

pub fn build_overlay_html(
    body_html: &str,
    brand_tokens: &BrandTokens,
    brand_fonts: &[BrandFontData],
    duration_sec: f64,
    seek_sec: f64,
) -> String {
    let safe_duration = if duration_sec.is_finite() && duration_sec >= 0.0 {
        duration_sec
    } else {
        0.0
    };
    let safe_seek = if seek_sec.is_finite() {
        seek_sec.clamp(0.0, safe_duration)
    } else {
        0.0
    };
    let bg = sanitize_css_val(&brand_tokens.bg);
    let surface = sanitize_css_val(&brand_tokens.surface);
    let ink = sanitize_css_val(&brand_tokens.ink);
    let accent = sanitize_css_val(&brand_tokens.accent);
    let accent2 = sanitize_css_val(&brand_tokens.accent_secondary);
    let font_display = sanitize_css_val(&brand_tokens.font_display).replace('"', "");
    let font_body = sanitize_css_val(&brand_tokens.font_body).replace('"', "");
    let sora_css = bundled_sora_font_css();
    let custom_fonts_css = build_brand_fonts_css(brand_fonts);
    let gsap_js = safe_gsap_script();

    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8" />
<meta name="color-scheme" content="dark" />
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src data:;" />
<style>
{sora_css}
{custom_fonts_css}
:root {{
  color-scheme: dark;
  --brand-bg: {bg};
  --brand-surface: {surface};
  --brand-ink: {ink};
  --brand-accent: {accent};
  --brand-accent-secondary: {accent2};
  --brand-font-display: "{font_display}", "Sora", sans-serif;
  --brand-font-body: "{font_body}", "Sora", sans-serif;
}}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
html, body {{
  color-scheme: dark;
  width: 100%;
  height: 100%;
  background: transparent !important;
  overflow: hidden;
  display: grid;
  place-items: center;
  font-family: var(--brand-font-body);
  color: var(--brand-ink);
}}
</style>
<script>{gsap_js}</script>
</head>
<body>
{body_html}
<script>
(function() {{
  const DURATION = {safe_duration:.3};
  const INITIAL_T = {safe_seek:.3};
  window.__solarisSeek = function(sec) {{
    const raw = typeof sec === "number" && isFinite(sec) ? sec : INITIAL_T;
    const t = Math.max(0, Math.min(DURATION, raw));
    if (typeof document.getAnimations === "function") {{
      for (const anim of document.getAnimations()) {{
        try {{ anim.pause(); anim.currentTime = t * 1000; }} catch (_) {{}}
      }}
    }}
    if (window.gsap && window.gsap.globalTimeline && typeof window.gsap.globalTimeline.pause === "function") {{
      try {{ window.gsap.globalTimeline.pause(t, false); }} catch (_) {{}}
    }}
    if (window.__timelines && typeof window.__timelines === "object") {{
      for (const tl of Object.values(window.__timelines)) {{
        if (tl && typeof tl.pause === "function" && typeof tl.seek === "function") {{
          tl.pause();
          tl.seek(t, false);
        }}
      }}
    }}
    if (typeof window.__solarisOnSeek === "function") {{
      window.__solarisOnSeek(t, DURATION);
    }}
  }};
  window.__solarisSeek(INITIAL_T);
  document.addEventListener("DOMContentLoaded", function() {{ window.__solarisSeek(INITIAL_T); }});
  window.addEventListener("load", function() {{ window.__solarisSeek(INITIAL_T); }});
}})();
</script>
</body>
</html>"#
    )
}

fn caption_word_styles(preset: CaptionPreset, highlight: &str) -> (String, String, String) {
    match preset {
        CaptionPreset::NeonGlow => (
            format!("color:{highlight};text-shadow:0 0 18px {highlight};transform:scale(1.06);"),
            "color:var(--brand-ink,#F7F5EF);opacity:0.9;".to_string(),
            "color:var(--brand-ink,#F7F5EF);opacity:0.9;".to_string(),
        ),
        CaptionPreset::Karaoke => (
            format!("color:#080404;background:{highlight};padding:2px 10px;border-radius:8px;"),
            format!("color:{highlight};"),
            "color:var(--brand-ink,#F7F5EF);opacity:0.65;".to_string(),
        ),
        CaptionPreset::MonoCaps => (
            format!("color:{highlight};border-bottom:3px solid {highlight};"),
            "color:var(--brand-ink,#F7F5EF);opacity:0.88;".to_string(),
            "color:var(--brand-ink,#F7F5EF);opacity:0.88;".to_string(),
        ),
        CaptionPreset::Minimal => (
            format!("color:{highlight};"),
            "color:var(--brand-ink,#F7F5EF);opacity:0.92;".to_string(),
            "color:var(--brand-ink,#F7F5EF);opacity:0.92;".to_string(),
        ),
    }
}

pub fn build_caption_overlay_html(
    clip: &Clip,
    caption: &CaptionBlock,
    active_word_idx: Option<usize>,
    canvas_width: u32,
    canvas_height: u32,
) -> String {
    let font_fam = sanitize_css_val(&caption.font_family).replace(['"', '\''], "");
    let font_fam = if font_fam.is_empty() {
        "Sora".to_string()
    } else {
        font_fam
    };
    let highlight = sanitize_css_val(&caption.highlight_color);
    let highlight = if highlight.is_empty() {
        "#F7B733".to_string()
    } else {
        highlight
    };

    let is_portrait = canvas_height > canvas_width;
    let base_font_px = if is_portrait {
        (canvas_width as f64 * 0.052).round() as u32
    } else {
        (canvas_height as f64 * 0.054).round() as u32
    }
    .clamp(28, 74);

    // Match Studio UI (`WebGLProgramStage.tsx`) normalized coordinate mapping:
    // y in [-0.35, max_y] -> 50% + y * 78% (portrait max_y=0.32 keeps bottom 22% clear; landscape max_y=0.42 reaches lower-third up to 84%).
    let raw_y = if clip.transform.y.is_finite() {
        clip.transform.y
    } else {
        0.0
    };
    let raw_x = if clip.transform.x.is_finite() {
        clip.transform.x
    } else {
        0.0
    };
    let max_y = if is_portrait { 0.32 } else { 0.42 };
    let clamped_y = raw_y.clamp(-0.35, max_y);
    let max_top = if is_portrait { 76.0 } else { 84.0 };
    let top_pct = (50.0 + clamped_y * 78.0).clamp(12.0, max_top);
    let left_pct = (50.0 + raw_x.clamp(-0.30, 0.30) * 100.0).clamp(14.0, 86.0);
    let scale = if clip.transform.scale.is_finite() {
        clip.transform.scale.clamp(0.4, 2.5)
    } else {
        1.0
    };
    let opacity = if clip.transform.opacity.is_finite() {
        clip.transform.opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };

    let (container_style, word_transform) = match caption.preset {
        CaptionPreset::NeonGlow => (
            "display:inline-flex;flex-wrap:wrap;justify-content:center;gap:0.28em;padding:16px 30px;border-radius:20px;background:rgba(8,4,4,0.82);border:1.5px solid rgba(247,245,239,0.15);box-shadow:0 12px 34px rgba(0,0,0,0.72);font-weight:800;letter-spacing:0.02em;text-transform:uppercase;".to_string(),
            "uppercase",
        ),
        CaptionPreset::MonoCaps => (
            "display:inline-flex;flex-wrap:wrap;justify-content:center;gap:0.26em;padding:14px 26px;border-radius:10px;background:rgba(8,4,4,0.92);border:1px solid rgba(247,245,239,0.22);box-shadow:0 10px 28px rgba(0,0,0,0.68);font-weight:800;letter-spacing:0.12em;text-transform:uppercase;".to_string(),
            "uppercase",
        ),
        CaptionPreset::Karaoke => (
            "display:inline-flex;flex-wrap:wrap;justify-content:center;gap:0.3em;padding:16px 30px;border-radius:999px;background:rgba(26,14,14,0.88);border:1px solid rgba(247,183,51,0.35);box-shadow:0 10px 30px rgba(0,0,0,0.68);font-weight:800;letter-spacing:0.01em;".to_string(),
            "none",
        ),
        CaptionPreset::Minimal => (
            "display:inline-flex;flex-wrap:wrap;justify-content:center;gap:0.26em;padding:10px 22px;border-radius:14px;background:rgba(8,4,4,0.62);font-weight:600;letter-spacing:0.01em;text-shadow:0 2px 12px rgba(0,0,0,0.85);".to_string(),
            "none",
        ),
    };

    let (active_css, spoken_css, idle_css) = caption_word_styles(caption.preset, &highlight);
    let words_html = if caption.words.is_empty() {
        format!(
            "<span style=\"color:var(--brand-ink,#F7F5EF);\">{}</span>",
            escape_html(&clip.label)
        )
    } else {
        caption
            .words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let is_active = active_word_idx == Some(i);
                let is_spoken = active_word_idx.is_some_and(|act| i < act);
                let span_style = if is_active {
                    &active_css
                } else if is_spoken {
                    &spoken_css
                } else {
                    &idle_css
                };
                format!(
                    "<span data-word-idx=\"{i}\" style=\"display:inline-block;{span_style}\">{}</span>",
                    escape_html(&w.text)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let active_json = serde_json::to_string(&active_css).unwrap_or_else(|_| "\"\"".to_string());
    let spoken_json = serde_json::to_string(&spoken_css).unwrap_or_else(|_| "\"\"".to_string());
    let idle_json = serde_json::to_string(&idle_css).unwrap_or_else(|_| "\"\"".to_string());

    format!(
        "<div style=\"position:fixed;top:{top_pct:.2}%;left:{left_pct:.2}%;transform:translate(-50%,-50%) scale({scale:.3});opacity:{opacity:.3};width:86%;text-align:center;text-wrap:balance;font-family:'{font_fam}',var(--brand-font-display),'Sora',sans-serif;font-size:{base_font_px}px;line-height:1.15;text-transform:{word_transform};\"><div style=\"{container_style}\">{words_html}</div></div><script>(function(){{const A={active_json},S={spoken_json},I={idle_json};window.__solarisSetCaptionWord=function(idx){{const spans=document.querySelectorAll('[data-word-idx]');for(const el of spans){{const i=Number(el.getAttribute('data-word-idx'));el.setAttribute('style','display:inline-block;'+(i===idx?A:(idx!==null&&i<idx?S:I)));}}void document.body.offsetHeight;}};}})();</script>"
    )
}

fn resolve_gfx_body_html(clip: &Clip, assets: &[SavedCodeAsset]) -> Option<String> {
    let ClipKind::Gfx { gfx } = &clip.payload else {
        return None;
    };
    let saved = gfx
        .asset_id
        .as_deref()
        .and_then(|aid| assets.iter().find(|a| a.id == aid));
    let raw_html = gfx
        .inline_html
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| saved.map(|a| a.html_source.as_str()));
    let merged_props = match saved {
        Some(a) => merge_json_props(&a.default_props, &gfx.props),
        None => gfx.props.clone(),
    };
    let template = match raw_html {
        Some(src) => src.to_string(),
        None => format!(
            "<div style=\"padding:16px 24px;border-radius:16px;background:rgba(8,4,4,0.88);border:1px solid var(--brand-accent);font-weight:600;font-size:28px;\">{}</div>",
            escape_html(&clip.label)
        ),
    };
    let body = interpolate_props(&template, &merged_props);

    let tx = if clip.transform.x.is_finite() {
        clip.transform.x
    } else {
        0.0
    };
    let ty = if clip.transform.y.is_finite() {
        clip.transform.y
    } else {
        0.0
    };
    let sc = if clip.transform.scale.is_finite() {
        clip.transform.scale.clamp(0.05, 10.0)
    } else {
        1.0
    };
    let rot = if clip.transform.rotation_deg.is_finite() {
        clip.transform.rotation_deg
    } else {
        0.0
    };
    let op = if clip.transform.opacity.is_finite() {
        clip.transform.opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if tx.abs() > 1e-4
        || ty.abs() > 1e-4
        || (sc - 1.0).abs() > 1e-4
        || rot.abs() > 1e-2
        || op < 0.999
    {
        let tx_pct = tx * 100.0;
        let ty_pct = ty * 100.0;
        Some(format!(
            "<div style=\"width:100%;height:100%;display:grid;place-items:center;opacity:{op:.3};transform:translate({tx_pct:.2}%,{ty_pct:.2}%) scale({sc:.3}) rotate({rot:.2}deg);\">{body}</div>"
        ))
    } else {
        Some(body)
    }
}

fn compute_caption_word_slices(clip: &Clip, caption: &CaptionBlock) -> Vec<(usize, f64, f64)> {
    if !clip.start_sec.is_finite() || !clip.duration_sec.is_finite() || clip.duration_sec <= 0.0 {
        return Vec::new();
    }
    let clip_end = clip.start_sec + clip.duration_sec;
    if !clip_end.is_finite() || clip_end <= clip.start_sec {
        return Vec::new();
    }
    let mut slices = Vec::with_capacity(caption.words.len());
    for (wi, word) in caption.words.iter().enumerate() {
        let raw_w_start = if word.start_sec.is_finite() {
            word.start_sec
        } else {
            clip.start_sec
        };
        let w_start = if wi == 0 {
            clip.start_sec
        } else {
            raw_w_start.clamp(clip.start_sec, clip_end)
        };
        let min_end = (w_start + 0.02).min(clip_end);
        let next_start = caption.words.get(wi + 1).and_then(|nw| {
            nw.start_sec
                .is_finite()
                .then(|| nw.start_sec.clamp(clip.start_sec, clip_end))
        });
        let raw_w_end = if word.end_sec.is_finite() {
            word.end_sec
        } else {
            clip_end
        };
        let w_end = if wi + 1 == caption.words.len() {
            clip_end
        } else {
            next_start.unwrap_or(raw_w_end).clamp(min_end, clip_end)
        };
        if w_end <= w_start + 1e-4 {
            continue;
        }
        slices.push((wi, w_start, w_end));
    }
    slices
}

fn find_active_caption_word_at(caption: &CaptionBlock, time_sec: f64) -> Option<usize> {
    if caption.words.is_empty() {
        return None;
    }
    if let Some((idx, _)) = caption.words.iter().enumerate().find(|(_, w)| {
        w.start_sec.is_finite()
            && w.end_sec.is_finite()
            && time_sec >= w.start_sec
            && time_sec <= w.end_sec
    }) {
        return Some(idx);
    }
    if let Some((idx, _)) = caption
        .words
        .iter()
        .enumerate()
        .rev()
        .find(|(_, w)| w.start_sec.is_finite() && time_sec >= w.start_sec)
    {
        return Some(idx);
    }
    Some(0)
}

fn collect_overlay_clips(project: &Project) -> Vec<&Clip> {
    let any_overlay_solo = project
        .tracks
        .iter()
        .any(|t| matches!(t.kind, TrackKind::Gfx | TrackKind::Text) && t.solo);
    let mut out = Vec::new();
    // Enforce explicit Z-order: G1 (GFX) first (underneath), T1 (TEXT) second (on top).
    for target_kind in [TrackKind::Gfx, TrackKind::Text] {
        for track in &project.tracks {
            if track.kind != target_kind || track.muted {
                continue;
            }
            if any_overlay_solo && !track.solo {
                continue;
            }
            for clip in &track.clips {
                if matches!(
                    clip.payload,
                    ClipKind::Gfx { .. } | ClipKind::Caption { .. }
                ) {
                    out.push(clip);
                }
            }
        }
    }
    out
}

/// Renders unmuted G1 (`Gfx`) and T1 (`Text`) clips to transparent RGBA PNGs
/// (or multi-frame RGBA sequences for animated GFX / word-synced captions) via Headless Chromium.
/// Always renders G1 *under* T1 so captions stay cleanly legible on top of motion graphics.
pub fn render_project_overlays_via_chromium(
    toolchain: &MediaToolchain,
    project: &Project,
    brand_tokens: &BrandTokens,
    temp_dir: &Path,
) -> Result<Vec<OverlayImageSpec>, MediaError> {
    render_project_overlays_via_chromium_with_assets(
        toolchain,
        project,
        brand_tokens,
        &[],
        &[],
        temp_dir,
    )
}

pub fn render_project_overlays_via_chromium_with_assets(
    toolchain: &MediaToolchain,
    project: &Project,
    brand_tokens: &BrandTokens,
    assets: &[SavedCodeAsset],
    brand_fonts: &[BrandFontData],
    temp_dir: &Path,
) -> Result<Vec<OverlayImageSpec>, MediaError> {
    render_project_overlays_via_chromium_with_progress(
        toolchain,
        project,
        brand_tokens,
        assets,
        brand_fonts,
        temp_dir,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_project_overlays_via_chromium_with_progress<F>(
    toolchain: &MediaToolchain,
    project: &Project,
    brand_tokens: &BrandTokens,
    assets: &[SavedCodeAsset],
    brand_fonts: &[BrandFontData],
    temp_dir: &Path,
    mut on_progress: F,
) -> Result<Vec<OverlayImageSpec>, MediaError>
where
    F: FnMut(OverlayRenderProgress),
{
    let overlay_clips = collect_overlay_clips(project);
    if overlay_clips.is_empty() {
        return Ok(Vec::new());
    }

    let chrome = toolchain.chromium_path.as_ref().ok_or_else(|| {
        MediaError::ToolchainUnavailable(
            "headless Chromium not found; cannot render G1/T1 overlays".to_string(),
        )
    })?;

    if !temp_dir.as_os_str().is_empty() {
        fs::create_dir_all(temp_dir)?;
    }

    let w = project.canvas.width.max(2);
    let h = project.canvas.height.max(2);
    let (gfx_w, gfx_h, gfx_dpr) = gfx_design_viewport(w, h);
    let project_fps = (project.canvas.fps_num.max(1) as f64 / project.canvas.fps_den.max(1) as f64)
        .clamp(1.0, 60.0);
    let max_anim_frames = std::env::var("SOLARIS_GFX_MAX_FRAMES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_MAX_ANIMATED_FRAMES);

    let overlay_fps = project_fps.min(30.0);
    let total_frames: usize = overlay_clips
        .iter()
        .filter(|c| c.start_sec.is_finite() && c.duration_sec.is_finite() && c.duration_sec > 0.0)
        .map(|clip| match &clip.payload {
            ClipKind::Gfx { .. } => {
                let Some(body_html) = resolve_gfx_body_html(clip, assets) else {
                    return 0;
                };
                if has_dynamic_gfx_animation(&body_html) && clip.duration_sec > 0.1 {
                    let raw_frames = (clip.duration_sec * overlay_fps).ceil() as usize;
                    raw_frames.clamp(2, max_anim_frames)
                } else {
                    1
                }
            }
            ClipKind::Caption { caption } => {
                if caption.words.len() > 1 {
                    compute_caption_word_slices(clip, caption).len()
                } else {
                    1
                }
            }
            _ => 0,
        })
        .sum::<usize>()
        .max(1);

    let mut completed_frames = 0usize;
    on_progress(OverlayRenderProgress {
        completed_frames: 0,
        total_frames,
        stage_label: format!("Launching Chromium for {total_frames} overlay frame(s)"),
    });

    let mut cdp = match ChromiumCdpSession::launch(chrome, w, h, temp_dir) {
        Ok(session) => Some(session),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "persistent CDP Chromium session unavailable; falling back to single-shot CLI screenshots"
            );
            None
        }
    };

    let mut specs = Vec::new();
    let mut idx = 0usize;

    for clip in overlay_clips {
        if !clip.start_sec.is_finite() || !clip.duration_sec.is_finite() || clip.duration_sec <= 0.0
        {
            continue;
        }
        let clip_end = clip.start_sec + clip.duration_sec;
        if !clip_end.is_finite() {
            continue;
        }

        match &clip.payload {
            ClipKind::Gfx { .. } => {
                let Some(body_html) = resolve_gfx_body_html(clip, assets) else {
                    continue;
                };
                idx += 1;
                if has_dynamic_gfx_animation(&body_html) && clip.duration_sec > 0.1 {
                    let raw_frames = (clip.duration_sec * overlay_fps).ceil() as usize;
                    let frame_count = raw_frames.clamp(2, max_anim_frames);
                    // Never clamp effective_fps upward to 1.0 on long animated clips (> max_anim_frames sec)
                    let effective_fps = (frame_count as f64 / clip.duration_sec).clamp(0.001, 30.0);
                    let seq_dir = temp_dir.join(format!("gfx_seq_{idx}"));
                    fs::create_dir_all(&seq_dir)?;

                    if let Some(session) = cdp.as_mut() {
                        // Navigate ONCE, then seek + screenshot in-process over CDP
                        session.set_viewport(gfx_w, gfx_h, gfx_dpr)?;
                        let html_path = seq_dir.join("scene.html");
                        let doc = build_overlay_html(
                            &body_html,
                            brand_tokens,
                            brand_fonts,
                            clip.duration_sec,
                            0.0,
                        );
                        fs::write(&html_path, doc)?;
                        session.navigate_and_wait(&html_path)?;
                        for f in 0..frame_count {
                            let t = if frame_count <= 1 {
                                0.0
                            } else {
                                (f as f64 / (frame_count - 1) as f64) * clip.duration_sec
                            };
                            let png_path = seq_dir.join(format!("frame_{:06}.png", f + 1));
                            session.evaluate_js(&format!(
                                "window.__solarisSeek({t:.6}); void document.body.offsetHeight;"
                            ))?;
                            session.capture_png(&png_path)?;
                            completed_frames += 1;
                            on_progress(OverlayRenderProgress {
                                completed_frames,
                                total_frames,
                                stage_label: format!(
                                    "Rendering GFX & captions ({completed_frames}/{total_frames} frames)"
                                ),
                            });
                        }
                    } else {
                        for f in 0..frame_count {
                            let t = if frame_count <= 1 {
                                0.0
                            } else {
                                (f as f64 / (frame_count - 1) as f64) * clip.duration_sec
                            };
                            let html_path = seq_dir.join(format!("frame_{:06}.html", f + 1));
                            let png_path = seq_dir.join(format!("frame_{:06}.png", f + 1));
                            let doc = build_overlay_html(
                                &body_html,
                                brand_tokens,
                                brand_fonts,
                                clip.duration_sec,
                                t,
                            );
                            fs::write(&html_path, doc)?;
                            run_chromium_screenshot(
                                chrome, &html_path, &png_path, gfx_w, gfx_h, gfx_dpr, temp_dir,
                            )?;
                            completed_frames += 1;
                            on_progress(OverlayRenderProgress {
                                completed_frames,
                                total_frames,
                                stage_label: format!(
                                    "Rendering GFX & captions ({completed_frames}/{total_frames} frames)"
                                ),
                            });
                        }
                    }

                    specs.push(OverlayImageSpec::sequence(
                        seq_dir.join("frame_%06d.png"),
                        clip.start_sec,
                        clip_end,
                        effective_fps,
                    ));
                } else {
                    let html_path = temp_dir.join(format!("overlay_{idx}.html"));
                    let png_path = temp_dir.join(format!("overlay_{idx}.png"));
                    let mid_sec = (clip.duration_sec * 0.5).max(0.0);
                    let doc = build_overlay_html(
                        &body_html,
                        brand_tokens,
                        brand_fonts,
                        clip.duration_sec,
                        mid_sec,
                    );
                    fs::write(&html_path, doc)?;
                    if let Some(session) = cdp.as_mut() {
                        session.set_viewport(gfx_w, gfx_h, gfx_dpr)?;
                        session.navigate_and_wait(&html_path)?;
                        session.capture_png(&png_path)?;
                    } else {
                        run_chromium_screenshot(
                            chrome, &html_path, &png_path, gfx_w, gfx_h, gfx_dpr, temp_dir,
                        )?;
                    }
                    completed_frames += 1;
                    on_progress(OverlayRenderProgress {
                        completed_frames,
                        total_frames,
                        stage_label: format!(
                            "Rendering GFX & captions ({completed_frames}/{total_frames} frames)"
                        ),
                    });
                    specs.push(OverlayImageSpec::single(png_path, clip.start_sec, clip_end));
                }
            }
            ClipKind::Caption { caption } => {
                if caption.words.len() > 1 {
                    let slices = compute_caption_word_slices(clip, caption);
                    if slices.is_empty() {
                        continue;
                    }
                    if let Some(session) = cdp.as_mut() {
                        // Navigate ONCE to caption HTML, then update active word per slice via CDP
                        session.set_viewport(w, h, 1.0)?;
                        idx += 1;
                        let html_path = temp_dir.join(format!("caption_clip_{idx}.html"));
                        let body_html =
                            build_caption_overlay_html(clip, caption, Some(slices[0].0), w, h);
                        let doc = build_overlay_html(
                            &body_html,
                            brand_tokens,
                            brand_fonts,
                            clip.duration_sec,
                            0.0,
                        );
                        fs::write(&html_path, doc)?;
                        session.navigate_and_wait(&html_path)?;
                        for (wi, w_start, w_end) in slices {
                            idx += 1;
                            let png_path = temp_dir.join(format!("caption_{idx}.png"));
                            session
                                .evaluate_js(&format!("window.__solarisSetCaptionWord({wi});"))?;
                            session.capture_png(&png_path)?;
                            completed_frames += 1;
                            on_progress(OverlayRenderProgress {
                                completed_frames,
                                total_frames,
                                stage_label: format!(
                                    "Rendering GFX & captions ({completed_frames}/{total_frames} frames)"
                                ),
                            });
                            specs.push(OverlayImageSpec::single(png_path, w_start, w_end));
                        }
                    } else {
                        for (wi, w_start, w_end) in slices {
                            idx += 1;
                            let body_html =
                                build_caption_overlay_html(clip, caption, Some(wi), w, h);
                            let html_path = temp_dir.join(format!("caption_{idx}.html"));
                            let png_path = temp_dir.join(format!("caption_{idx}.png"));
                            let doc = build_overlay_html(
                                &body_html,
                                brand_tokens,
                                brand_fonts,
                                w_end - w_start,
                                0.0,
                            );
                            fs::write(&html_path, doc)?;
                            run_chromium_screenshot(
                                chrome, &html_path, &png_path, w, h, 1.0, temp_dir,
                            )?;
                            completed_frames += 1;
                            on_progress(OverlayRenderProgress {
                                completed_frames,
                                total_frames,
                                stage_label: format!(
                                    "Rendering GFX & captions ({completed_frames}/{total_frames} frames)"
                                ),
                            });
                            specs.push(OverlayImageSpec::single(png_path, w_start, w_end));
                        }
                    }
                } else {
                    idx += 1;
                    let active = (!caption.words.is_empty()).then_some(0);
                    let body_html = build_caption_overlay_html(clip, caption, active, w, h);
                    let html_path = temp_dir.join(format!("caption_{idx}.html"));
                    let png_path = temp_dir.join(format!("caption_{idx}.png"));
                    let doc = build_overlay_html(
                        &body_html,
                        brand_tokens,
                        brand_fonts,
                        clip.duration_sec,
                        0.0,
                    );
                    fs::write(&html_path, doc)?;
                    if let Some(session) = cdp.as_mut() {
                        session.set_viewport(w, h, 1.0)?;
                        session.navigate_and_wait(&html_path)?;
                        session.capture_png(&png_path)?;
                    } else {
                        run_chromium_screenshot(
                            chrome, &html_path, &png_path, w, h, 1.0, temp_dir,
                        )?;
                    }
                    completed_frames += 1;
                    on_progress(OverlayRenderProgress {
                        completed_frames,
                        total_frames,
                        stage_label: format!(
                            "Rendering GFX & captions ({completed_frames}/{total_frames} frames)"
                        ),
                    });
                    specs.push(OverlayImageSpec::single(png_path, clip.start_sec, clip_end));
                }
            }
            _ => {}
        }
    }

    Ok(specs)
}

/// Renders all unmuted active `G1` (first) and `T1` (on top) clips at `time_sec` into transparent
/// RGBA PNGs using the same persistent CDP Chromium pipeline for `preview.capture_frame` & Vision QA.
#[allow(clippy::too_many_arguments)]
pub fn render_single_frame_overlays_via_chromium(
    toolchain: &MediaToolchain,
    project: &Project,
    time_sec: f64,
    brand_tokens: &BrandTokens,
    saved_assets: &[SavedCodeAsset],
    brand_fonts: &[BrandFontFace],
    temp_dir: &Path,
) -> Result<Vec<PathBuf>, MediaError> {
    if !time_sec.is_finite() || time_sec < 0.0 {
        return Err(MediaError::ParseError(
            "time_sec must be finite and >= 0".to_string(),
        ));
    }

    let active_clips: Vec<&Clip> = collect_overlay_clips(project)
        .into_iter()
        .filter(|clip| {
            clip.start_sec.is_finite()
                && clip.duration_sec.is_finite()
                && clip.duration_sec > 0.0
                && time_sec >= clip.start_sec
                && time_sec < clip.start_sec + clip.duration_sec
        })
        .collect();

    if active_clips.is_empty() {
        return Ok(Vec::new());
    }

    let chrome = toolchain.chromium_path.as_ref().ok_or_else(|| {
        MediaError::ToolchainUnavailable(
            "headless Chromium not found; cannot render single-frame overlays".to_string(),
        )
    })?;

    if !temp_dir.as_os_str().is_empty() {
        fs::create_dir_all(temp_dir)?;
    }

    let w = project.canvas.width.max(2);
    let h = project.canvas.height.max(2);
    let (gfx_w, gfx_h, gfx_dpr) = gfx_design_viewport(w, h);
    let mut cdp = match ChromiumCdpSession::launch(chrome, w, h, temp_dir) {
        Ok(session) => Some(session),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "persistent CDP Chromium session unavailable for single-frame QA; falling back to CLI"
            );
            None
        }
    };

    let mut pngs = Vec::with_capacity(active_clips.len());
    for (idx, clip) in active_clips.into_iter().enumerate() {
        let local_t = (time_sec - clip.start_sec).clamp(0.0, clip.duration_sec);
        let (body_html, vp_w, vp_h, vp_dpr) = match &clip.payload {
            ClipKind::Gfx { .. } => {
                let Some(body) = resolve_gfx_body_html(clip, saved_assets) else {
                    continue;
                };
                (body, gfx_w, gfx_h, gfx_dpr)
            }
            ClipKind::Caption { caption } => {
                let active_idx = find_active_caption_word_at(caption, time_sec);
                (
                    build_caption_overlay_html(clip, caption, active_idx, w, h),
                    w,
                    h,
                    1.0,
                )
            }
            _ => continue,
        };

        let html_path = temp_dir.join(format!("qa_overlay_{idx}.html"));
        let png_path = temp_dir.join(format!("qa_overlay_{idx}.png"));
        let doc = build_overlay_html(
            &body_html,
            brand_tokens,
            brand_fonts,
            clip.duration_sec,
            local_t,
        );
        fs::write(&html_path, doc)?;

        if let Some(session) = cdp.as_mut() {
            session.set_viewport(vp_w, vp_h, vp_dpr)?;
            session.navigate_and_wait(&html_path)?;
            session.evaluate_js(&format!(
                "window.__solarisSeek({local_t:.6}); void document.body.offsetHeight;"
            ))?;
            session.capture_png(&png_path)?;
        } else {
            run_chromium_screenshot(chrome, &html_path, &png_path, vp_w, vp_h, vp_dpr, temp_dir)?;
        }
        pngs.push(png_path);
    }

    Ok(pngs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use solaris_core_ir::{AspectPreset, AudioSettings, CaptionWord, GfxInstance, Transform2D};

    #[test]
    fn canonicalizes_and_percent_encodes_relative_file_uri() {
        let dir = std::env::temp_dir().join(format!("solaris uri test {}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("overlay #1.html");
        fs::write(&file, "<div>ok</div>").unwrap();
        let uri = path_to_file_uri(&file).unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(uri.contains("%20"));
        assert!(uri.contains("%231.html"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn builds_offline_html_with_csp_gsap_sora_brand_fonts_and_dom_load_seek() {
        let tokens = BrandTokens {
            bg: "#080404".into(),
            surface: "#1A0E0E".into(),
            ink: "#F7F5EF".into(),
            accent: "#F05A00".into(),
            accent_secondary: "#F7B733".into(),
            font_display: "CustomBrand".into(),
            font_body: "Sora".into(),
        };
        let fonts = vec![
            BrandFontData {
                family: "CustomBrand".into(),
                mime: "font/woff2".into(),
                data_base64: "AAAA\nBBBB==".into(),
            },
            // Malicious base64 / mime must be rejected strictly
            BrandFontData {
                family: "EvilB64".into(),
                mime: "font/woff2".into(),
                data_base64: "AAAA\") ; background: url(http://evil)".into(),
            },
            BrandFontData {
                family: "EvilMime".into(),
                mime: "font/woff2;evil".into(),
                data_base64: "AAAA".into(),
            },
        ];
        let doc = build_overlay_html("<div>Hello</div>", &tokens, &fonts, 2.0, 0.5);
        assert!(doc.contains("Content-Security-Policy"));
        assert!(doc.contains("default-src 'none'"));
        assert!(doc.contains("font-family: \"Sora\""));
        assert!(doc.contains("font-family: \"CustomBrand\""));
        assert!(doc.contains("data:font/woff2;base64,AAAABBBB=="));
        assert!(!doc.contains("EvilB64"));
        assert!(!doc.contains("EvilMime"));
        assert!(doc.contains("window.__solarisSeek"));
        assert!(doc.contains("DOMContentLoaded"));
        assert!(doc.contains("window.addEventListener(\"load\""));
        assert!(doc.contains("globalTimeline.pause"));
        assert!(doc.len() > 50_000, "expected vendored GSAP + Sora inlined");
    }

    #[test]
    fn resolves_saved_asset_preserves_js_braces_and_safe_zone_captions() {
        let asset = SavedCodeAsset {
            id: "stat-card-solar".into(),
            name: "Stat Card".into(),
            category: "cards".into(),
            tags: vec![],
            props_schema: serde_json::json!({}),
            default_props: serde_json::json!({ "value": "10 HRS", "label": "SAVED" }),
            html_source: "<div>{{value}} - {{label}}</div><script>const obj = {{a: 1}};</script>"
                .into(),
            default_duration_sec: 3.0,
            created_at: Utc::now(),
        };
        let gfx_clip = Clip {
            id: "g1".into(),
            label: "Stat".into(),
            start_sec: 0.0,
            duration_sec: 3.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Gfx {
                gfx: GfxInstance {
                    asset_id: Some("stat-card-solar".into()),
                    props: serde_json::json!({ "value": "99%" }),
                    inline_html: None,
                },
            },
        };
        let resolved = resolve_gfx_body_html(&gfx_clip, &[asset]).unwrap();
        assert_eq!(
            resolved,
            "<div>99% - SAVED</div><script>const obj = {{a: 1}};</script>"
        );

        let cap = CaptionBlock {
            words: vec![
                CaptionWord {
                    text: "SHIP".into(),
                    start_sec: 0.0,
                    end_sec: 0.5,
                },
                CaptionWord {
                    text: "FAST".into(),
                    start_sec: 0.5,
                    end_sec: 1.0,
                },
            ],
            preset: CaptionPreset::Karaoke,
            font_family: "Sora".into(),
            highlight_color: "#F7B733".into(),
        };
        let default_cap_clip = Clip {
            id: "t1".into(),
            label: "SHIP FAST".into(),
            start_sec: 0.0,
            duration_sec: 1.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: cap.clone(),
            },
        };
        let default_html = build_caption_overlay_html(&default_cap_clip, &cap, Some(1), 1080, 1920);
        assert!(default_html.contains("top:50.00%"));
        assert!(default_html.contains("#F7B733"));
        assert!(default_html.contains("FAST"));
        assert!(default_html.contains("window.__solarisSetCaptionWord"));

        let shifted_cap_clip = Clip {
            transform: Transform2D {
                y: 0.24,
                scale: f64::NAN,
                ..Transform2D::default()
            },
            ..default_cap_clip
        };
        let shifted_html = build_caption_overlay_html(&shifted_cap_clip, &cap, Some(1), 1080, 1920);
        assert!(shifted_html.contains("top:68.72%"));
        assert!(shifted_html.contains("scale(1.000)"));
    }

    #[test]
    fn caption_word_slicing_never_panics_on_near_end_words_or_missing_chromium() {
        let cap = CaptionBlock {
            words: vec![
                CaptionWord {
                    text: "A".into(),
                    start_sec: 0.995,
                    end_sec: 1.0,
                },
                CaptionWord {
                    text: "B".into(),
                    start_sec: 0.999,
                    end_sec: 1.0,
                },
            ],
            preset: CaptionPreset::Minimal,
            font_family: "Sora".into(),
            highlight_color: "#F7B733".into(),
        };
        let clip = Clip {
            id: "t_edge".into(),
            label: "Edge".into(),
            start_sec: 0.0,
            duration_sec: 1.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: cap.clone(),
            },
        };
        // Previously panicked when w_start + 0.02 > clip_end
        let slices = compute_caption_word_slices(&clip, &cap);
        assert!(!slices.is_empty());

        // Returning Err (never silent drop) when overlays exist and Chromium is unavailable
        let mut proj = Project::new_default("no_chrome", AspectPreset::Vertical9x16);
        let no_chrome = MediaToolchain {
            ffmpeg_path: None,
            ffprobe_path: None,
            chromium_path: None,
        };
        let tokens = solaris_core_ir::BrandPlaybook::solaris_default().tokens;
        // No overlays -> Ok(empty)
        assert!(
            render_project_overlays_via_chromium(&no_chrome, &proj, &tokens, Path::new("/tmp"))
                .unwrap()
                .is_empty()
        );
        // With an active overlay -> Err(ToolchainUnavailable)
        proj.tracks[0].clips.push(clip);
        assert!(matches!(
            render_project_overlays_via_chromium(&no_chrome, &proj, &tokens, Path::new("/tmp")),
            Err(MediaError::ToolchainUnavailable(_))
        ));
        assert!(matches!(
            render_single_frame_overlays_via_chromium(
                &no_chrome,
                &proj,
                0.5,
                &tokens,
                &[],
                &[],
                Path::new("/tmp"),
            ),
            Err(MediaError::ToolchainUnavailable(_))
        ));
        assert_eq!(
            parse_devtools_port("DevTools listening on ws://127.0.0.1:43210/devtools/browser/abc"),
            Some(43210)
        );
    }

    #[test]
    fn gfx_design_viewport_matches_stage_coordinate_space() {
        assert_eq!(gfx_design_viewport(1080, 1920), (540, 960, 2.0));
        assert_eq!(gfx_design_viewport(1920, 1080), (960, 540, 2.0));
        assert_eq!(gfx_design_viewport(1080, 1080), (720, 720, 1.5));
        assert_eq!(gfx_design_viewport(320, 240), (320, 240, 1.0));
    }

    #[test]
    fn caption_css_sanitization_prevents_quote_attribute_breakout_and_allows_landscape_lower_third()
    {
        let cap = CaptionBlock {
            words: vec![CaptionWord {
                text: "HELLO".into(),
                start_sec: 0.0,
                end_sec: 1.0,
            }],
            preset: CaptionPreset::NeonGlow,
            font_family: "Sora'\" onclick=\"alert(1)".into(),
            highlight_color: "#F7B733\" onmouseover=\"alert(1)'".into(),
        };
        let clip = Clip {
            id: "t_inj".into(),
            label: "HELLO".into(),
            start_sec: 0.0,
            duration_sec: 1.0,
            source_in_sec: 0.0,
            transform: Transform2D {
                y: 0.42,
                ..Transform2D::default()
            },
            audio: AudioSettings::default(),
            payload: ClipKind::Caption {
                caption: cap.clone(),
            },
        };

        let landscape_html = build_caption_overlay_html(&clip, &cap, Some(0), 1920, 1080);
        // Quotes stripped so attributes & font-family cannot break out
        assert!(!landscape_html.contains("\" onmouseover="));
        assert!(!landscape_html.contains("onclick=\""));
        assert!(
            landscape_html
                .contains("font-family:'Sora onclick=alert(1)',var(--brand-font-display)")
        );
        assert!(landscape_html.contains("top:82.76%"));

        // Portrait remains safely clamped at y = 0.32 -> 74.96%
        let portrait_html = build_caption_overlay_html(&clip, &cap, Some(0), 1080, 1920);
        assert!(portrait_html.contains("top:74.96%"));
    }

    #[test]
    fn cdp_http_reader_respects_content_length_without_waiting_for_keepalive_eof() {
        struct KeepAliveReader {
            data: Vec<u8>,
            pos: usize,
        }
        impl Read for KeepAliveReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.pos >= self.data.len() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "simulated keep-alive stall (EOF never arrives)",
                    ));
                }
                let n = buf.len().min(self.data.len() - self.pos);
                buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            }
        }

        let json_body =
            r#"[{"type":"page","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/1"}]"#;
        let raw_resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json_body.len(),
            json_body
        );
        let mut reader = KeepAliveReader {
            data: raw_resp.into_bytes(),
            pos: 0,
        };
        let (status, body) = read_http_response_body(&mut reader).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, json_body.as_bytes());

        // Also verify Transfer-Encoding: chunked without EOF
        let chunked = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            json_body.len(),
            json_body
        );
        let mut chunked_reader = KeepAliveReader {
            data: chunked.into_bytes(),
            pos: 0,
        };
        let (c_status, c_body) = read_http_response_body(&mut chunked_reader).unwrap();
        assert_eq!(c_status, 200);
        assert_eq!(c_body, json_body.as_bytes());
    }

    #[test]
    fn persistent_cdp_session_launches_and_renders_animated_frames_when_chrome_available() {
        let toolchain = MediaToolchain::from_env();
        let Some(chrome) = toolchain.chromium_path.as_ref() else {
            return;
        };

        let dir = std::env::temp_dir().join(format!("solaris_cdp_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let _guard = TempDirGuard(dir.clone());

        let mut session = ChromiumCdpSession::launch(chrome, 320, 480, &dir)
            .expect("persistent CDP Chromium session must connect cleanly on keep-alive HTTP");
        let html_path = dir.join("scene.html");
        let tokens = solaris_core_ir::BrandPlaybook::solaris_default().tokens;
        let doc = build_overlay_html(
            r#"<div class="clip" data-start="0" data-duration="2"><h1 id="t">SOLARIS</h1><script>window.__timelines={};const tl=gsap.timeline({paused:true});tl.fromTo('#t',{opacity:0},{opacity:1,duration:1},0);window.__timelines.main=tl;</script></div>"#,
            &tokens,
            &[],
            2.0,
            0.0,
        );
        fs::write(&html_path, doc).unwrap();
        session.navigate_and_wait(&html_path).unwrap();
        session
            .evaluate_js("window.__solarisSeek(1.0); void document.body.offsetHeight;")
            .unwrap();
        let png_path = dir.join("out.png");
        session.capture_png(&png_path).unwrap();
        let meta = fs::metadata(&png_path).unwrap();
        assert!(meta.len() > 200, "expected non-empty PNG from CDP");
    }
}
