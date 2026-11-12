# Light Translate — 设计与开发准则

## 核心设计铁律

**所有设计必须遵循视觉对齐。**

### 视觉对齐执行细则

1. **纵向轴线对齐（Vertical Axis Alignment）**
   - 侧边栏（顶部品牌区、中部主导航区、底部设置与控制区）的图标中心线、文字起始左边缘、右侧状态指示器/徽标右边缘必须严格共线，严禁不同分组因容器尺寸或内边距差异产生 `2px ~ 8px` 的阶梯错位。
   - 页面内容区的标题、分割线（`Divider`）、卡片边缘与列表首字左边缘/末字右边缘必须共享统一的基准对齐线（如列表在无边框分栏内必须使用 `edge="flush"` 对齐分栏左右边界，按钮文字需对齐边界或做内边距光学补偿）。

2. **横向基线与等高对齐（Horizontal Baseline & Row Parity）**
   - 并列双栏或多栏模块（如概览页「快捷触发」与「近期收录」）的栏目标题行高度必须锁定一致；无论右侧动态操作按钮是否显示（如空状态隐藏按钮时），标题基线与下方分割线（`Divider`）必须 100% 处于同一水平 Y 坐标。
   - 并列列表的每一行必须保持一致的字号层级、行高（`line-height`）与总行高，确保同行主标题、副文案两行文字及行间分割线在左右两栏之间水平贯通对齐。
   - 空状态占位区（`EmptyState`）与相邻非空模块并列时，应保持容器高度与视觉重心平衡。

3. **视图切换零跳动（Zero Layout Shift on Toggle）**
   - 在同一卡片内切换时间跨度或视图模式（如「翻译足迹」在「近一个月」与「近一年」之间切换）时，图表区域必须左右满幅贴合容器边界，且保持稳定的垂直高度，避免引发下方板块上下跳动。

## 本机构建与 macOS 签名铁律（防止 TCC 权限失效）

**严禁使用 ad-hoc 签名（`codesign -`）或无证书的裸 `pnpm tauri build` 产物直接覆盖本机 `~/Applications/Light Translate.app`。**

1. **原因**：ad-hoc 签名会将 macOS `designated requirement` 写死为二进制 `cdhash`，每次重编译 `cdhash` 变化都会导致「辅助功能」与「屏幕录制」TCC 权限失效，迫使用户在系统设置中反复重开。
2. **标准做法**：本地构建与安装**必须**直接运行：
   ```bash
   pnpm install:mac
   ```
   该脚本（`scripts/install-macos.sh`）会自动使用钥匙串固定证书 `Apple Development: Yisu Wang (G2J5UC64SB)`（SHA-1: `ECC226BD94A8F42B309074E4D3E4E1381F2EDCA2`，配合 `--options runtime`）签名并安装到 `~/Applications/Light Translate.app`。
3. **验收标准**：安装完成后可执行 `codesign -dv -r - "$HOME/Applications/Light Translate.app"` 自检，确认 `designated =>` 锚定到证书主体 `subject.CN`，**绝不可出现 `cdhash`**。
