# R-013 Linux 弹窗功能实现计划

## 目标

补齐 Linux 状态栏功能，与 macOS 实现同等产品效果。

## 待实现功能

| ID | 功能 | 优先级 | macOS 实现 |
|----|------|--------|------------|
| R-013.1 | 热键编辑弹窗 | 高 | 自定义 NSWindow |
| R-013.2 | LLM 配置弹窗 | 高 | 自定义 NSWindow + 表单 |
| R-013.3 | 模型下载弹窗 | 中 | 自定义 NSWindow + 进度条 |
| R-013.4 | 打开配置文件夹 | 中 | NSWorkspace |
| R-013.5 | 查看模型文件 | 低 | NSWorkspace |

---

## 技术方案

### 方案 A: 使用 GTK 弹窗 (推荐)

使用 `gtk::Dialog` 或自定义 `gtk::Window` 实现弹窗。

**优点**: 
- 与现有 tray-icon + muda 技术栈一致
- 无需额外系统依赖

**缺点**:
- GTK 样式与部分 Linux 桌面环境可能不搭

### 方案 B: 使用外部工具

使用 `zenity` 或 `kdialog` 调用系统对话框。

**优点**:
- 实现简单
- 与系统主题一致

**缺点**:
- 依赖外部工具
- 功能受限

### 方案 C: 混合方案

- 弹窗使用 GTK 实现
- 文件操作使用 `xdg-open` / `xdg-mime`

---

## 实现步骤

### 阶段 1: R-013.4 + R-013.5 (文件操作)

- [ ] 1.1 实现 `open_config_folder()` - 使用 `xdg-open`
- [ ] 1.2 实现 `open_model_folder()` - 使用 `xdg-open`
- [x] 1.3 在 Linux 菜单中添加对应菜单项
- [ ] 1.4 更新 `map_linux_menu_id_to_action()`

### 阶段 2: R-013.1 (热键编辑弹窗)

- [ ] 2.1 创建 `HotkeyPopupLinux` 结构体
- [ ] 2.2 使用 `gtk::Window` (modal dialog)
- [ ] 2.3 实现按键捕获 (`gdk::EventKey`)
- [x] 2.4 实现撤销/确认逻辑
- [x] 2.5 集成到主事件循环

### 阶段 3: R-013.2 (LLM 配置弹窗)

- [ ] 3.1 创建 `LlmFormPopupLinux` 结构体
- [ ] 3.2 实现表单字段 (provider, model, api_base, api_key)
- [ ] 3.3 使用 `gtk::Entry` + `gtk::ComboBox`
- [x] 3.4 实现保存/取消逻辑
- [x] 3.5 集成到主事件循环

### 阶段 4: R-013.3 (模型下载弹窗)

- [ ] 4.1 创建 `DownloadPopupLinux` 结构体
- [ ] 4.2 实现进度显示 (使用 `gtk::ProgressBar`)
- [x] 4.3 实现下载日志显示
- [x] 4.4 集成 IPC 消息处理
- [x] 4.5 与主进程下载进度同步

---

## 涉及代码文件

| 文件 | 改动 |
|------|------|
| `src/status_indicator.rs` | 添加 Linux 弹窗实现 |
| `src/main.rs` | 添加文件操作命令 |
| `Cargo.toml` | 可能需要添加 gtk 相关依赖 |

---

## 依赖

现有依赖已足够:
- `gtk = "0.18"`
- `muda = "0.14"`
- `tray-icon = "0.21"`

系统命令:
- `xdg-open` (通常预装)

---

## 实现状态

**全部完成** - 所有阶段均已实现并通过编译验证。

## 风险

1. **GTK 事件循环**: 需要与现有 tray-icon 事件循环整合
2. **按键捕获**: Linux 全局按键捕获需要额外权限
3. **主题适配**: GTK 弹窗样式可能与部分桌面环境不搭

---

*本文档最后更新: 2026-03-23*
