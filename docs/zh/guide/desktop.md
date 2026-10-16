# 桌面客户端

AGT Studio 是 agt 控制台的桌面版，支持 macOS、Windows 和 Linux。它的页面和 `agt web` 一样，但直接在应用内部调用 agt，不经过本机网页服务；关掉窗口后常驻系统托盘，有事需要你处理时会弹系统通知。

## 安装

从[最新发布](https://github.com/blacsheep/agt/releases/latest)下载对应系统的安装包：

| 系统 | 文件 |
| --- | --- |
| macOS（Apple 芯片和 Intel） | `AGT.Studio_<版本>_universal.dmg` |
| Windows | `AGT.Studio_<版本>_x64-setup.exe` 或 `.msi` |
| Linux | `.AppImage`、`.deb` 或 `.rpm` |

Linux 上需要 WebKitGTK 4.1（`libwebkit2gtk-4.1-0`），`.deb` 和 `.rpm` 安装包会自动装上。

::: warning 未签名的版本
安装包没有签名，系统会要求你确认一次。

**macOS：** 第一次打开 AGT Studio 时会被拦截。点**完成**，打开**系统设置 → 隐私与安全性**，在页面下方点**仍要打开**，再输入密码确认。如果提示"已损坏，无法打开"，在终端里清除下载时附带的隔离标记，然后重新打开：

```bash
xattr -dr com.apple.quarantine "/Applications/AGT Studio.app"
```

**Windows：** SmartScreen 可能弹出警告，点**更多信息** → **仍要运行**。
:::

## 使用的数据

- **和命令行同一个工作空间：** `$AGT_HOME`，或 `~/.agt`。用 `agt` 或 `agt web` 创建的资源、Agent 和会话都会出现在应用里，反过来也一样。
- **和 `agt web` 同一个操作身份**（`web-operator`），两者可以管理同一个工作空间。
- **只有一个可执行文件。** 页面需要的 agt 命令也由应用自己执行，不需要另外安装。想在终端里用，仍然可以单独安装 `agt` 命令行。

同一个工作空间不要同时运行 `agt web` 和桌面应用：两者都会启动这个工作空间的调度器，而同一时间只能有一个。

## 托盘和通知

关闭窗口后，AGT Studio 会留在系统托盘里，Agent 照常回复，定时任务照常触发。点托盘图标可以重新打开窗口；在托盘菜单里选**退出**才会关闭应用和它的调度器。

窗口在后台时，以下情况会弹系统通知：

- 有 Agent 在你的会话里发了消息；
- [定时任务](./timers.md)到点；
- 有请求在等待你[审批](./permissions.md)。

系统可能会询问一次是否允许 AGT Studio 显示通知。

## 更新

如果发布时附带了更新签名，应用启动时会自动下载并安装更新，并提示你，下次打开时生效。没有签名的版本需要下载新的安装包来升级。

## 从源码构建

```bash
cargo install tauri-cli --version "^2.12" --locked
cd crates/agt-desktop
cargo tauri dev       # 运行，修改 web/public 后自动刷新
cargo tauri build     # 在 target/release/bundle 下生成安装包
```

Linux 上需要先安装 WebView 和托盘相关的库，例如 Ubuntu：

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```
