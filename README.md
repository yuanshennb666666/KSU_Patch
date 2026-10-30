# ksupatch

在 PC 上离线为 Android GKI 镜像写入 **KernelSU LKM** 补丁。不依赖手机、adb 或网络。

| 项目 | 值 |
| --- | --- |
| 版本 | 0.5.4 |
| 平台 | Windows x64 / x86 / ARM64，Linux x86_64 |
| 运行时依赖 | 无（单文件；Linux 为静态 musl 构建） |
| 许可 | GPL-3.0 |

## 下载文件

| 平台 | 文件 | 大小 | SHA-256 |
| --- | --- | ---: | --- |
| Windows x64 | `ksupatch-windows-x64.exe` | 327,680 | `25613b2d452b5f036f93441016d41c6f50ebdc945a62f8c974681f7b97aeb09e` |
| Windows x86 | `ksupatch-windows-x86.exe` | 327,168 | `8830c7c281c3607ac2f570e0aa1cfff2b5f560e8bbfe9ed04434ba59efcce415` |
| Windows ARM64 | `ksupatch-windows-arm64.exe` | 833,024 | `c1aa26ea1ce0699b8ca4a58a87a267719adbe0038120d124d8670643fe0d7a92` |
| Linux x86_64 | `ksupatch-linux-x86_64` | 441,000 | `ad6972db209d291fcdb67bf6d83e79133d88fe666b4cd01f1be3186b8334228e` |

Windows x64/x86 与 Linux 版本经过 UPX 压缩；ARM64 版本为未压缩的 PE，因为 UPX 5.1.1 尚不支持 `win64/arm64`。所有文件均为单文件，无需安装。

## 为什么用它

- **完全离线**：在电脑上完成，运行时不连接手机、adb 或网络。
- **版本锁定**：内核模块和 `ksuinit` 直接从 KernelSU 家族 APK 中提取，产物对应该 APK 的版本。
- **结果可复现**：在已实测的镜像和版本上，产物与手机端安装结果逐字节一致。
- **管理器识别**：按 APK 包名和内嵌资产表自动识别官方 KernelSU、KernelSU-Next、SukiSU-Ultra、MakoSU、7kimisu 等分支；APK 文件名任意。
- **多平台**：同一个命令行工具覆盖 Windows 与 Linux，不依赖 .NET、VC++ 运行库或 Wine。

## 快速开始

Windows（x64 为例）：

```cmd
ksupatch-windows-x64.exe "KernelSU.apk" "init_boot.img" --verify
```

Linux：

```sh
chmod +x ksupatch-linux-x86_64
./ksupatch-linux-x86_64 "KernelSU.apk" "init_boot.img" --verify
```

两个参数即可，KMI 会自动解析：

1. 从 `boot.img` 的 kernel 段读取内核版本；
2. 从 ramdisk 中已有模块的 `vermagic` 读取；
3. 从同目录固件的 `boot.img` 读取；如果该内核没有 Android 分支标记，则结合 `build.prop` / `prop.default` 判断。

`--kmi-from <镜像>` 是显式覆盖：先读取并校验；与镜像 kernel 不一致时直接报错，不会静默选中不匹配的模块。

如果 `init_boot.img` 旁边有同固件的 `boot.img`，工具会自动读取；纯 stock 的 `init_boot.img` 单独一个文件不包含 kernel，此时把 `boot.img` 放在旁边，或直接写 `6.6` / 完整的 `uname -r`。

## 支持范围

### 管理器

| 管理器 | 补丁 | 说明 |
| --- | --- | --- |
| KernelSU（官方） | 支持 | 完整流程 |
| KernelSU-Next | 支持 | 自动排除随包的 susfs 辅助模块 |
| SukiSU-Ultra | 支持 | 随机化 `lib*.so` 也能识别 |
| MakoSU（MKSU） | 支持 | SukiSU-Ultra 下游 |
| 7kimisu（7KSU） | 支持 | 模块文件名为 `sevenk.ko`，工具自动适配 |
| ReSukiSU / BakaSU | 支持 | 完整流程 |
| KittiSU | 支持 | 完整流程 |
| XECKernelPro | 支持 | 识别自定义 `xecinit` 辅助 ELF |
| ApexSU | APK 检查 | APK 资产可检查；补丁需要内嵌 LKM 载荷 |

### KMI

| KMI | Android 分支 |
| --- | --- |
| `android12-5.10` / `android13-5.10` | 5.10 |
| `android13-5.15` / `android14-5.15` | 5.15 |
| `android14-6.1` | 6.1 |
| `android15-6.6` | 6.6 |
| `android16-6.12` | 6.12 |
| `android17-6.18` | 6.18 |

只要 APK 中带有对应 `.ko`，就可以为该 KMI 打补丁。载荷选择覆盖 `aarch64`、`x86_64`、`riscv64` 三种 64 位 ABI，具体取决于 APK 提供的资产。

### 镜像

- `boot.img`
- `init_boot.img`
- `vendor_boot.img`
- GKI header **v3 / v4**

## 用法

```text
ksupatch <manager.apk> <image> [<kmi>|auto] [选项]
ksupatch --dump-assets <manager.apk>
ksupatch --version
```

### 位置参数

| 参数 | 说明 |
| --- | --- |
| `manager.apk` | KernelSU 家族管理器 APK，提供内核模块和 `ksuinit`，并锁定版本。文件名任意，按包名与资产表识别 |
| `image` | 待修补的 `boot.img`、`init_boot.img` 或 `vendor_boot.img` |
| `kmi` | 可省略。可写完整 KMI（`android15-6.6`）、裸内核版本（`6.6`）、手机 `uname -r` 的完整 release（`6.6.77-android15-8-g...`）或 `auto` |

### 选项

| 选项 | 说明 |
| --- | --- |
| `-o, --output <路径>` | 输出文件；或输出目录（末尾带 `\` 或 `/`），使用默认文件名 |
| `--out <目录>` | 只指定输出目录 |
| `--out-name <文件名>` | 只指定输出文件名；只接受纯文件名，不能带目录、盘符或 `..` |
| `--module <文件>` | 用自己的 `.ko` 覆盖 APK 中的模块；会校验 ELF 类型、ABI、vermagic 和 KMI |
| `--arch <架构>` | `aarch64`（默认）/ `x86_64` / `riscv64`，通常自动识别 |
| `--kmi-from <镜像>` | 显式从另一个镜像读取 KMI；与镜像 kernel 冲突会报错 |
| `--ksu-config <token>` | 写入 `ksu_config`（默认不写，与手机端一致） |
| `--no-stock-backup` | 不写 `stock_image.sha1`（默认写，与手机端一致） |
| `--verify` | 重新解析产物，检查 `init`、`init.real` 和模块 |
| `--dump-assets` | 只检查 APK：管理器、ABI、资产表、KMI、自检结果 |
| `-v, --verbose` | 输出完整八阶段细节；默认只显示一屏摘要 |
| `--color <when>` | `auto`（默认）/ `always` / `never`；支持 `NO_COLOR` |
| `-h, --help` | 显示帮助 |
| `-V, --version` | 显示版本 |

默认输出到当前目录，文件名 `kernelsu_patched_YYYYMMDD_HHMMSS.img`。时间戳按 UTC 在运行时生成；重名会自动追加 `-1`、`-2`，不覆盖已有文件。

## KMI 如何确定

**KMI 由内核版本决定。** `android15-6.6` 指 android15 分支的 6.6 内核；Android 平台版本只用于区分共用同一内核版本的分支。

`auto` 按证据的直接程度依次尝试，全部离线：

1. 镜像自带的 kernel 段（`boot.img`）
2. ramdisk 中已有 `*.ko` 的 `vermagic`（曾经安装过 KernelSU 的 `init_boot`）
3. 同目录固件的 `boot.img`（只认规范文件名，并交叉检查架构）

如果都无法确定，工具会列出 APK 提供的全部 KMI，并给出明确错误。

`--kmi-from <镜像>` 是显式覆盖：先读取并校验；与镜像 kernel 不一致时直接报错。显式写完整 KMI、裸内核版本或完整 `uname -r` 时，也会与镜像 kernel 交叉校验，冲突时返回 exit 5，不会静默装入不匹配的模块。

完整 `uname -r` 按以下规则解析：

```text
5.10.198-android13-8-g1a2b3c4d5e6f  ->  android13-5.10
6.6.77-android15-8-gdeadbeef        ->  android15-6.6
5.10.198-g1a2b3c4d5e6f              ->  kernel 5.10, branch from build.prop
5.10.198                            ->  kernel 5.10, branch from the image/build.prop
```

release 中带 `androidNN` 时直接用它和 `major.minor` 组成 KMI；APK 没有该 KMI 就报错。release 不带 `androidNN` 时，才回退到镜像 kernel 或 ramdisk 的 `build.prop` / `prop.default` 消歧：`5.10` + Android 13 → `android13-5.10`，Android 12 → `android12-5.10`。

## 输出

默认只打印一屏摘要，输入与结果分块对齐：

```text
ksupatch 0.5.4

  APK       KernelSU.apk  [13.01 MiB]
  Image     output\init_boot.img  [8.00 MiB]

  Manager   KernelSU (official)  v3.3.1
  KMI       android13-5.10
  Module    kernelsu.ko  360.0 KiB
  Output    temp\neo_boot.img  [8.00 MiB]
  Result    verified  2.51s
```

加 `--verbose`（`-v`）展开完整八阶段，适合排查 APK 资产选择或 KMI 判定。终端上默认彩色；重定向到文件时自动关闭颜色。

### 退出码

```text
0 成功   1 参数错   2 输入文件   3 APK   4 镜像格式
5 KMI    6 资产     7 注入       8 读写
```

失败时输出 `ERROR <阶段>` 以及 `Input / Cause / Hint / Exit`，便于定位是文件路径、APK 资产还是镜像格式问题。

## 平台说明

- **系统要求**：Windows 10 1809+ / Windows 11；Linux 需要 x86_64 内核与基本 shell，静态 musl 构建不依赖发行版库。
- **Windows x64 / x86**：原生控制台程序。x86 版本显式嵌入 `asInvoker` manifest，不会因为文件名含 `patch` 触发 Windows 的安装程序检测而请求管理员权限；不需要 UAC。
- **Windows ARM64**：LLVM/MinGW-w64 UCRT 构建，适用于 Windows on ARM；未用 UPX 压缩。只能在 ARM64 Windows 上运行，x64 主机无法直接执行验证。
- **Linux x86_64**：静态链接 musl，不依赖系统 glibc 或额外的动态库；`chmod +x` 后直接运行。

## 从源码构建

需要 Rust stable、C 编译器（`lz4-sys` 需要）和 PowerShell。

Windows x64：

```cmd
build.bat
```

Windows x86（在 x86 MSVC 开发者环境中）：

```cmd
cargo build --release --target i686-pc-windows-msvc
```

Windows ARM64（LLVM/MinGW-w64 UCRT 工具链）：

```cmd
rustup target add aarch64-pc-windows-gnullvm
cargo build --release --target aarch64-pc-windows-gnullvm
```

Linux x86_64（musl 工具链）：

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

发布包中的 x64/x86/Linux 文件随后用以下命令压缩：

```text
tools\upx.exe --best --lzma <文件>
```

构建脚本 `build.bat` 只构建当前 Windows 主机并调用 UPX；`package.bat` 将 `release\` 中的四个平台文件、文档和校验和打包到 `dist\`。默认不嵌入图标；需要图标时设置 `KSUPATCH_ICON=1` 后构建。

## 目录

```text
ksupatch\
  src\                         Rust 源码
  assets\ksupatch.manifest     asInvoker 清单（x86 必需）
  assets\icon.ico              可选图标
  docs\ksu-branches.md         分支 APK 下载清单
  build.bat                    Windows x64 构建 + UPX
  package.bat                  组装发布目录与 zip
  tools\upx.exe                UPX 5.1.1
  Cargo.toml / Cargo.lock      Rust 依赖
  release\                     已编译的四个平台文件
```

## 边界与安全

- `--verify` 覆盖注入点、模块与 ksuinit 的字节一致性，以及 `init.real` 是否存在；内核实际加载模块的行为需要在刷机后确认。
- 支持 `boot` / `init_boot` / `vendor_boot` 的 GKI header v3/v4。
- LKM 载荷使用 64 位 ABI：`aarch64`、`x86_64`、`riscv64`。
- 补丁面向 stock、未叠加其它 ramdisk 补丁的镜像；已带 Magisk 标记的 ramdisk 会被识别并停止，避免叠加。
- **刷机前请备份原分区。**

## 许可证

**GPL-3.0**，见 [`LICENSE`](LICENSE)。第三方组件及授权见 [`THIRD-PARTY.md`](THIRD-PARTY.md)。

本工具静态链接 GPL-3.0 的 `android-bootimg`；可选图标取自 GPL-3.0 的 KernelSU。

---

## English

**ksupatch** is an offline command-line tool that patches an Android GKI `boot.img` / `init_boot.img` / `vendor_boot.img` with KernelSU in LKM mode.

- Runs entirely on a PC: no phone, adb, root, or network at run time.
- The kernel module and `ksuinit` come from the KernelSU-family APK, so the result is tied to that release.
- Byte-for-byte identical to the phone-side install on the images and versions tested.
- Supports official KernelSU, KernelSU-Next, SukiSU-Ultra, MakoSU (MKSU), 7kimisu (7KSU), ReSukiSU/BakaSU, KittiSU, and XECKernelPro. The APK file name is arbitrary; the package and embedded asset table identify it.
- Handles `boot`, `init_boot`, and `vendor_boot` with GKI header v3/v4.
- Available as Windows x64, Windows x86, Windows ARM64, and static Linux x86_64 binaries.

### Quick start

```cmd
ksupatch-windows-x64.exe "path\KernelSU.apk" "path\init_boot.img" --verify
```

```sh
chmod +x ksupatch-linux-x86_64
./ksupatch-linux-x86_64 "path/KernelSU.apk" "path/init_boot.img" --verify
```

Two arguments are enough; the KMI is resolved automatically. When a stock `init_boot.img` is alone, put the firmware's `boot.img` next to it, or pass a bare kernel version such as `6.6` or the full `uname -r` release.

### Usage

```text
ksupatch <manager.apk> <image> [<kmi>|auto] [options]
ksupatch --dump-assets <manager.apk>
ksupatch --version
```

Useful options: `-o/--output`, `--out`, `--out-name`, `--module`, `--arch`, `--kmi-from`, `--ksu-config`, `--no-stock-backup`, `--verify`, `--dump-assets`, `-v/--verbose`, `--color auto|always|never`.

The default output is a one-screen summary; `--verbose` prints the full eight-stage trace. Colours are enabled on a terminal and disabled automatically when redirected.

### Platform notes

- **Requirements**: Windows 10 1809+ or Windows 11; any x86_64 Linux with a basic shell. The Linux build is statically linked against musl, so no distribution libraries are required.
- **Windows x64 / x86**: native console builds. The x86 image embeds an explicit `asInvoker` manifest, so Windows does not raise a UAC prompt just because the name contains "patch". No runtime redistributable is needed.
- **Windows ARM64**: LLVM/MinGW-w64 UCRT build for Windows on ARM; not UPX-compressed. It cannot be executed on an x64 host.
- **Linux x86_64**: statically linked against musl; copy it, `chmod +x`, and run.

### Build

Windows x64: `build.bat`. Other targets:

```text
cargo build --release --target i686-pc-windows-msvc
cargo build --release --target aarch64-pc-windows-gnullvm
cargo build --release --target x86_64-unknown-linux-musl
```

A C compiler is required for `lz4-sys`; ARM64 and Linux use the corresponding LLVM/musl toolchains. UPX-compress the x64/x86/Linux files with `tools\upx.exe --best --lzma`.

### Licence

GPL-3.0. Third-party notices are in [`THIRD-PARTY.md`](THIRD-PARTY.md).
