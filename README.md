# Light Translate 2.1

基于 **Rust + Tauri v2 + React 19 (TypeScript) + Meta Astryx** 的桌面轻量取词与即时翻译工具。**macOS 与 Windows 是主要目标平台**；Linux（仅 X11）提供主窗口词典、手动翻译与生词本的有限兼容支持（差异说明见文末）。

---

## 下载与安装（macOS / Windows / Linux）

所有正式版安装包均在 **[GitHub Releases 最新版本页](https://github.com/IndiumAndy/Light-Translate-v2/releases/latest)** 发布。每个安装包文件名均以 **`LightTranslate-macOS-*` / `LightTranslate-Windows-*` / `LightTranslate-Linux-*`** 明确标注所属系统，请按当前系统下载对应文件，切勿混用：

| 操作系统 | 适用设备 / 发行版 | 直接下载入口（点击即下最新版） | 格式说明 |
| :--- | :--- | :--- | :--- |
| **macOS** *(14.0 Sonoma 及以上)* | Apple Silicon (M1–M4) 与 Intel 双架构通用 | **[下载 LightTranslate-macOS-Universal.dmg](https://github.com/IndiumAndy/Light-Translate-v2/releases/latest/download/LightTranslate-macOS-Universal.dmg)** | `.dmg` 磁盘映像（唯一 macOS 安装包，无需区分芯片） |
| **Windows** *(Win 10 / 11 64位)* | 个人桌面标准安装（**推荐**） | **[下载 LightTranslate-Windows-x64-setup.exe](https://github.com/IndiumAndy/Light-Translate-v2/releases/latest/download/LightTranslate-Windows-x64-setup.exe)** | `.exe` 中文安装向导（支持当前用户免管理员安装，自动检测 WebView2） |
| **Windows** *(Win 10 / 11 64位)* | MSI 软件包 / 批量部署 | **[下载 LightTranslate-Windows-x64.msi](https://github.com/IndiumAndy/Light-Translate-v2/releases/latest/download/LightTranslate-Windows-x64.msi)** | `.msi` 标准 Windows Installer 安装包 |
| **Linux (X11)** *(x86_64)* | Ubuntu / Debian / Linux Mint 等 | **[下载 LightTranslate-Linux-x86_64.deb](https://github.com/IndiumAndy/Light-Translate-v2/releases/latest/download/LightTranslate-Linux-x86_64.deb)** | `.deb` 软件包（自动解析系统依赖） |
| **Linux (X11)** *(x86_64)* | 跨发行版免安装通用版 | **[下载 LightTranslate-Linux-x86_64.AppImage](https://github.com/IndiumAndy/Light-Translate-v2/releases/latest/download/LightTranslate-Linux-x86_64.AppImage)** | `.AppImage` 单文件便携包，赋权即可运行 |
| **Linux (X11)** *(x86_64)* | Fedora / RHEL / openSUSE 等 | **[下载 LightTranslate-Linux-x86_64.rpm](https://github.com/IndiumAndy/Light-Translate-v2/releases/latest/download/LightTranslate-Linux-x86_64.rpm)** | `.rpm` 软件包 |

> 完整历史版本与 `SHA256SUMS.txt` 校验文件入口：<https://github.com/IndiumAndy/Light-Translate-v2/releases>

### 1. macOS 安装步骤
1. 下载 **`LightTranslate-macOS-Universal.dmg`**（请勿下载 `.exe` / `.msi` / `.deb`）。
2. 双击打开 `.dmg`，将 **Light Translate.app** 拖入右侧 **Applications（应用程序）** 文件夹。
3. **首次打开提示处理**：由于无焦点取词浮窗使用了 macOS 原生面板私有能力（未上架 App Store），首次打开若提示“无法验证开发者”或“已损坏”，在终端执行以下命令即可放行：
   ```bash
   xattr -cr "/Applications/Light Translate.app"
   ```
4. 启动后在应用「设置 → 权限与忽略」中按引导开启 **辅助功能**（屏幕取词）与 **屏幕录制**（区域框选 OCR）权限。

### 2. Windows 安装步骤
1. 推荐下载 **`LightTranslate-Windows-x64-setup.exe`**（若需企业 MSI 部署可下载 **`LightTranslate-Windows-x64.msi`**）。
2. 双击运行安装程序；若弹出 Windows SmartScreen 提示，点击 **「更多信息」→「仍要运行」**，按简体中文向导完成安装。
3. 启动后在「设置 → 翻译引擎」填入 API Key（如 Gemini / DeepSeek / OpenAI 兼容接口）即可直接使用悬停取词、划词翻译与截屏 OCR。

### 3. Linux (X11) 安装步骤
请确保桌面会话处于 **X11**（暂不支持纯 Wayland 全局特性），按发行版三选一安装：

- **Ubuntu / Debian 系（`.deb`）**：
  ```bash
  sudo apt install ./LightTranslate-Linux-x86_64.deb
  ```
- **通用免安装版（`.AppImage`）**：
  ```bash
  chmod +x ./LightTranslate-Linux-x86_64.AppImage
  ./LightTranslate-Linux-x86_64.AppImage
  ```
- **Fedora / RHEL / openSUSE 系（`.rpm`）**：
  ```bash
  sudo dnf install ./LightTranslate-Linux-x86_64.rpm
  ```

---

## 核心架构亮点

1. **三级混合取词管线（高命中率）**
   - **自动唤醒 Chromium / Electron 无障碍树**：首次命中 Chrome、Arc、Edge、Brave、VS Code、Notion 等应用时自动注入 `AXManualAccessibility = true`（Windows 侧通过 `IUIAutomation::ElementFromPoint` 激活控件树）。
   - **词边缘容差与 4 层有界子树几何命中**：兼容带标点/空格范围与 `±1` 字符边缘容差，支持 4 层 24 节点内按矩形精准定位按钮、标签栏或嵌套内联文本。
   - **零冷却本地离线 OCR 兜底**：PDF、Canvas、图片区域无缝切入系统级离线 OCR（macOS Apple `Vision` 开启语言纠错；Windows `Windows.Media.Ocr`），随查词同步提取所在整句作为多义词消歧 `context`。
2. **跨行整句智能拼合**
   - **无障碍跨节点段落聚合**：内联节点被 `<a>`、`<code>`、`<span>` 拆碎时自动回溯 1~2 层段落父容器拼合完整句子，缩写与小数安全分句（保留 `Dr.`、`3.14`、`example.com`）。
   - **OCR 列内投影聚类**：先按列分割再在目标列内部做 `1.85×` 行高与首行缩进/短尾行对齐扩展，避免侧边栏和行号串行。
3. **顺手无焦点的轻量交互**
   - **松键不断连**：预热常驻无焦点浮窗（macOS `NSPanel` `.nonactivatingPanel` / Windows `WS_EX_NOACTIVATE`），松开修饰键仅由穿透态转为可交互驻留态，后台 SSE 流式翻译绝不中断。
   - **浮窗一键 `[译整句]`**、**选区原地浮窗翻译**（支持双击 `Option/Alt` 或快捷键）以及 **框选截图原位覆盖翻译**。默认快捷键为 macOS `⌥⌘T` / `⌃⌘S`、Windows `Alt+Shift+T` / `Alt+Shift+S`，支持在设置页自定义录制。
4. **百毫秒级并发响应与清透棉白词典界面**
   - `< 5ms` Rust 内存离线词典（4.3 万词条 + 词形还原）+ HTTP/2 连接池 SSE 流式大模型（内置 Gemini、DeepSeek 与任意 OpenAI 兼容端点）。
   - 基于 Meta Astryx 设计系统构建主工作台：支持原生可折叠侧栏、26/52 周阅读足迹热力图、实时词典+流式双语工作台及生词本导入导出。

---

## 本地开发与构建

```bash
pnpm install
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri dev

# 构建当前系统桌面安装包
pnpm tauri build
```

Rust 版本由仓库根目录的 `rust-toolchain.toml` 固定为 **1.90.0**。本机装有 `rustup` 时执行 `cargo` 会自动匹配该工具链版本。

### 本机构建缓存（可选）

默认编译产物位于 `src-tauri/target`（已在 `.gitignore` 中忽略）。若项目位于开启了 iCloud 同步的目录中，可在当前终端会话中将 Rust 编译缓存重定向到本地缓存目录，避免同步附加扩展属性影响 macOS 代码签名：

```bash
export CARGO_TARGET_DIR="$HOME/Library/Caches/LightTranslateV2/target"
```

---

## macOS 私有 API 与分发说明

不抢焦点的取词浮窗依赖两处 macOS 私有行为：

- `tauri.conf.json` 的 `macOSPrivateApi`
- `src-tauri/native/macos_bridge.m` 中的 `LTNonActivatingPanel` 与 `_preventsActivation`

因此本应用通过 GitHub Releases 直接分发 `.dmg`，无法上架 Mac App Store（移除上述能力会导致取词浮窗抢占当前阅读窗口焦点）。

---

## 平台支持说明

### macOS 与 Windows（完整支持）

悬停取词、系统离线 OCR、全局快捷键、剪贴板读写与无焦点浮窗均具备各自平台的原生实现。默认全局快捷键（可在「设置 → 取词与快捷键」中自由改绑或解绑）：

- **划词翻译**：macOS `⌥⌘T`（`Alt+Command+T`），Windows `Alt+Shift+T`
- **区域截屏**：macOS `⌃⌘S`（`Control+Command+S`），Windows `Alt+Shift+S`

### Linux（仅 X11 有限兼容）

主窗口内的手动翻译、离线词典查询、阅读统计、设置与生词本导入导出均可正常编译与使用；但 `src-tauri/src/platform/linux.rs` 暂未实现以下桌面级全局能力：

- 无全局键鼠钩子（悬停取词、双击修饰键与全局快捷键不会触发）。
- 无无障碍树取词与本地离线 OCR（仅支持读取已有 X11 PRIMARY 选区文本）。
- 剪贴板写入返回 `NotSupported`，浮窗为普通窗口（无点击穿透与免激活特性），且不支持 Wayland。
