# Light Translate 2.0

基于 **Rust + Tauri v2 + React 19 (TypeScript)** 重构的跨平台桌面轻量级取词与即时翻译工具，主力覆盖 **macOS 与 Windows**（100% 体验对等），并提供 **Linux (X11)** 兼容层。

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
   - **浮窗一键 `[译整句]`**、**选区原地浮窗翻译**（支持双击 `Option/Alt` 或快捷键，无需鼠标停在选区内）以及 **框选截图翻译**（macOS `⌃⌘S` / Windows `Alt+Shift+S`）。
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
