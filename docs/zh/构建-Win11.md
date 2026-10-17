# 在 Win11 上构建 MeetingMinutes

本项目 fork 自 [Meetily](https://github.com/Zackriya-Solutions/meeting-minutes)。有两种方式拿到 Win11 安装包：用 GitHub Actions 构建（推荐，不用在本机装工具链），或在 Win11 本机从源码构建。

## 方式一：GitHub Actions 构建（推荐）

1. 打开仓库的 **Actions** 页，左侧选 **Build and Test - Windows**。
2. 点 **Run workflow**，分支选要构建的分支，`Build type` 选 `release`，**不要勾选 `Sign the build`**（签名需要上游公司的证书，fork 里没有），`Upload build artifacts` 保持勾选。
3. 一次完整构建可能接近一小时。成功后在运行页底部的 **Artifacts** 里下载 `meetily-windows-x64-release-<版本号>`，里面有 NSIS 安装程序（`nsis/*.exe`）和 MSI（`msi/*.msi`）。
4. 安装包未签名，Win11 首次运行可能弹出 SmartScreen 提示，点“更多信息 → 仍要运行”。

CI 构建出的是 Vulkan 版（GPU 加速转写），要求 CPU 支持 AVX2（不需要 AVX-512）。

macOS 版同理：在 Actions 里选 **Build and Test - macOS**，同样不勾选 `Sign the build`（默认已不勾选）。产物是 Apple Silicon 版 `.dmg`，未签名，首次打开要在 Finder 里右键应用选“打开”。

## 方式二：Win11 本机从源码构建

### 1. 安装依赖

| 依赖 | 版本或说明 |
|---|---|
| Node.js | 20 |
| pnpm | 9.15.9：`npm install -g pnpm@9.15.9` |
| Rust | stable，目标 `x86_64-pc-windows-msvc`（rustup 默认） |
| Visual Studio 2022 Build Tools | 勾选“使用 C++ 的桌面开发”工作负载（含 Windows SDK） |
| CMake | 最新版，加入 PATH |
| LLVM | 安装到 `C:\Program Files\LLVM`，并设置 `LIBCLANG_PATH=C:\Program Files\LLVM\bin` |
| Vulkan SDK（可选） | 1.4.309.0，只有要构建 Vulkan 版时才需要 |

### 2. 构建（PowerShell，从仓库根目录开始）

```powershell
# 1) 安装前端依赖
cd frontend
pnpm install --frozen-lockfile
cd ..

# 2) 先构建 llama-helper 侧车程序（打包时必须存在，tauri build 不会自动构建它）
cargo build --release -p llama-helper
New-Item -ItemType Directory -Force -Path frontend\src-tauri\binaries | Out-Null
Copy-Item target\release\llama-helper.exe frontend\src-tauri\binaries\llama-helper-x86_64-pc-windows-msvc.exe

# 3) 打包（默认 CPU 版；装了 Vulkan SDK 可改用 tauri:build:vulkan）
cd frontend
pnpm run tauri:build:cpu
```

也可以直接运行 `frontend\build-gpu.bat`，它会自动构建并复制 llama-helper，再按检测到的 GPU 选择加速方式。

### 3. 输出位置

- NSIS 安装程序：`target\release\bundle\nsis\`
- MSI 安装包：`target\release\bundle\msi\`

## 国内网络注意事项

构建和首次使用时需要访问以下地址，在国内可能需要代理：

| 时机 | 地址 | 用途 |
|---|---|---|
| 构建 | registry.npmjs.org、crates.io | 前端和 Rust 依赖 |
| 构建 | github.com（Releases） | `build.rs` 下载 FFmpeg 和 ONNX Runtime 1.22.0 |
| 首次使用 | huggingface.co | 下载 Whisper / Parakeet 转写模型 |

如果构建机下载 GitHub Releases 困难，可以把 FFmpeg 预先放到 `frontend/src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe`，`build.rs` 检测到可用文件时会跳过下载。ONNX Runtime 会做文件校验，预放置的规则见 `frontend/src-tauri/build/onnxruntime.rs`。

## 使用前须知（阶段一）

- 默认转写引擎 Parakeet **不支持中文**。测试中文时，在设置里把转写引擎切到 **Whisper**，下载 `small` 或 `base` 模型。
- 自动更新目前仍指向上游 Meetily，**弹出更新提示时不要点**，否则会被替换成上游版本（M1-02 处理）。
- 数据统计（PostHog）目前仍开着，阶段一只能使用不涉密的测试材料（M1-02 处理）。
- 应用数据目录：`%APPDATA%\cn.meetingminutes.app`，与原版 Meetily 分开。

## 常见问题

| 现象 | 原因和处理 |
|---|---|
| Actions 在 “Sync Certificate to Windows Certificate Store” 一步报 `Invalid credentials` | 勾选了 `Sign the build`。取消勾选重新运行。 |
| 打包时报找不到 `binaries/llama-helper-x86_64-pc-windows-msvc.exe` | 没有先构建侧车，按上面第 2 步执行。 |
| 编译 whisper-rs 或 llama-cpp 时报找不到 libclang | 没装 LLVM 或没设 `LIBCLANG_PATH`。 |
