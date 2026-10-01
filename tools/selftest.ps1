# End-to-end acceptance test for ksupatch.
#
# Part 1 exercises every code path against the synthetic fixtures and checks the exit code.
# Part 2 proves the installed payload is byte-for-byte the official KernelSU release asset.
#
# Usage:  powershell -ExecutionPolicy Bypass -File tools\selftest.ps1

# The phone's own patched image, for the headline equality check.
#
# It is only meaningful against the exact stock file the phone patched, so it is opt-in:
# pass -PhoneImage <file> when you have a matching pair. Leave it empty otherwise.
param([string]$PhoneImage = "")

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\release\ksupatch.exe"

if (-not (Test-Path $exe)) { Write-Host "build first: build.bat"; exit 1 }
if (-not (Test-Path (Join-Path $root "fixtures\boot_v4.img"))) {
    & python (Join-Path $root "tools\make_fixtures.py") (Join-Path $root "fixtures") | Out-Null
}

Push-Location $root

# ---------------------------------------------------------------------------------------
# Part 1 -- behaviour and exit codes
# ---------------------------------------------------------------------------------------
$cases = @(
    @{ n = "boot v4, auto KMI + verify";        a = @("KernelSU.apk", "fixtures\boot_v4.img", "auto", "--verify", "--out", "out\cases"); e = 0 },
    @{ n = "boot v3, auto KMI";                 a = @("KernelSU.apk", "fixtures\boot_v3.img", "auto", "--out", "out\cases"); e = 0 },
    @{ n = "boot v4, explicit KMI";             a = @("KernelSU.apk", "fixtures\boot_v4.img", "android14-6.1", "--out", "out\cases"); e = 0 },
    @{ n = "init_boot, no KMI (must refuse)";   a = @("KernelSU.apk", "fixtures\init_boot_v4.img"); e = 5 },
    @{ n = "init_boot, KMI given + verify";     a = @("KernelSU.apk", "fixtures\init_boot_v4.img", "android13-5.10", "--verify", "--out", "out\cases"); e = 0 },
    @{ n = "init_boot, --arch x86_64";          a = @("KernelSU.apk", "fixtures\init_boot_v4.img", "android16-6.12", "--arch", "x86_64", "--out", "out\cases"); e = 0 },
    @{ n = "x86_64 + android12-5.10 (pair)";    a = @("KernelSU.apk", "fixtures\init_boot_v4.img", "android12-5.10", "--arch", "x86_64", "--out", "out\cases"); e = 0 },
    @{ n = "x86_64 + android13-5.10 (pair)";    a = @("KernelSU.apk", "fixtures\init_boot_v4.img", "android13-5.10", "--arch", "x86_64", "--out", "out\cases"); e = 0 },
    @{ n = "x86_64 + android13-5.15 (pair)";    a = @("KernelSU.apk", "fixtures\init_boot_v4.img", "android13-5.15", "--arch", "x86_64", "--out", "out\cases"); e = 0 },
    @{ n = "magisk-patched (must refuse)";      a = @("KernelSU.apk", "fixtures\boot_magisk.img", "auto"); e = 7 },
    @{ n = "legacy header v2 (must refuse)";    a = @("KernelSU.apk", "fixtures\boot_old.img", "auto"); e = 4 },
    @{ n = "KMI absent from APK (must refuse)"; a = @("KernelSU.apk", "fixtures\boot_v4.img", "android99-9.9"); e = 5 },
    @{ n = "not a boot image (must refuse)";    a = @("KernelSU.apk", "Cargo.toml", "auto"); e = 4 },
    @{ n = "missing apk (must refuse)";         a = @("nope.apk", "fixtures\boot_v4.img", "auto"); e = 2 },
    @{ n = "dump-assets self-check";            a = @("--dump-assets", "KernelSU.apk"); e = 0 }
)

Write-Host "part 1 -- behaviour and exit codes"
$pass = 0
$fail = 0
foreach ($c in $cases) {
    $a = $c.a
    $out = (& $exe @a 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $ok = ($code -eq $c.e)
    if ($ok) { $pass++ } else { $fail++ }

    $detail = ""
    foreach ($key in @("auto-detected", "reason :", "result        :")) {
        $line = ($out -split "`r?`n" | Where-Object { $_ -match [regex]::Escape($key) } | Select-Object -First 1)
        if ($line) { $detail = $line.Trim() }
    }

    Write-Host ("  [{0}] {1,-32} exit={2} (want {3})  {4}" -f `
        $(if ($ok) { "PASS" } else { "FAIL" }), $c.n, $code, $c.e, $detail)
    if (-not $ok) {
        ($out -split "`r?`n" | Where-Object { $_ -match "reason|hint" }) | ForEach-Object { Write-Host "        $_" }
    }
}

# ---------------------------------------------------------------------------------------
# Part 1b -- KMI read from the firmware's boot.img sitting next to init_boot.img
#
# This is also the only way to exercise the pristine, never-patched install path: the
# ramdisk has an `init` and no `init.real`, so the rename has to happen.
# ---------------------------------------------------------------------------------------
Write-Host ""
Write-Host "part 1b -- KMI from the sibling boot.img"

$sb = "out\sibling"
Remove-Item -Recurse -Force $sb -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $sb | Out-Null
Copy-Item "fixtures\init_boot_v4.img" "$sb\init_boot.img"
Copy-Item "fixtures\boot_v4.img" "$sb\boot.img"

$out = (& $exe "KernelSU.apk" "$sb\init_boot.img" "--verify" "--out" "$sb\patched" 2>&1 | Out-String)
$ok = ($LASTEXITCODE -eq 0) -and ($out -match "sibling .boot\.img.") -and ($out -match "fresh install") -and ($out -match "init\.real")
if ($ok) { $pass++ } else { $fail++ }
Write-Host ("  [{0}] no KMI given: resolved from the kernel in boot.img, fresh install" -f $(if ($ok) { "PASS" } else { "FAIL" }))
($out -split "`r?`n" | Where-Object { $_ -match "auto-detected|mode  |init  " }) | ForEach-Object { Write-Host "        $($_.Trim())" }
if (-not $ok) { $out -split "`r?`n" | Where-Object { $_ -match "reason|hint" } | ForEach-Object { Write-Host "        $_" } }

# Without a sibling there is nothing to read, and the tool must say so rather than guess.
Remove-Item "$sb\boot.img" -Force
$out = (& $exe "KernelSU.apk" "$sb\init_boot.img" 2>&1 | Out-String)
$ok = ($LASTEXITCODE -eq 5) -and ($out -match "no kernel version could be established")
if ($ok) { $pass++ } else { $fail++ }
Write-Host ("  [{0}] no KMI and no sibling: refuses instead of guessing" -f $(if ($ok) { "PASS" } else { "FAIL" }))

# ---------------------------------------------------------------------------------------
# Part 2 -- the installed payload must equal the official release asset, byte for byte
# ---------------------------------------------------------------------------------------
Write-Host ""
Write-Host "part 2 -- payload identity against the official release assets"

$payloads = @(
    @{ n = "aarch64 android13-5.10"; img = "fixtures\boot_v4.img";        extra = @("auto");                          mod = "lkm-aarch64-android13-5.10_kernelsu.ko"; init = "ksuinit-aarch64"; dir = "out\p-a64" },
    @{ n = "x86_64 android12-5.10";  img = "fixtures\init_boot_v4.img";   extra = @("android12-5.10", "--arch", "x86_64"); mod = "lkm-x86_64-android12-5.10_kernelsu.ko"; init = "ksuinit-x86_64";  dir = "out\p-x64" }
)

foreach ($p in $payloads) {
    $a = @("KernelSU.apk", $p.img) + $p.extra + @("--out", $p.dir)
    & $exe @a | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host ("  [FAIL] {0}: patch failed" -f $p.n)
        $fail++
        continue
    }
    $img = (Get-ChildItem (Join-Path $p.dir "*.img") | Sort-Object LastWriteTime | Select-Object -Last 1).FullName
    $out = (& python "tools\verify_payload.py" $img $p.mod $p.init 2>&1 | Out-String)
    $ok = ($out -match "kernelsu\.ko\s+: IDENTICAL") -and ($out -match "init\s+: IDENTICAL")
    if ($ok) { $pass++ } else { $fail++ }
    Write-Host ("  [{0}] {1,-20} payload {2}" -f $(if ($ok) { "PASS" } else { "FAIL" }), $p.n, $(if ($ok) { "IDENTICAL to " + $p.mod } else { "DIFFERS" }))
    if (-not $ok) { $out -split "`r?`n" | ForEach-Object { Write-Host "        $_" } }
}

# ---------------------------------------------------------------------------------------
# Part 3 -- optional smoke test on a real device image
#
# Runs only when a real image is present in the repo root. Set the two variables below to
# match that image: the KMI it needs, and the release asset the module must equal.
# ---------------------------------------------------------------------------------------
$realImage = "init_boot.img"
# The KMI follows the KERNEL, not the platform: this image's build.prop reports Android 17,
# but the device kernel is 6.6, and 6.6 exists in this APK only as android15-6.6.
$realKmi = "android15-6.6"
$realModuleAsset = "lkm-aarch64-android15-6.6_kernelsu.ko"
$realInitAsset = "ksuinit-aarch64"

# Optional engine-equivalence golden.
#
# The phone-produced image is the ground truth for this project, but it is tied to one exact
# stock file; compare only when the caller supplies a matching pair via -PhoneImage.
$realPhoneImage = $PhoneImage

if (Test-Path $realImage) {
    Write-Host ""
    Write-Host "part 3 -- real device image ($realImage)"

    # With no KMI argument at all, the tool must work the version out by itself and say
    # which evidence it used.
    $out = (& $exe "KernelSU.apk" $realImage "--out" "out\auto" 2>&1 | Out-String)
    $autoOk = ($LASTEXITCODE -eq 0) -and ($out -match "auto-detected") -and ($out -match [regex]::Escape($realKmi))
    if ($autoOk) { $pass++ } else { $fail++ }
    Write-Host ("  [{0}] KMI resolved with no argument given" -f $(if ($autoOk) { "PASS" } else { "FAIL" }))
    ($out -split "`r?`n" | Where-Object { $_ -match "auto-detected|resolved  " }) | ForEach-Object { Write-Host "        $($_.Trim())" }
    if (-not $autoOk) { $out -split "`r?`n" | Where-Object { $_ -match "reason|hint" } | ForEach-Object { Write-Host "        $_" } }

    $a = @("KernelSU.apk", $realImage, $realKmi, "--verify", "--out", "out\real")
    $out = (& $exe @a 2>&1 | Out-String)
    $ok = ($LASTEXITCODE -eq 0) -and ($out -match "result        : PASS")
    if ($ok) { $pass++ } else { $fail++ }
    Write-Host ("  [{0}] patch + verify" -f $(if ($ok) { "PASS" } else { "FAIL" }))

    # The produced image must keep the partition size and stay LZ4-compressed.
    $img = Get-ChildItem "out\real\*.img" | Sort-Object LastWriteTime | Select-Object -Last 1
    $sameSize = ($img.Length -eq (Get-Item $realImage).Length)
    if ($sameSize) { $pass++ } else { $fail++ }
    Write-Host ("  [{0}] output keeps the partition size ({1:N0} bytes)" -f `
        $(if ($sameSize) { "PASS" } else { "FAIL" }), $img.Length)

    $vout = (& python "tools\verify_payload.py" $img.FullName $realModuleAsset $realInitAsset 2>&1 | Out-String)
    $idOk = ($vout -match "kernelsu\.ko\s+: IDENTICAL") -and ($vout -match "init\s+: IDENTICAL")
    if ($idOk) { $pass++ } else { $fail++ }
    Write-Host ("  [{0}] payload IDENTICAL to {1}" -f $(if ($idOk) { "PASS" } else { "FAIL" }), $realModuleAsset)
    if (-not $idOk) { $vout -split "`r?`n" | ForEach-Object { Write-Host "        $_" } }

    # The headline requirement: the defaults must reproduce the phone's own output.
    if ($realPhoneImage -and (Test-Path $realPhoneImage)) {
        $ph = (Get-FileHash $realPhoneImage -Algorithm SHA256).Hash
        $m = @("KernelSU.apk", $realImage, $realKmi, "--out", "out\phone_eq")
        & $exe @m | Out-Null
        $mimg = Get-ChildItem "out\phone_eq\*.img" | Sort-Object LastWriteTime | Select-Object -Last 1
        $mh = (Get-FileHash $mimg.FullName -Algorithm SHA256).Hash
        $mOk = ($mh -eq $ph)
        if ($mOk) { $pass++ } else { $fail++ }
        Write-Host ("  [{0}] byte-identical to the phone's own patched image" -f $(if ($mOk) { "PASS" } else { "FAIL" }))
        Write-Host ("        phone {0}" -f $ph)
        Write-Host ("        ours  {0}" -f $mh)
    } else {
        Write-Host "  [skip] phone-image equality (pass -PhoneImage <file> with a matching stock)"
    }
} else {
    Write-Host ""
    Write-Host "part 3 -- skipped (no $realImage in the repo root)"
}

Pop-Location
Write-Host ""
Write-Host ("{0} passed, {1} failed" -f $pass, $fail)
if ($fail -gt 0) { exit 1 }
