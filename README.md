# Light Translate 2.0

基于 **Rust + Tauri v2 + React 19 (TypeScript)** 的桌面轻量取词与即时翻译工具。**macOS 与 Windows 是主要目标。** Linux（仅 X11）是有限兼容层，不是同等支持的平台，缺口见文末。

## 核心架构升级亮点

1. **三级混合取词管线（高命中率）**
   - **自动唤醒 Chromium / Electron 无障碍树**：首次命中 Chrome、Arc、Edge、Brave、VS Code、Notion 等应用时自动注入 `AXManualAccessibility = true`（Windows 侧通过 `IUIAutomation::ElementFromPoint` 激活控件树）。
   - **词边缘容差与 4 层有界子树几何命中**：兼容带标点/空格范围与 `±1` 字符边缘容差，支持 4 层 24 节点内按矩形精准定位按钮、标签栏或嵌套内联文本。
   - **零冷却本地离线 OCR 兜底**：PDF、Canvas、图片区域无缝切入系统级离线 OCR（macOS Apple `Vision` 开启语言纠错；Windows `Windows.Media.Ocr`），随查词同步提取所在整句作为多义词消歧 `context`。
2. **跨行整句智能拼合**
   - **无障碍跨节点段落聚合**：内联节点被 `<a>`、`<code>`、`<span>` 拆碎时自动回溯 1~2 层段落父容器拼合完整句子，缩写与小数安全分句（保留 `Dr.`、`3.14`、`example.com`）。
   - **OCR 列内投影聚类**：废除全局同行连坐限制，先按列分割再在目标列内部做 `1.85×` 行高与首行缩进/短尾行对齐扩展，彻底免疫侧边栏和行号干扰。
3. **顺手无焦点的轻量交互**
   - **松键不断连**：预热常驻无焦点浮窗（macOS `NSPanel` `.nonactivatingPanel` / Windows `WS_EX_NOACTIVATE`），松开修饰键仅由穿透态转为可交互驻留态，后台 SSE 流式翻译绝不中断。
   - **浮窗一键 `[译整句]`**、**选区原地浮窗翻译**（支持双击 `Option/Alt` 或快捷键，无需鼠标停在选区内）以及 **框选截图翻译**。默认快捷键是 macOS `⌥⌘T` / `⌃⌘S`、Windows `Alt+Shift+T` / `Alt+Shift+S`，可在设置里改。Linux 不响应这些全局快捷键。
4. **百毫秒级三级并发响应**
   - `< 5ms` Rust 内存离线词典（4.3 万词条 + 词形还原）→ `80~250ms` 极速机器直译预热 → `200~400ms` 首字（TTFT）HTTP/2 连接池 + SSE 流式大模型（内置 Gemini、DeepSeek 与任意 OpenAI 兼容端点）。

## 本地开发与构建

```bash
pnpm install
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri dev
```

Rust 版本由仓库根目录的 `rust-toolchain.toml` 固定为 **1.90.0**。当前依赖里的 Tauri 2.12 要求 rustc 1.90 或更高。本机装了 rustup 时，在仓库目录里执行 cargo 会自动用这个版本。

### 本机构建缓存（可选）

`.cargo/config.toml` 不设置 `target-dir`。以前写死的 `/Users/atat/Library/Caches/LightTranslateV2/target` 在 GitHub 的 Ubuntu runner 上不存在，Rust 测试会马上失败。

不设置时，编译产物在 `src-tauri/target`（已在 `.gitignore` 中）。

如果仓库放在会同步的 macOS 目录里，希望把产物留在本机缓存，避开 iCloud 同步以及 Finder 给文件加上的信息导致代码签名出问题，只在自己的 shell 里导出：

```bash
export CARGO_TARGET_DIR="$HOME/Library/Caches/LightTranslateV2/target"
```

这是环境变量，不会进仓库，CI 和其他机器仍使用默认的 `src-tauri/target`。

## macOS 私钥有 API 与分发

不抢焦点的浮窗依赖两处私有行为，不要删：

- `tauri.conf.json` 的 `macOSPrivateApi`
- `src-tauri/native/macos_bridge.m` 里的 `LTNonActivatingPanel`，以及装到窗口类上的 `_preventsActivation`

后果：

- **不能上 Mac App Store。** 审核不接受私有 API。
- **Developer ID 公证不保证通过。** 公证扫描可能标出 `_preventsActivation`。用当前打包产物跑一次 `notarytool` 才能知道结果。去掉这两处不能换来上架，浮窗会抢走当前应用的焦点。

## 平台支持

### macOS 与 Windows

悬停取词、系统离线 OCR、全局快捷键、复制到剪贴板、不抢焦点的浮窗都有各自的系统实现。这两边是实际要用的平台。macOS 浮窗依赖上面的私有 API。

默认快捷键（设置页可以改；改完会写到原生监听里）：

- 划词翻译：macOS `⌥⌘T`（`Alt+Command+T`），Windows `Alt+Shift+T`
- 框选 OCR：macOS `⌃⌘S`（`Control+Command+S`），Windows `Alt+Shift+S`

### Linux（X11）做不到的事

可以编译。主窗口里的手动翻译、离线词典、设置和生词本走的是共用 Rust 代码，不依赖下面这些平台接口。下列行为在 `src-tauri/src/platform/linux.rs` 里没有实现：

- **没有全局键鼠监听。** 悬停取词、双击修饰键、Escape、设置里的全局快捷键都不会触发。`update_trigger_config` 把参数直接丢掉。
- **没有无障碍取词。** 只有 X11 PRIMARY 选区里已经有文字时才能读到那段选区。
- **没有 OCR。** 悬停 OCR 直接返回不支持。框选会读一块 X11 像素，然后仍然返回不支持，不会识别文字。
- **不能写入剪贴板。** 复制会返回 `NotSupported`，不会假装已经写入。
- **浮窗不是不抢焦点的面板**，也没有点击穿透。显示时只是移动、改大小并 `show`。
- **读不到光标下或焦点应用的标识**，按应用排除基本不起作用。
- **权限状态不是真的。** 有 `DISPLAY` 就把辅助功能和屏幕录制都报成已授权。
- **不支持 Wayland。** 只用 X11（`x11rb`）。
