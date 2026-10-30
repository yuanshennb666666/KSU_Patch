# KernelSU 分支 APK 下载清单

本文件记录 `ksupatch` 验证过的 KernelSU 家族发行版，供选择 APK 时参考。APK 不随本工具分发，请从下列上游 Release 自行下载。

**APK 文件名不影响识别**：工具读取 `AndroidManifest.xml` 的包名和内嵌的 KMI 资产表，自动判断是哪个管理器；把任意 APK 改名后仍可正常使用。

下载日期：2026-10-23。SHA-256 为本地实测值，可与上游 Release 资产核对。

| 分支 | 上游 | 版本 | APK 大小 | 兼容性 |
| --- | --- | --- | ---: | --- |
| KernelSU（官方） | <https://github.com/tiann/KernelSU> | v3.3.1 | 13,637,206 | 支持 |
| KernelSU-Next | <https://github.com/KernelSU-Next/KernelSU-Next> | v3.4.2 | 16,682,167 | 支持（含 8 个 SUSFS 辅助模块） |
| SukiSU-Ultra | <https://github.com/SukiSU-Ultra/SukiSU-Ultra> | v4.2.1-r1 | 18,149,797 | 支持（随机化 `lib*.so`） |
| MakoSU（MKSU） | <https://github.com/Spring-bulid/MakoSU> | v4.1.9 | 12,817,275 | 支持（SukiSU-Ultra 下游；x86_64 容器无 LKM 载荷） |
| 7kimisu（7KSU） | <https://github.com/hartgerinktarcila-design/7kimisu> | v2.30 | 13,349,961 | 支持（模块名是 `sevenk.ko`） |
| ReSukiSU | <https://github.com/cctv18/ReSukiSU_CI> | v4.2.0-rc3 | 18,748,828 | 支持（universal 包） |
| BakaSU | <https://github.com/Baka-SU/BakaSU> | v4.2.0-rc3 | 9,379,362 | 支持（ReSukiSU 同源） |
| KittiSU | <https://github.com/terebiko/KittiSU> | 20260928 | 7,794,451 | 支持 |
| XECKernelPro | <https://github.com/byBOOK10086/XECKernelPro> | v31147 | 30,711,242 | 支持（自定义 `xecinit` 辅助 ELF） |
| ApexSU | <https://github.com/qrjhamron/ApexSU> | v2.0.0-beta.5 | 7,281,935 | 不支持：APK 内只有 magiskboot/JNI，没有内嵌 LKM 载荷 |

KowSU（`KOWX712/KernelSU`）只有源码/CI，没有公开的 APK Release，未下载。
SukiSU（非 Ultra）已经并入 SukiSU-Ultra，没有独立的当前发行版；旧 SukiSU 包仍会按包名/资产表识别。

## SHA-256

```text
d3b085adefec18222dc377e6fe1a3a54bb84118cbdb3083fccbf3e22ec08f360  ApexSU_v2.0.0-beta.5.apk
25657bc449439687608fffa04b4b586de90fc405e3dc6217bd997fc71ba0a0a1  BakaSU_v4.2.0-rc3.apk
774a5511f7034a4930e881114e202319cbdc34c5aaaa5c7c6b6d2902364e4c58  KernelSU_Next_v3.4.2.apk
b4a059ace71de2fa65cc7f12b1c58d55fc23ab25e057ee4230ce779742abe450  KernelSU_v3.3.1.apk
085a477f96f322c481ef23c54a755f16a5958c8bff00df4a689ed6e31df94b8d  KittiSU_20260928.apk
037610d4baaaf3713bbd4843e3a94cf0c4e4e1a554a8ea1bb7fc8ee553d76a5d  MakoSU_v4.1.9.apk
f711facb0f6d462bd2c9c46b5badba456352bfe0ab4099c23dccf43adcf9f0f3  ReSukiSU_v4.2.0-rc3.apk
73a0a2d91f06445ac841ea81ea600f0ae9d3997cbf45e58aaa27b9a600779dcb  SukiSU_v4.2.1-r1.apk
e5ea520ae184a310ef5db15b8c50f1c4fc90048c2ff35dedf4879de39d83d575  7kimisu_v2.30.apk
46f03eb601f4d8da338149e9483634cc64a0a94ecef000bf40774bd980afc71e  XECKernelPro_v31147.apk
```

## 下载地址

- <https://github.com/tiann/KernelSU/releases/download/v3.3.1/KernelSU_v3.3.1_32680-release.apk>
- <https://github.com/KernelSU-Next/KernelSU-Next/releases/download/v3.4.2/KernelSU_Next_v3.4.2_33355-release.apk>
- <https://github.com/SukiSU-Ultra/SukiSU-Ultra/releases/download/v4.2.1-r1/SukiSU_v4.2.1-r1_41049-release.apk>
- <https://github.com/Spring-bulid/MakoSU/releases/download/v4.1.9/MakoSU_v4.1.9_40900-release.apk>
- <https://github.com/hartgerinktarcila-design/7kimisu/releases/download/v2.30/7kimisu-v2.30.apk>
- <https://github.com/cctv18/ReSukiSU_CI/releases/download/ReSukiSU_37180105212/ReSukiSU_v4.2.0-rc3_35203-universal-release.apk>
- <https://github.com/Baka-SU/BakaSU/releases/download/v4.2.0-rc3/ReSukiSU_v4.2.0-rc3_35171-arm64-v8a-release.apk>
- <https://github.com/terebiko/KittiSU/releases/download/20260928/KittiSU_20260928-spoofed_35111-arm64-v8a-release.apk>
- <https://github.com/byBOOK10086/XECKernelPro/releases/download/v31147-4ad9ee8a/XECKernelPro_v31001-12fd189e-146-g4ad9ee8_31147-release.apk>
- <https://github.com/qrjhamron/ApexSU/releases/download/v2.0.0-beta.5/ApexSU-v2.0.0-beta.5.apk>

## 使用提示

```text
ksupatch-windows-x64.exe <下载的.apk> <boot.img 或 init_boot.img> --dump-assets
```

先用 `--dump-assets` 确认管理器、ABI、资产表和可用 KMI，再去掉该参数执行实际修补。ApexSU 的 APK 内没有 LKM 载荷，只能做资产检查，不能用于打补丁。
