# ksupatch

离线给 Android GKI 镜像打 KernelSU（LKM 模式）补丁的 Windows 命令行工具。
Offline Windows CLI that patches an Android GKI image with KernelSU in LKM mode.

```
target\release\ksupatch.exe     316,416 字节 (0.30 MB)   单文件静态 exe，零运行时依赖
sha256  85277bec5a456ba098e83c622fb84108e88736adf492f6fa551b4e4255550f7c
```

---

# 中文

## 这是什么

在电脑上给 `boot.img` / `init_boot.img` 打 KernelSU 的 LKM（可加载内核模块）补丁。

- **不需要手机**、不需要 adb、不需要 root、**不需要联网**
- 内核模块和 `ksuinit` 全部**从 KernelSU 安装包（APK）里现取**，所以产物与 APK 版本严格匹配
- 产物与**手机端 KernelSU 安装的结果逐字节一致**（SHA-256 已实测对齐）
- 零运行时依赖，单个 exe 可随意复制到任何目录或 U 盘使用

## 快速开始

```cmd
cd /d <项目目录>\target\release
ksupatch "路径\KernelSU.apk" "路径\init_boot.img" --verify
```

**只需要两个参数。** KMI 会自动判定。

若 `init_boot.img` 所在的固件目录里已有 `boot.img`，工具会**自动从它读出内核版本**，无需手动指定。

## 用法

```
ksupatch <kernelsu.apk> <boot.img|init_boot.img> [<kmi>|auto] [选项]
ksupatch --dump-assets <kernelsu.apk>
```

| 位置参数 | 说明 |
| --- | --- |
| `kernelsu.apk` | 官方 KernelSU 安装包（提供模块 + ksuinit，并锁定版本） |
| `镜像` | 待修补的 `boot.img` 或 `init_boot.img`（header v3/v4） |
| `kmi` | 可省略。可写 `android15-6.6`、裸内核版本 `6.6`、或 `auto` |

| 选项 | 说明 |
| --- | --- |
| `-o, --output <路径>` | 指定保存位置和名称。给文件路径；给目录（末尾带 `\`）则用默认名 |
| `--out <目录>` | 只指定目录 |
| `--out-name <文件>` | 只指定文件名 |
| `--verify` | 重新解析产物并断言注入点正确 |
| `--dump-assets` | 只列出 APK 里的内嵌资产及判定依据 |
| `--arch <架构>` | `aarch64`（默认）或 `x86_64`，一般自动识别 |
| `--module <文件>` | 用自己的 `.ko` 覆盖 APK 里的 |
| `--kmi-from <镜像>` | 从另一个镜像读内核版本 |
| `--color <when>` | `auto`（默认）/ `always` / `never` |
| `--ksu-config <token>` | 写入 `ksu_config`（默认不写，与手机端一致） |
| `--no-stock-backup` | 不写 `stock_image.sha1`（默认写，与手机端一致） |
| `--help` | 帮助 |

默认输出：当前目录，文件名 `kernelsu_patched_YYYYMMDD_HHMMSS.img`（时间戳每次运行当场生成，UTC，与手机端命名规则一致，同名不覆盖）。

## KMI 怎么定的

**KMI 跟内核版本走，不跟 Android 平台版本走。** `android15-6.6` = android15 分支的 6.6 内核。

`auto` 按"证据有多直接"依次尝试，全部离线：

1. 镜像自带的 kernel 段（`boot.img`）
2. ramdisk 里已存在 `*.ko` 的 `vermagic`（已装过 KernelSU 的 `init_boot`）
3. **同目录的固件 `boot.img`**（只认规范文件名 `boot.img`，并用架构交叉过滤，避免误读其它设备的镜像）
4. `--kmi-from <镜像>`

都不成立就明确报错、列出 APK 提供的全部 KMI。

> ⚠️ **纯 stock 的 `init_boot.img` 单独一个文件无法自动判定** —— `init_boot` 正是 Android 13 为了只放 ramdisk 而切出来的分区，**内核不在里面**（官方实现同样做不到）。这种情况在旁边放个 `boot.img`，或直接写 `6.6`。

## 输出与退出码

纯英文的八阶段流程，每次都打印实际取值和判断依据。终端上彩色，**重定向到文件时自动关闭**（日志里不会混进转义符）。

```
0 成功   1 参数错   2 输入文件   3 APK   4 镜像格式
5 KMI    6 资产     7 注入       8 读写
```

报错格式固定：`阶段 / 输入 / 原因 / 建议动作`。

## 限制

- `--verify` 只能证明「注入点正确 + 载荷与官方资产逐字节一致 + 分区大小保持」。**它无法离线证明内核会加载这个模块** —— 那个只有刷进去开机才知道。
- 支持 `boot` / `init_boot` 的 **header v3/v4**；v0/v1/v2 与 `vendor_boot` 会**明确报错**，不静默处理。
- **刷机前请备份原分区。**

## 构建

```cmd
cd /d <项目目录>
build.bat
```

`build.bat` 编译完成后用 `tools\upx.exe` 压缩。首次构建需要联网拉依赖，之后完全离线可用（依赖已在本地 cargo 缓存里）。

## 验收状态

```
part 1   行为与退出码                            15/15
part 1b  同目录 boot.img 自动解析 / 无兄弟时拒绝      2/2
part 2   载荷与官方 Release 资产字节一致（两架构）     2/2
part 3   真实镜像  无需参数解析 KMI · 修补+自检 · 保持分区大小 · 载荷一致
                                        23 passed, 0 failed
```

真机实测（Android 15，内核 `6.6.77-android15-8`，ramdisk `LZ4_LEGACY`）：
产物 `init_boot_patched.img` 内 `kernelsu.ko` 与 `init` 的 SHA-256 与官方 Release 资产**逐字节一致**；
原 `init` 无损保存为 `init.real`，其余 16 个条目零改动，分区大小保持 8,388,608 字节。

## 目录与辅助工具

```
ksupatch\
  target\release\ksupatch.exe     可执行文件（要用的就是这个）
  build.bat                       构建 + 压缩
  Cargo.toml / Cargo.lock         依赖清单（Rust）
  build.rs                        把图标嵌进 exe
  assets\icon.ico                 图标（取自 KernelSU APK 的启动图标）
  src\                            源码
  tools\                          辅助与验证脚本、upx.exe
```

| 脚本 | 用途 |
| --- | --- |
| `tools\selftest.ps1` | 全套验收回归（23 项）；`-PhoneImage <文件>` 可加"与手机产物对哈希" |
| `tools\inspect_ramdisk.py <镜像>` | 拆开镜像看 ramdisk 内容、格式、条目、KMI 线索 |
| `tools\diff_images.py <a> <b>` | 两个镜像逐条目差分 |
| `tools\verify_payload.py <镜像> <模块资产> <ksuinit资产>` | 与官方 Release 资产做字节校验 |
| `tools\match_phone.ps1 -Phone <手机产物>` | 自动试 4 种开关组合，找出与手机产物匹配的那个 |
| `tools\make_fixtures.py` | 生成测试夹具 |
| `tools\make_icon.py` | 从 APK 重新生成图标 |

## 许可证

**GPL-3.0**，见 `LICENSE`。第三方组件及各自授权见 `THIRD-PARTY.md`。

之所以是 GPL-3.0：本工具**静态链接**了 `android-bootimg`（该库是 GPL-3.0），并且内置图标取自 GPL-3.0 的 KernelSU。

---

# English

## What it is

Patches a `boot.img` / `init_boot.img` with KernelSU in LKM (loadable kernel module) mode,
entirely on a PC.

- **No phone, no adb, no root, no network**
- The kernel module and `ksuinit` are taken **straight out of the KernelSU APK**, so the
  result is locked to that exact release
- The output is **byte-for-byte identical to what an install on the device produces**
  (SHA-256 verified)
- Zero runtime dependencies: one static exe you can copy anywhere

## Quick start

```cmd
cd /d <project>\target\release
ksupatch "path\KernelSU.apk" "path\init_boot.img" --verify
```

**Two arguments is all it takes.** The KMI is worked out automatically.

If the firmware folder already holds `boot.img` next to `init_boot.img`, the kernel version
is read from it, so nothing has to be typed.

## Usage

```
ksupatch <kernelsu.apk> <boot.img|init_boot.img> [<kmi>|auto] [options]
ksupatch --dump-assets <kernelsu.apk>
```

| Argument | Meaning |
| --- | --- |
| `kernelsu.apk` | Official KernelSU Manager APK (supplies the module and ksuinit, anchors the version) |
| image | The `boot.img` or `init_boot.img` to patch (header v3/v4) |
| `kmi` | Optional. Either `android15-6.6`, a bare kernel version `6.6`, or `auto` |

| Option | Meaning |
| --- | --- |
| `-o, --output <path>` | Where to write the result: a file path, or a directory (trailing separator) to use the default name |
| `--out <dir>` | Directory only |
| `--out-name <file>` | File name only |
| `--verify` | Re-parse the produced image and assert the install is present |
| `--dump-assets` | List the assets inside the APK and the evidence behind each KMI decision |
| `--arch <arch>` | `aarch64` (default) or `x86_64`; normally detected |
| `--module <file>` | Use this kernelsu.ko instead of the one inside the APK |
| `--kmi-from <image>` | Read the kernel version from another image |
| `--color <when>` | `auto` (default) / `always` / `never` |
| `--ksu-config <tokens>` | Write a `ksu_config` file (by default none is written, matching the device) |
| `--no-stock-backup` | Do not record `stock_image.sha1` (by default it is recorded, matching the device) |
| `--help` | Show help |

Default output: the current directory, named `kernelsu_patched_YYYYMMDD_HHMMSS.img`. The
timestamp is taken at run time (UTC, same convention as the device), so runs never collide.

## How the KMI is chosen

**The KMI follows the KERNEL, never the Android platform version.** `android15-6.6` is the
android15 branch's 6.6 kernel.

`auto` tries these in order of how direct the evidence is, all offline:

1. the kernel inside the image (`boot.img`)
2. the `vermagic` of a `*.ko` already in the ramdisk (an init_boot that was patched before)
3. **the firmware's `boot.img`, if it sits next to this image** (only the exact name
   `boot.img` is accepted, and the architecture is cross-checked, so an image belonging to
   another device is not picked up by accident)
4. `--kmi-from <image>`

If none of them applies, it fails with an explicit message listing every KMI the APK
provides.

> ⚠️ **A pristine `init_boot.img` on its own cannot be resolved** -- `init_boot` is the
> partition Android 13 carved out to hold only the ramdisk, so the kernel is simply not in
> that file (the official implementation cannot do it either). Put a `boot.img` next to it,
> or pass the kernel version, e.g. `6.6`.

## Output and exit codes

An English, eight-stage trace that prints every value it actually used and the evidence
behind it. Coloured on a terminal, and **colour turns itself off when the output is
redirected**, so logs never collect escape sequences.

```
0 ok   1 usage   2 input   3 apk   4 image   5 kmi   6 asset   7 inject   8 io
```

Failures always carry: stage / input / reason / hint.

## Limits

- `--verify` proves the install points are present, that the payload matches the official
  release assets byte for byte, and that the partition size is preserved. **It cannot prove
  offline that the kernel will load the module** -- only booting the device can.
- `boot` and `init_boot` headers **v3/v4** are supported. v0/v1/v2 and `vendor_boot` are
  rejected with an explicit reason rather than handled silently.
- **Back up the original partition before flashing.**

## Building

```cmd
cd /d <project>
build.bat
```

`build.bat` builds and then compresses the result with `tools\upx.exe`. The first
build needs the network for dependencies; later builds work offline from the local cargo
cache.

## Verification status

```
part 1   behaviour and exit codes                                15/15
part 1b  KMI from a sibling boot.img / refusal when there is none  2/2
part 2   payload equals the official release assets (both ABIs)    2/2
part 3   real device image: KMI with no argument, patch + verify,
         partition size preserved, payload identical
                                                23 passed, 0 failed
```

Measured on a real device image (Android 15, kernel
`6.6.77-android15-8`, `LZ4_LEGACY` ramdisk): the `kernelsu.ko` and `init` inside the
produced `init_boot_patched.img` are **byte-for-byte identical** to the official release
assets, the original `init` is preserved losslessly as `init.real`, the other 16 entries are
untouched, and the image stays 8,388,608 bytes.

## Layout and helper scripts

```
ksupatch\
  target\release\ksupatch.exe     the executable you run
  build.bat                       build + compress
  Cargo.toml / Cargo.lock         dependency manifest (Rust)
  build.rs                        embeds the icon into the exe
  assets\icon.ico                 icon, taken from the KernelSU APK's launcher icon
  src\                            source
  tools\                          helper and verification scripts, plus upx.exe
```

| Script | Purpose |
| --- | --- |
| `tools\selftest.ps1` | Full acceptance regression (23 checks); add `-PhoneImage <file>` for the phone-equality check |
| `tools\inspect_ramdisk.py <image>` | Unpack an image: ramdisk contents, format, entries, KMI hints |
| `tools\diff_images.py <a> <b>` | Entry-by-entry diff of two images |
| `tools\verify_payload.py <image> <module-asset> <ksuinit-asset>` | Byte-check against the official release assets |
| `tools\match_phone.ps1 -Phone <image>` | Try all four switch combinations and report which matches the phone's output |
| `tools\make_fixtures.py` | Generate the test fixtures |
| `tools\make_icon.py` | Regenerate the icon from the APK |

## License

**GPL-3.0**; see `LICENSE`. Third-party components and their licences are listed in
`THIRD-PARTY.md`.

The reason is that the tool statically links `android-bootimg`, which is itself GPL-3.0, and
that the bundled icon comes from KernelSU, likewise GPL-3.0.
