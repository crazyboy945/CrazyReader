# CrazyReader

一个用 Rust 写的桌面电子书阅读器，基于 [eframe / egui](https://github.com/emilk/egui)。
支持 epub、markdown、pdf、txt 四种格式，以连续流式滚动的形式阅读，并带目录跳转、
文字标记、摘录、笔记和阅读位置记忆。

## 功能

- **多格式**：epub（含内嵌图片与目录）、Markdown（含本地图片、代码块语法高亮）、
  PDF（hayro 渲染，按页分页）、txt（UTF-8 自动回退到 GBK）。
- **连续流式阅读**：整本书按视图宽度排版成一个长滚动区，单页或双页两种栏式，
  可以一直滚到书末而不受章节边界限制。
- **目录跳转**：左侧栏同时提供文件系统树和当前文档的章节目录，点击目录跳到对应标题。
  文件树按类型绘制图标（文件夹 / epub / Markdown / PDF / 文本）。
- **多页签**：可同时打开多本书，页签上有各自的关闭按钮。
- **标记 / 摘录 / 笔记**：在阅读区选中文字后弹出单行悬浮工具窗（只有按钮和关闭），
  提供标记（给选中文字加背景色）、摘录（存入原文）、笔记（带引用的笔记）、复制四个动作；
  右侧笔记面板集中列出笔记、摘录和标记，可逐条删除或跳回原文位置。
- **外观**：浅色 / 深色主题，阅读字体可选（扫描系统中文字体），
  背景有跟随主题、米白、纯白、羊皮纸、淡青、豆沙绿、浅灰、深灰、夜黑等预设，
  字号可调。
- **Markdown 自定义样式**：可以指定一个 CSS 文件覆盖默认排版，改完自动热重载，
  详见 [Markdown 自定义样式](#markdown-自定义样式)。
- **进度记忆**：每本书的阅读位置、笔记、摘录、标记都会保存，字号、主题、背景等
  偏好设置一并记录，下次启动自动恢复到上次的书和位置。

## 界面

| 区域 | 内容 |
| --- | --- |
| 顶栏 | 打开文件、打开目录、侧栏 / 笔记 / 工具窗开关、书页签（类型图标 + 关闭按钮） |
| 左侧栏 | 文件夹树（标题即当前目录路径，右侧「刷新」）+ 当前文档章节目录 |
| 阅读区 | 连续滚动正文，选中文字后出现单行悬浮工具窗（标记 / 摘录 / 笔记 / 复制，右侧关闭） |
| 底栏 | 进度、字号 / 缩放、单双页与主题、标记、字体 / 背景 / 样式分组，最右侧为状态提示 |
| 右侧面板 | 笔记输入框、笔记列表、摘录列表、标记列表（可拖动宽度） |

面板标题带一条主题色竖条并加粗，当前正在阅读的目录项以主题色底纹标出；
顶栏和底栏的按钮都带矢量图标，当前文档的页签用与阅读区一致的纸色和顶部主题色条标出。

## Markdown 自定义样式

阅读 Markdown 时，底栏「样式」下拉框可以选择一个 CSS 文件覆盖默认排版：

```
样式 → 选择 CSS 文件…        # 应用样式表
样式 → 重新载入              # 手动重载（保存文件也会自动重载）
样式 → 无（内置样式）        # 取消
```

仓库里的 [`assets/reader-example.css`](assets/reader-example.css) 是一份可直接套用的示例。
样式表路径记在 `library.json` 的 `settings.css` 中，重启后继续生效；阅读器每 1.5 秒
检查一次文件修改时间，编辑保存后正文会立刻重排，无需重启。

支持的元素选择器：`body`、`html`、`p`、`div`、`h1`~`h6`、`li`、`ul`、`ol`、`a`、
`code`、`pre`、`blockquote`、`hr`、`img`、`strong`、`em`；`body` 上的字号、行距、
颜色与字体会向下继承。支持的属性：

| 属性 | 说明 |
| --- | --- |
| `font-size` | `px` / `em` / `rem` / `%` / `pt`，`em` 相对父级字号 |
| `font-family` | 按名称匹配系统已安装字体（微软雅黑、宋体、黑体、楷体、仿宋、Noto CJK 等） |
| `color`、`background` | `#rgb`、`#rrggbb`、`#rrggbbaa`、`rgb()`、`rgba()` 与约 40 个 CSS 颜色名 |
| `font-weight`、`font-style`、`text-decoration` | 粗体、斜体、下划线 / 删除线 |
| `line-height` | 无单位数值按倍数处理，也可用 `px` |
| `margin`、`margin-top`、`margin-bottom` | 块前 / 块后间距 |
| `padding` | 四值简写，给块加内边距（配合 `background`、`border-left` 使用） |
| `border-left` | `6px solid #1565c0` 形式的左侧竖条 |
| `text-align`、`text-indent` | 对齐；首行缩进按全角空格计算，`2em` 即缩进两字 |
| `max-width` | 仅对 `img` 生效，限制图片宽度 |

class、id 和后代选择器会被解析后忽略，`@media` 之类的规则会被跳过；无法识别的声明
会显示在「样式」下拉框底部的红色提示里，不会中断整份样式表。

## 快捷键

| 按键 | 作用 |
| --- | --- |
| `Ctrl+O` | 打开文件（可多选） |
| `Ctrl+D` | 打开目录 |
| `Ctrl+M` | 标记当前选中的文字 |
| `PageDown` / `Space` / `→` | 向下翻一屏 |
| `PageUp` / `←` | 向上翻一屏 |

## 构建

需要较新的 Rust 工具链（crate 使用 edition 2024）。Windows 上 `build.rs` 会调用
`windres` 编译应用图标资源，因此需要 MinGW-w64 binutils 在 `PATH` 中；
非 Windows 平台会跳过这一步。

```sh
cargo build --release        # 产物在 target/release/crazy-reader.exe
cargo run --release
```

## 数据存储

所有状态集中在 `%APPDATA%\CrazyReader\library.json`（非 Windows 下为 `%USERPROFILE%\CrazyReader\library.json`），
包含两部分：

- `books` —— 每本书的阅读位置、笔记、摘录、标记
- `settings` —— 字号、主题、单双页、背景、阅读字体、Markdown 样式表路径、
  上次浏览的目录与展开的目录树分支

设置 `APPDATA` 环境变量可以把这个文件重定向到别的位置，便于并行测试。

## 目录结构

```
src/
  main.rs        应用入口与全部界面逻辑
  model.rs       文档模型（Block / Inline / Document / Note）
  css.rs         Markdown 自定义样式表解析（选择器、颜色、长度、继承）
  style.rs       主题、背景预设、阅读配色、强调色、字体装载
  storage.rs     library.json 读写（书籍进度、笔记、设置）
  hilite.rs      代码块语法高亮
  loaders/       epub / markdown / pdf / txt 解析与分页
assets/          应用图标、Windows 资源脚本、示例样式表 reader-example.css
```

## 已知限制

- 主要面向 Windows 开发与测试，其他平台未经验证。
- PDF 按整页渲染后拼接，重排文字、跨页选中和 PDF 内目录跳转尚不支持。
- 标记以纯文本片段匹配，同一段落内选中；重复出现的句子会被一并高亮。
