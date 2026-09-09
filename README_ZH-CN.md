# Wizard101 对话翻译助手 (Wizard101 Chat Translator)

[English](README.md) | [繁體中文](README_ZH-TW.md) | 简体中文

[![ci](https://github.com/GoneTone/wizard101-chat-translator/actions/workflows/ci.yml/badge.svg)](https://github.com/GoneTone/wizard101-chat-translator/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/GoneTone/wizard101-chat-translator)](https://github.com/GoneTone/wizard101-chat-translator/releases/latest)

Wizard101 的聊天 AI 翻译软件——叠在游戏上，双向实时翻译。

**看消息**：聊天对话出现的每一句都翻成你设置的语言、显示在叠加窗口里，译文上方保留原文，你永远看得到对方实际说了什么。

**要发言**：在游戏里打开聊天输入框，翻译输入框就会自动出现。用任何语言打字、按 Enter，英文译文就会被逐字键入游戏聊天输入框（游戏不支持直接粘贴）。翻译内容**不会自动发送**，让你可以确认，然后按 Enter 发送。

聊天消息要翻成哪个语言由你决定，设置里直接填语言名称（`繁體中文（台灣）`、`日本語`、`Español`…）；对方用什么语言则由 AI 自动判断。

聊天内容是直接从游戏的聊天窗口读出来，不是从画面识别：不会看错字、不会乱序，他人与自己的发言、谁说的都涵盖，也不碰任何网络封包。

## ⚠ 重要

本软件会挂入（注入）游戏进程读取对话内容，此行为未经 KingsIsle 认可，可能被认定违反 [Wizard101 服务条款](https://www.wizard101.com/game/termsofuse)；依该条款，KingsIsle 可基于任何理由（或无需理由）停权，**请自行斟酌**。

## 下载

下载 `Wizard101ChatTranslator.exe`，放到任意文件夹即可。配置文件 `config.json` 与日志文件 `app.log`、`messages.log` 都会存在 exe 同一个文件夹。

<https://github.com/GoneTone/wizard101-chat-translator/releases/latest>

**更新版本**：请先关闭本软件——点叠加窗口标题栏右上角的 ✕——再换文件。程序运行中，exe 会被 Windows 锁住、也还挂在游戏上。关掉后下载新版 `Wizard101ChatTranslator.exe`，覆盖同一个文件夹里的旧文件即可，原本的 `config.json` 会照常沿用，服务商、密钥等设置都不必重填。

**杀毒软件误判**：本软件会读取游戏聊天内存（挂入游戏进程）并监听全局热键，行为模式与部分恶意程序相似，可能被杀毒软件标记或直接删除／隔离，这是误判，请自行评估风险后把程序加入杀毒软件白名单。

**管理员权限**：一般情况（游戏以普通权限启动）不需要管理员权限。只有当 Wizard101 本身以**管理员**身份运行时，本软件也必须以管理员身份运行（挂入游戏进程需要对等或更高权限）。权限不足时叠加窗口会显示「⚠  权限不足」横幅，关掉本软件、右键选「以管理员身份运行」再开一次即可。

## 使用方式

1. 双击运行 exe：
   - **第一次运行**（找不到 `config.json`）会弹出**首次设置向导**：第一步选界面语言，第二步选翻译服务、填密钥并测试连接，第三步选目标语言与热键。保存后自动进入主流程。
   - 之后每次启动都直接进主流程，不会再弹向导；界面语言仍可随时在设置窗口（齿轮 ⚙）改，保存即立即生效，不必重开程序。
2. 打开 Wizard101 并**登录进游戏世界内**，叠加窗口才读得到聊天消息。
3. 聊天出现消息 → 叠加窗口显示**原文 + 目标语言译文**（最新在最下，可向上滚动看历史；默认不会依时间淡出，但最多保留 200 条，超过会移除最旧）。
   - **可移动、可缩放**：拖动顶端标题栏移动、拖任一边或任一角缩放（右下角另有把手；标题栏上的 ⚙／─／✕ 按钮本身仍是按钮，不触发缩放）。位置与大小自动保存。
   - **可选中复制**：在消息上按住左键拖动选中文本，可跨多条消息；拖到消息区的上下边缘之外会自动滚动，能选到画面外的内容。再按 `Ctrl+C` 或在选中处按右键选「复制」即可，复制出来的文本**消息与消息之间以空行分隔**。
   - 注意：窗口不鼠标穿透，它盖住的那块区域点击不会传到游戏。
4. 想发言：在游戏里打开聊天输入框 → 翻译输入框自动出现 → 输入消息（任何语言）→ Enter → 软件切回游戏、把翻好的英文**逐字自动键入**聊天输入框 → 自己检查后按 Enter 发送。
   - 若把翻译输入框关掉了，或在设置内关闭了自动呼出（`auto_show_input`），可以按 `Ctrl+Space`（默认）叫它出来。热键只在游戏窗口位于前台时生效，切到其他程序不会误触。
   - 译文自动键入进游戏时丢字／太快 → 打开设置窗口（⚙），在「高级」标签页调大「键入延迟（秒）」。
   - 打字期间光标要停在游戏聊天输入框，别去点别的窗口。
5. 想改设置（服务商、密钥、目标语言、热键…）时，点叠加窗口标题栏的齿轮（⚙）打开设置窗口，保存即生效，不必重开程序。
6. 要结束程序时，点叠加窗口标题栏右上角的 ✕（会先解除游戏挂入再退出）。
7. 每次启动会自动检查 GitHub 上有没有新版本，有的话叠加窗口会多出一条蓝色横幅（点横幅开下载页，点右侧 ✕ 关掉这次提醒）；也可以在设置窗口的「关于」标签页手动检查更新、查看项目链接与日志文件所在文件夹。

## 功能与特色

- **看得懂别人在说什么**：聊天每一句都实时翻成你的语言，原文保留在译文上方；消息一次涌进来也跟得上
- **用自己的语言发言**：打开游戏聊天输入框，翻译输入框就自动出现；打完按 Enter，英文译文自动键入聊天框，要不要发送由你决定
- **不会看错字、不会漏消息**：直接读游戏的聊天内容，不是识别画面，他人与自己的发言都涵盖
- **要翻成什么语言你自己填**：直接填语言名称（`繁體中文（台灣）`、`日本語`、`Español`…），对方用什么语言则由 AI 自动判断
- **选你要用的 AI**：OpenAI（ChatGPT）、Anthropic（Claude），或自己搭的 OpenAI 兼容服务；每家各存一份设置，换来换去不必重填密钥
- **系统消息想不想看都行**：掉落、经验、升级广播之类的默认不翻，需要时在设置开启
- **窗口摆顺手就不用再管**：拖动移动、拉边角缩放、调低不透明度、不看时缩成小气泡，位置与大小都记住，下次启动还在原地
- **想留下的内容可以复制**：在消息上拖动选中文本，可跨多条消息，`Ctrl+C` 或右键复制
- **设置随时能改**：首次运行有向导带你设置；之后点齿轮（⚙）改，保存立刻生效，不必重开程序
- **被强制结束或崩溃也能接着用**：正常结束会自己解除挂入；万一被强制结束，下次启动会先清掉上次的残留再重新接上，不必重开游戏
- **界面多语言、自动检查更新**：首次运行依 Windows 系统语言自动判定；有新版会在叠加窗口上提醒你

## 疑难排解

- **显示「权限不足」横幅**：游戏以管理员身份运行，软件挂不进去。关掉软件，改以管理员身份启动（从源码运行时，改以管理员身份打开终端）。
- **显示「游戏版本不兼容」横幅**：本软件靠固定的内存特征（pattern）找到聊天控件，游戏更新可能让这组特征失效。先把**游戏与本软件**都更新到最新版；若两者都已是最新仍出现这条横幅，代表本软件尚未跟上这个游戏版本，只能等新版发布（可在设置窗口的「关于」标签页检查更新）。这种情况下不会有任何东西被写进游戏，放着不管也不会有副作用。
- **状态停在「连接游戏中…」或显示「游戏未就绪／连接中断」**：确认游戏已启动并登录进游戏世界内。
- **自动键入译文时游戏丢字**：在设置窗口（⚙）的「高级」标签页调大「键入延迟（秒）」，并确认打字期间光标停在游戏聊天输入框。

## 截图

TODO

## 已知限制

- 游戏聊天白名单：非白名单英文词可能被游戏自动过滤，任何翻译软件都绕不过。
- 译文不会瞬间出现：默认每 0.4 秒扫一次聊天，再等翻译服务响应才显示，慢多少主要看你选的服务商与模型。
- 玩家对话不缓存，每一条都会实际送一次翻译（也就是每条都算 API 费用）。原因有二：玩家讲的话几乎不会一字不差地重复，缓存几乎不会命中；而且每条翻译都会带上前几条对话当上下文，同一句话在不同语境下本来就该翻得不一样，硬套旧译文反而会翻错。只有句型固定的系统消息才缓存。
- 依赖固定的内存特征（pattern）：游戏更新后 pattern 可能失效（挂入时报 `PatternFailed`，或 hook 挂上了却不被触发），需等支持跟进才会恢复；从源码运行时，是等收信用的依赖包更新后重跑 `uv sync`。两种情形叠加窗口都显示「⚠  游戏版本不兼容」横幅，与「游戏没开」分得开。
- 挂入游戏与全局热键（keyboard 包）只在游戏以管理员身份运行时需要同样提权（完整性等级要对等），一般情况不必；权限不足时横幅会直接指出解法。

## 开发

以下为源码开发、调试、打包用的流程。

### 前置需求

- Windows
- [uv](https://docs.astral.sh/uv/) —— Python 版本由 `.python-version` 锁定，`uv sync` 会自动下载对应版本，本机与 CI 一致

### 安装

1. `uv sync` —— 创建 `.venv` 并依 `pyproject.toml` 装好所有依赖（含收信用的 wizwalker；已在 `[tool.uv.sources]` 指向有跟进最新 client pattern 的 [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker)，官方 PyPI 版 pattern 过旧、对不上现行 client）。
2. `uv run run.py` —— 第一次运行会跑首次设置向导（选服务商、填密钥或自建服务器网址、测试连接），设置写进项目根目录的 `config.json`（不进版本控制）。

> 游戏更新导致挂入报 `PatternFailed` 时，更新 fork 后重跑 `uv sync`（或 `uv lock --upgrade-package wizwalker`）取得新 pattern。

### 从源码运行

1. 打开并登录 Wizard101 到游戏世界内。
2. `uv run run.py`（等同 `uv run python -m src.main`）。
3. 操作方式与[使用方式](#使用方式)一节相同；出现横幅时见[疑难排解](#疑难排解)。

### 新增界面语言

界面语言清单由 `src/i18n/` 底下的语言文件扫描而来，**新增一个语言不必改任何代码**：

1. 复制 `src/i18n/zh-TW.json`（源语言，key 最齐全）成 `src/i18n/<语言码>.json`，例如 `ja.json`，把每一条文案翻好。
2. 文件开头这几个字段是该语言自己的数据，不是给译者翻的文案：

   | 字段 | 说明 |
   |------|------|
   | `language.name` | 该语言的自称（endonym），例如 `日本語`。语言菜单在任何界面语言下都显示它、不翻译；也是这个语言的用户首次运行时默认的 `target_language` |
   | `language.font` | 界面字族，例如 `Yu Gothic UI`。没声明则退回 `Segoe UI` |
   | `language.locales` | 这个语言文件要认领哪些 Windows locale 名称，空格分隔（例如 `zh_TW zh_HK zh_MO`）。首次运行检测系统语言时，先看有没有语言文件认领该 locale，没人认领才退回比对语言前缀。**同语言不同字集（`zh_TW` 对繁体、`zh_CN` 对简体）必须指名**，否则两份语言文件抢同一个前缀；`ja_JP` 这种靠前缀就对得上语言码 `ja`，留空即可 |
   | `language.translators` | 这份译文的译者署名，例如 `[GoneTone](https://github.com/GoneTone)、Someone`。可用 `[文字](网址)` 加行内链接（只接受 `http`／`https`）。留空＝不显示；设置窗口的语言下拉底下、首次设置向导的语言步骤与「关于」标签页都会显示它 |

3. `uv run pytest`——`tests/test_i18n.py` 会检查新语言文件的 key 与源语言一致、变量（`{app}` 等）没被翻坏、metadata 有填。

打包时 `build.spec` 以 `src/i18n/*.json` 收录，新文件会自动被带进 exe。

### 检查

```
uv run ruff check src tests   # lint：未使用的 import、未定义名称、import 排序（规则见 pyproject.toml）
uv run pytest                 # 单元测试（默认 4 个 worker 并行跑；要串行跑加 -p no:xdist）
```

推上 GitHub 后 CI（`.github/workflows/ci.yml`）会在 Windows runner 上跑同样的 lint 与测试。

### 打包（exe）

```
uv run pyinstaller build.spec --noconfirm
```

产物在 `dist/Wizard101ChatTranslator.exe`，单一 windowed exe（无控制台黑窗）。分发时只需这个 exe，`config.json`、`app.log` 与 `messages.log` 会在用户第一次运行时自动创建在 exe 同一个文件夹（见[下载](#下载)）。

### 日志文件

两份日志都以「每次启动一段」分段、只保留近 7 天，每行开头是 UTC＋0 时间戳：

- `app.log`：程序诊断输出（挂入、翻译请求、设置生效、异常等）。
- `messages.log`：收信端读到的**原始**聊天内容，不做任何清理与过滤（`RAW` 为含标记的原文、含系统消息，只排除游戏自己灌进聊天控件的 `[WARN]`／`[ERRO]`／`[DBGM]` 调试行；`OUT` 为实际送去翻译的行）。消息漏翻／重复翻译之类的问题请一并附上这份。

### 发布版本

版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)，唯一真实来源是 `src/__init__.py` 的 `__version__`（`pyproject.toml` 的 `version` 只是元数据，由 `tests/test_version.py` 锁定两者一致）。破坏性变更（例如 `config.json` 字段改名）一律进 major 版，重大更新也可能升 major；minor 版加功能、patch 版只修 bug。

发布版本由 GitHub Actions 的 `release-windows` workflow（`.github/workflows/release-windows.yml`）完成，不必在本机打包：

1. 到 GitHub 的 **Actions → release-windows → Run workflow**，填入 tag（`v0.2.0`，一律 `v` 前缀；预发布版写 `v0.2.0-rc.1` 并勾 pre-release）。
2. workflow 会在 `master` 上：把 `src/__init__.py`、`pyproject.toml`、`uv.lock` 的版本号改成 tag 的版本并 commit（`chore(release): bump version to v0.2.0 [skip ci]`）→ 跑 lint 与测试 → `pyinstaller` 打包 → 以该 commit 创建**草稿** Release、上传 `Wizard101ChatTranslator.exe`，版本说明用 GitHub 自动生成的 What's Changed 加上 `.github/release-footer.md` 的固定文案。
3. 到 Releases 页检查草稿（可在最上方补一段人写的版本说明），确认后按 **Publish**。

> 更新检查看的是 GitHub 的 **Release**（`/releases/latest`），草稿与 pre-release 都不算，要按下 Publish 用户端才会收到更新提醒。同一个 tag 重跑 workflow：草稿还在的话，只会把它改指向新的 commit 并重传 exe，版本说明（含你手写的那段）维持原样、不会重新生成；若该 tag 的 Release 已经发布，workflow 会直接失败并拒绝覆盖它的文件。

## 图标

程序图标（`src/assets/icon.ico`）改作自 Wizard101 的游戏图标，右上角叠上「文A」翻译徽章，exe 与所有窗口共用同一份。底图版权属 KingsIsle Entertainment，本项目与其并无隶属关系。
