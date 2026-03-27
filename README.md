# LocalShare — 局域网文件直连工具

一个 Tauri 桌面应用，让局域网内的电脑可以**直接用本地 Word / WPS / Excel 等应用打开对方的文件**，无需 U 盘。

---

## 功能

| 功能 | 说明 |
|---|---|
| 🖥 主机模式 | 选择文件夹，启动共享服务 |
| 💻 客户端模式 | 自动扫描局域网发现主机，也可手动输入 IP |
| ▶ 一键打开 | 点击文件 → 自动下载到临时目录 → 用系统默认应用打开 |
| ⬇ 下载 | 保存文件到本地 |
| 📂 目录浏览 | 支持多级目录浏览 |

---

## 开发环境安装

### 前置要求

1. **Rust**（stable）
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Node.js**（>= 18）  
   https://nodejs.org

3. **Tauri CLI**
   ```bash
   npm install
   ```

4. **系统依赖（Linux）**
   ```bash
   sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
   ```

5. **`hostname` crate 依赖**  
   在 `src-tauri/Cargo.toml` 中已包含，Cargo 会自动下载。

---

## 运行开发版

```bash
npm run dev
```

---

## 构建安装包

```bash
npm run build
```

构建产物在 `src-tauri/target/release/bundle/` 下，Windows 生成 `.msi` / `.exe`，macOS 生成 `.dmg`。

---

## 使用方法

### 主机端（有文件的电脑）

1. 打开 LocalShare → 选择「🖥 主机模式」
2. 点击「📂 选择」，选择要共享的文件夹
3. 点击「🚀 启动共享服务」
4. 将显示的局域网地址（如 `http://192.168.1.5:8899`）发给同事

### 客户端（要访问文件的电脑）

1. 打开 LocalShare → 选择「💻 客户端」
2. 等待自动扫描局域网，或手动输入主机 IP
3. 点击主机卡片连接
4. 浏览文件，点击「▶ 打开」即可用本地 Word / WPS 打开文件

---

## 技术架构

```
localshare/
├── src/
│   └── index.html          # 前端 UI（原生 HTML/CSS/JS）
└── src-tauri/
    ├── Cargo.toml           # Rust 依赖
    ├── tauri.conf.json      # Tauri 配置
    └── src/
        ├── main.rs          # 入口
        └── lib.rs           # 核心逻辑
            ├── Axum HTTP 服务器（主机端）
            ├── 局域网扫描（客户端）
            ├── download_and_open()  ← 下载 + 系统应用打开
            └── Tauri commands
```

### 核心流程：点击「打开」后发生了什么？

```
客户端点击「▶ 打开」
  → 前端调用 invoke('download_and_open', { url, fileName })
  → Rust: reqwest 下载文件到 %TEMP%/localshare_open/
  → Rust: open::that(path) 调用系统默认关联程序
  → Word / WPS / Excel 打开文件
```

---

## 常见问题

**Q: 对方能修改我的文件吗？**  
A: 不能，服务器只提供只读下载。文件在对方本地临时目录打开，修改后需手动发回。

**Q: 支持多个客户端同时连接吗？**  
A: 支持，Axum 是异步服务器，可并发处理。

**Q: 为什么扫描不到主机？**  
A: 检查防火墙是否放行端口（默认 8899），Windows 需允许"专用网络"访问。

**Q: 文件打开后是临时文件，保存了去哪里？**  
A: 保存在 `%TEMP%\localshare_open\` 目录，需手动复制到目标位置。建议修改完用「下载」功能保存到正确位置。
