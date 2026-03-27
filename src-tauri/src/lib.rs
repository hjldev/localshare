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
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Local};
use mime_guess::from_path;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncReadExt};
use tower_http::cors::{Any, CorsLayer};
use tauri_plugin_dialog::DialogExt;

// ─── 全局状态 ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    root_dir: Arc<Mutex<Option<PathBuf>>>,
    host_name: String,
    server_port: u16,
}

static GLOBAL_STATE: Lazy<Arc<Mutex<ServerInfo>>> =
    Lazy::new(|| Arc::new(Mutex::new(ServerInfo::default())));

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
    // 通过连接外部地址来确定本机 IP
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

#[derive(Deserialize)]
struct PathQuery {
    path: Option<String>,
}

async fn api_list(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<DirListing>, (StatusCode, String)> {
    let root = state
        .root_dir
        .lock()
        .unwrap()
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "未选择共享目录".into()))?;

    let rel = q.path.unwrap_or_default();
    list_dir(&root, &rel).map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn api_download(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Response, (StatusCode, String)> {
    let root = state
        .root_dir
        .lock()
        .unwrap()
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "未选择共享目录".into()))?;

    let rel = q.path.ok_or((StatusCode::BAD_REQUEST, "缺少 path 参数".into()))?;
    let full = root.join(rel.trim_start_matches('/'));
    let full = full
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "文件不存在".into()))?;

    // 安全检查
    let root_canon = root.canonicalize().unwrap();
    if !full.starts_with(&root_canon) {
        return Err((StatusCode::FORBIDDEN, "禁止访问".into()));
    }
    if !full.is_file() {
        return Err((StatusCode::BAD_REQUEST, "不是文件".into()));
    }

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
        server_port: port,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/info", get(api_info))
        .route("/api/list", get(api_list))
        .route("/api/download", get(api_download))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
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
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;

    // 用系统应用打开
    open::that(&dest).map_err(|e| format!("打开失败: {}", e))?;
    Ok(dest.to_string_lossy().to_string())
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
                Duration::from_millis(300),
                reqwest::get(&url),
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
            download_and_open,
            discover_hosts,
            get_my_ip,
            pick_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
