use std::{
    collections::HashMap,
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use anyhow::Result;
use axum::{
    body::Body,
    extract::Request,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Local};
use mime_guess::from_path;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tokio::{fs::File, io::AsyncReadExt};
use tower::ServiceExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeFile;
use tauri_plugin_dialog::DialogExt;

// ─── 全局状态 ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    root_dir: Arc<Mutex<Option<PathBuf>>>,
    host_name: String,
}

static GLOBAL_STATE: Lazy<Arc<Mutex<ServerInfo>>> =
    Lazy::new(|| Arc::new(Mutex::new(ServerInfo::default())));
static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        // 局域网访问禁止走环境代理（如 ALL_PROXY），避免本地地址请求失败
        .no_proxy()
        .build()
        .expect("failed to build reqwest client")
});

#[derive(Default, Clone, Serialize)]
struct ServerInfo {
    running: bool,
    ip: String,
    port: u16,
    root_dir: String,
}

// ─── 数据结构 ────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct FileEntry {
    name: String,
    path: String, // 相对路径（URL path）
    is_dir: bool,
    size: u64,
    size_human: String,
    modified: String,
    modified_ts: u64,
    ext: String,
    icon: String,
}

#[derive(Serialize)]
struct DirListing {
    path: String,
    entries: Vec<FileEntry>,
}

#[derive(Serialize)]
struct ApiInfo {
    name: String,
    version: String,
    host: String,
    root: String,
}

const SHARED_BROWSER_JS: &str = include_str!("../../src/shared-file-browser.js");
const SHARED_BROWSER_CSS: &str = include_str!("../../src/shared-file-browser.css");
const WEB_INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>LocalShare Web</title>
  <link rel="stylesheet" href="/assets/shared-file-browser.css" />
  <style>
    :root {
      --lsb-accent: #c5622d;
      --lsb-accent-soft: rgba(197, 98, 45, 0.1);
      --lsb-text: #1e1d1a;
      --lsb-muted: #6c675d;
      --lsb-line: rgba(77, 64, 45, 0.14);
      --lsb-line-strong: rgba(77, 64, 45, 0.22);
      --lsb-panel: #fffdf7;
      --lsb-toolbar-bg: rgba(255, 255, 255, 0.5);
      --lsb-surface2: #f7f1e6;
      --lsb-icon-bg: linear-gradient(135deg, rgba(197, 98, 45, 0.12), rgba(44, 115, 98, 0.12));
      --lsb-danger: #b94a34;
    }

    * { box-sizing: border-box; }
    body {
      margin: 0;
      font-family: "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
      color: var(--lsb-text);
      background:
        radial-gradient(circle at top left, rgba(197, 98, 45, 0.18), transparent 28%),
        radial-gradient(circle at top right, rgba(44, 115, 98, 0.15), transparent 24%),
        linear-gradient(180deg, #f7f2e8 0%, #efe6d4 100%);
      min-height: 100vh;
      padding: 24px;
    }

    .shell {
      max-width: 980px;
      margin: 0 auto;
      background: rgba(255, 251, 243, 0.92);
      backdrop-filter: blur(16px);
      border: 1px solid rgba(255, 255, 255, 0.6);
      border-radius: 28px;
      box-shadow: 0 18px 40px rgba(84, 57, 26, 0.12);
      overflow: hidden;
    }

    .hero {
      padding: 28px 28px 18px;
      background: linear-gradient(135deg, rgba(197, 98, 45, 0.08), rgba(44, 115, 98, 0.08));
      border-bottom: 1px solid var(--lsb-line);
    }

    .eyebrow {
      display: inline-block;
      padding: 6px 10px;
      border-radius: 999px;
      font-size: 12px;
      color: var(--lsb-accent);
      background: var(--lsb-accent-soft);
      margin-bottom: 12px;
    }

    h1 {
      margin: 0;
      font-size: clamp(28px, 5vw, 42px);
      line-height: 1.05;
      letter-spacing: -0.03em;
    }

    .sub {
      margin: 10px 0 0;
      color: var(--lsb-muted);
      max-width: 680px;
      line-height: 1.6;
    }

    .browser-wrap {
      height: min(72vh, 920px);
      min-height: 420px;
    }

    @media (max-width: 760px) {
      body { padding: 14px; }
      .hero { padding-left: 18px; padding-right: 18px; }
      .browser-wrap { height: auto; min-height: calc(100vh - 220px); }
    }
  </style>
</head>
<body>
  <main class="shell">
    <section class="hero">
      <div class="eyebrow">LocalShare Web</div>
      <h1>局域网文件浏览</h1>
      <p class="sub">浏览器可直接访问共享目录，支持进入子目录、在线播放视频和下载文件。这个文件浏览区与桌面客户端共用同一套界面逻辑。</p>
    </section>
    <section id="browser" class="browser-wrap"></section>
  </main>

  <script type="module">
    import { createSharedFileBrowser } from "/assets/shared-file-browser.js";

    const INLINE_VIDEO_EXTS = new Set(["mp4", "webm", "mov", "m4v"]);
    const INLINE_IMAGE_EXTS = new Set(["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "avif"]);
    const browser = createSharedFileBrowser({
      mount: document.getElementById("browser"),
      onNavigate: (path) => load(path, true),
      getTopActions: () => [
        { label: "刷新", onClick: () => load(currentPath, false) },
      ],
      getItemActions: (entry) => {
        if (entry.is_dir) return [];
        const actions = [];
        if (INLINE_VIDEO_EXTS.has((entry.ext || "").toLowerCase())) {
          actions.push({
            label: "直接播放",
            kind: "secondary",
            href: `/api/view?path=${encodeURIComponent(entry.path)}`,
            targetBlank: true,
          });
        }
        if (INLINE_IMAGE_EXTS.has((entry.ext || "").toLowerCase())) {
          actions.push({
            label: "直接查看",
            kind: "secondary",
            href: `/api/view?path=${encodeURIComponent(entry.path)}`,
            targetBlank: true,
          });
        }
        actions.push({
          label: "下载文件",
          kind: "primary",
          href: `/api/download?path=${encodeURIComponent(entry.path)}`,
        });
        return actions;
      },
    });

    let currentPath = "/";

    async function load(path, pushHistory) {
      currentPath = path || new URLSearchParams(window.location.search).get("path") || "/";
      if (pushHistory) {
        const url = new URL(window.location.href);
        if (currentPath !== "/") url.searchParams.set("path", currentPath);
        else url.searchParams.delete("path");
        history.pushState({ path: currentPath }, "", url);
      } else {
        const url = new URL(window.location.href);
        if (currentPath !== "/") url.searchParams.set("path", currentPath);
        else url.searchParams.delete("path");
        history.replaceState({ path: currentPath }, "", url);
      }

      browser.setLoading("正在加载目录...");
      try {
        const resp = await fetch(`/api/list?path=${encodeURIComponent(currentPath)}`);
        if (!resp.ok) {
          throw new Error(await resp.text() || `HTTP ${resp.status}`);
        }
        const listing = await resp.json();
        browser.setData({
          path: listing.path,
          entries: listing.entries,
          statusText: `${listing.entries.length} 个项目`,
        });
      } catch (err) {
        browser.setError(String(err.message || err));
      }
    }

    window.addEventListener("popstate", (event) => {
      load(event.state?.path || "/", false);
    });

    load(null, false);
  </script>
</body>
</html>
"#;

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

fn human_size(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = n as f64;
    let mut i = 0;
    while size >= 1024.0 && i < UNITS.len() - 1 {
        size /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", n)
    } else {
        format!("{:.1} {}", size, UNITS[i])
    }
}

fn file_icon(ext: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "📁";
    }
    match ext {
        "docx" | "doc" => "📄",
        "xlsx" | "xls" | "csv" => "📊",
        "pptx" | "ppt" => "📋",
        "pdf" => "📕",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" => "🖼",
        "mp4" | "mov" | "avi" | "mkv" | "webm" => "🎬",
        "mp3" | "wav" | "flac" | "aac" | "ogg" => "🎵",
        "zip" | "rar" | "7z" | "tar" | "gz" => "🗜",
        "txt" | "md" => "📝",
        "py" => "🐍",
        "js" | "ts" => "📜",
        "html" | "htm" => "🌐",
        "css" => "🎨",
        "rs" => "🦀",
        _ => "📄",
    }
}

fn get_local_ip() -> String {
    // 优先使用本地网卡枚举，离线场景也能拿到局域网地址
    if let Ok(ip) = local_ip_address::local_ip() {
        if !ip.is_loopback() {
            return ip.to_string();
        }
    }
    // 兜底：通过连接外部地址来推断本机 IP
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

fn list_dir(root: &Path, rel: &str) -> Result<DirListing> {
    let dir = if rel.is_empty() || rel == "/" {
        root.to_path_buf()
    } else {
        root.join(rel.trim_start_matches('/'))
    };

    // 安全检查：不能跳出 root
    let dir = dir.canonicalize()?;
    let root_canon = root.canonicalize()?;
    if !dir.starts_with(&root_canon) {
        anyhow::bail!("Access denied");
    }

    let mut entries: Vec<FileEntry> = Vec::new();

    let mut read_dir = std::fs::read_dir(&dir)?;
    let mut raw: Vec<_> = vec![];
    while let Some(Ok(e)) = read_dir.next() {
        raw.push(e);
    }
    // 目录在前，按名称排序
    raw.sort_by(|a, b| {
        let ad = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let bd = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        bd.cmp(&ad).then(a.file_name().cmp(&b.file_name()))
    });

    for entry in raw {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        // 过滤系统隐藏文件，避免出现 .localized 这类无意义条目
        if name.starts_with('.') {
            continue;
        }
        let is_dir = meta.is_dir();
        let size = if is_dir { 0 } else { meta.len() };
        let ext = if is_dir {
            String::new()
        } else {
            Path::new(&name)
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        };
        let modified_ts = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let modified = if modified_ts > 0 {
            let dt: DateTime<Local> =
                DateTime::from(std::time::UNIX_EPOCH + Duration::from_secs(modified_ts));
            dt.format("%Y-%m-%d %H:%M").to_string()
        } else {
            "—".to_string()
        };

        // URL 路径
        let entry_rel = if rel.is_empty() || rel == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", rel.trim_end_matches('/'), name)
        };

        entries.push(FileEntry {
            icon: file_icon(&ext, is_dir).to_string(),
            name,
            path: entry_rel,
            is_dir,
            size,
            size_human: if is_dir { "—".to_string() } else { human_size(size) },
            modified,
            modified_ts,
            ext,
        });
    }

    Ok(DirListing {
        path: if rel.is_empty() { "/".to_string() } else { rel.to_string() },
        entries,
    })
}

// ─── Axum 路由处理 ────────────────────────────────────────────────────────────

async fn api_info(State(state): State<AppState>) -> Json<ApiInfo> {
    let root = state
        .root_dir
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Json(ApiInfo {
        name: state.host_name.clone(),
        version: "2.0".into(),
        host: state.host_name.clone(),
        root,
    })
}

async fn web_index() -> Html<&'static str> {
    Html(WEB_INDEX_HTML)
}

async fn shared_browser_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        SHARED_BROWSER_JS,
    )
}

async fn shared_browser_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        SHARED_BROWSER_CSS,
    )
}

#[derive(Deserialize)]
struct PathQuery {
    path: Option<String>,
}

async fn api_list(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<DirListing>, (StatusCode, String)> {
    let root = shared_root(&state)?;

    let rel = q.path.unwrap_or_default();
    list_dir(&root, &rel).map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

fn shared_root(state: &AppState) -> Result<PathBuf, (StatusCode, String)> {
    state
        .root_dir
        .lock()
        .unwrap()
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "未选择共享目录".into()))
}

fn resolve_shared_file(root: &Path, rel: &str) -> Result<PathBuf, (StatusCode, String)> {
    let full = root.join(rel.trim_start_matches('/'));
    let full = full
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "文件不存在".into()))?;

    let root_canon = root
        .canonicalize()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !full.starts_with(&root_canon) {
        return Err((StatusCode::FORBIDDEN, "禁止访问".into()));
    }
    if !full.is_file() {
        return Err((StatusCode::BAD_REQUEST, "不是文件".into()));
    }

    Ok(full)
}

async fn api_download(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Response, (StatusCode, String)> {
    let root = shared_root(&state)?;

    let rel = q.path.ok_or((StatusCode::BAD_REQUEST, "缺少 path 参数".into()))?;
    let full = resolve_shared_file(&root, &rel)?;

    let file_name = full.file_name().unwrap().to_string_lossy().to_string();
    let mime = from_path(&full).first_or_octet_stream();
    let mut file = File::open(&full)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let encoded_name = urlencoding_encode(&file_name);
    let content_disposition =
        format!("attachment; filename=\"{}\"; filename*=UTF-8''{}", file_name, encoded_name);

    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::CONTENT_DISPOSITION, content_disposition),
            (
                header::CONTENT_LENGTH,
                buf.len().to_string(),
            ),
        ],
        buf,
    )
        .into_response())
}

async fn api_view(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
    req: Request,
) -> Result<Response, (StatusCode, String)> {
    let root = shared_root(&state)?;
    let rel = q.path.ok_or((StatusCode::BAD_REQUEST, "缺少 path 参数".into()))?;
    let full = resolve_shared_file(&root, &rel)?;

    let response = ServeFile::new(full)
        .oneshot(req.map(|_| Body::empty()))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(response.map(Body::new))
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

// ─── Tauri 命令 ───────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct ServerStatus {
    running: bool,
    ip: String,
    port: u16,
    root_dir: String,
}

/// 启动文件服务器
#[tauri::command]
async fn start_server(root_dir: String, port: u16) -> Result<ServerStatus, String> {
    let root = PathBuf::from(&root_dir);
    if !root.is_dir() {
        return Err("目录不存在".into());
    }

    let ip = get_local_ip();
    let host_name = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "LocalShare".into());

    let state = AppState {
        root_dir: Arc::new(Mutex::new(Some(root.clone()))),
        host_name: host_name.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(web_index))
        .route("/assets/shared-file-browser.js", get(shared_browser_js))
        .route("/assets/shared-file-browser.css", get(shared_browser_css))
        .route("/api/info", get(api_info))
        .route("/api/list", get(api_list))
        .route("/api/view", get(api_view))
        .route("/api/download", get(api_download))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("端口 {} 监听失败: {}", port, e))?;

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("axum serve exited: {}", e);
        }
    });

    // 更新全局状态
    {
        let mut info = GLOBAL_STATE.lock().unwrap();
        *info = ServerInfo {
            running: true,
            ip: ip.clone(),
            port,
            root_dir: root_dir.clone(),
        };
    }

    Ok(ServerStatus {
        running: true,
        ip,
        port,
        root_dir,
    })
}

/// 获取服务器状态
#[tauri::command]
fn get_server_status() -> ServerStatus {
    let info = GLOBAL_STATE.lock().unwrap().clone();
    ServerStatus {
        running: info.running,
        ip: info.ip,
        port: info.port,
        root_dir: info.root_dir,
    }
}

/// 用系统默认应用打开文件（客户端下载后调用）
#[tauri::command]
async fn open_file_with_system(file_path: String) -> Result<(), String> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("文件不存在: {}", file_path));
    }
    open::that(&path).map_err(|e| format!("打开失败: {}", e))
}

#[tauri::command]
async fn open_with_system(target: String) -> Result<(), String> {
    open::that(target).map_err(|e| format!("打开失败: {}", e))
}

/// 下载文件到本地临时目录并打开
#[tauri::command]
async fn download_and_open(url: String, file_name: String) -> Result<String, String> {
    // 临时目录
    let tmp_dir = std::env::temp_dir().join("localshare_open");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let safe_name = file_name.replace(['/', '\\'], "_");
    let dest = tmp_dir.join(format!("{}_{}", ts, safe_name));

    // 下载
    let resp = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;

    // 用系统应用打开
    open::that(&dest).map_err(|e| format!("打开失败: {}", e))?;
    Ok(dest.to_string_lossy().to_string())
}

/// 下载文件到本机 Downloads 目录
#[tauri::command]
async fn download_file(
    app: tauri::AppHandle,
    url: String,
    file_name: String,
) -> Result<String, String> {
    let dl_dir = app
        .path()
        .download_dir()
        .map_err(|e| format!("获取下载目录失败: {}", e))?;
    std::fs::create_dir_all(&dl_dir).map_err(|e| e.to_string())?;

    let safe_name = file_name.replace(['/', '\\'], "_");
    let mut target = dl_dir.join(&safe_name);
    if target.exists() {
        let stem = std::path::Path::new(&safe_name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let ext = std::path::Path::new(&safe_name)
            .extension()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        for i in 1..=9999 {
            let name = if ext.is_empty() {
                format!("{} ({})", stem, i)
            } else {
                format!("{} ({}).{}", stem, i, ext)
            };
            let candidate = dl_dir.join(name);
            if !candidate.exists() {
                target = candidate;
                break;
            }
        }
    }

    let resp = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
    Ok(target.to_string_lossy().to_string())
}

/// 扫描局域网发现主机（简单端口扫描）
#[tauri::command]
async fn discover_hosts(port: u16) -> Vec<HashMap<String, String>> {
    let local_ip = get_local_ip();
    let parts: Vec<&str> = local_ip.split('.').collect();
    if parts.len() != 4 {
        return vec![];
    }
    let prefix = format!("{}.{}.{}.", parts[0], parts[1], parts[2]);

    let mut handles = vec![];
    for i in 1u32..=254 {
        let ip = format!("{}{}", prefix, i);
        let url = format!("http://{}:{}/api/info", ip, port);
        handles.push(tokio::spawn(async move {
            match tokio::time::timeout(
                Duration::from_millis(700),
                HTTP_CLIENT.get(&url).send(),
            )
            .await
            {
                Ok(Ok(resp)) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        let mut m = HashMap::new();
                        m.insert("ip".into(), ip);
                        m.insert(
                            "name".into(),
                            json["name"].as_str().unwrap_or("LocalShare").to_string(),
                        );
                        Some(m)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }));
    }

    let mut result = vec![];
    for h in handles {
        if let Ok(Some(m)) = h.await {
            result.push(m);
        }
    }
    result
}

/// 获取本机 IP
#[tauri::command]
fn get_my_ip() -> String {
    get_local_ip()
}

/// 选择共享目录（由 Rust 侧弹系统目录选择框）
#[tauri::command]
async fn pick_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
    app.dialog().file().pick_folder(move |folder| {
        let picked = folder.and_then(|f| {
            f.into_path()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        });
        let _ = tx.send(picked);
    });
    rx.await.map_err(|e| e.to_string())
}

// ─── main ────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            start_server,
            get_server_status,
            open_file_with_system,
            open_with_system,
            download_and_open,
            download_file,
            discover_hosts,
            get_my_ip,
            pick_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
