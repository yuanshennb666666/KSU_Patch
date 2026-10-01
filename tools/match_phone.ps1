# Find which switch combination reproduces a phone-produced patched image, byte for byte.
#
# The repack itself is already proven byte-equivalent to the official implementation; what
# is left is knowing which options the phone's install used. Those come down to two things:
#
#   ksu_config      an install that uses a module supplied as a file writes no config at
#                   all, while one that uses the built-in module writes `bundled=1`
#   stock backup    an install that protects its backup also records `stock_image.sha1`
#
# The defaults already match the device-side install, so a plain run is usually the answer;
# this script checks all four combinations and reports which one matches.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File tools\match_phone.ps1 -Phone <patched.img>
#   powershell -ExecutionPolicy Bypass -File tools\match_phone.ps1 -Phone <patched.img> `
#       -Stock init_boot.img -Kmi android15-6.6 -Apk KernelSU.apk

param(
    [Parameter(Mandatory = $true)][string]$Phone,
    [string]$Stock = "init_boot.img",
    [string]$Kmi = "android15-6.6",
    [string]$Apk = "KernelSU.apk"
)

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\release\ksupatch.exe"

foreach ($p in @($exe, (Join-Path $root $Phone), (Join-Path $root $Stock), (Join-Path $root $Apk))) {
    if (-not (Test-Path $p)) { Write-Host "missing: $p"; exit 1 }
}

Push-Location $root
Remove-Item -Recurse -Force "out\phone" -ErrorAction SilentlyContinue

$phoneHash = (Get-FileHash $Phone -Algorithm SHA256).Hash
$phoneSize = (Get-Item $Phone).Length
Write-Host "phone image    : $Phone"
Write-Host "                 $phoneSize bytes"
Write-Host "                 sha256 $phoneHash"
Write-Host "stock image    : $Stock ($((Get-Item $Stock).Length) bytes)"
Write-Host "kmi            : $Kmi"
Write-Host ""

$variants = @(
    @{ n = "default (no config, backup)";     extra = @() },
    @{ n = "--ksu-config bundled=1";          extra = @("--ksu-config", "bundled=1") },
    @{ n = "--no-stock-backup";               extra = @("--no-stock-backup") },
    @{ n = "bundled=1 + --no-stock-backup";   extra = @("--ksu-config", "bundled=1", "--no-stock-backup") }
)

$match = $null
foreach ($v in $variants) {
    $dir = Join-Path "out\phone" ($v.n -replace '[^A-Za-z0-9]+', '_')
    $a = @($Apk, $Stock, $Kmi) + $v.extra + @("--out", $dir, "--out-name", "candidate.img")
    & $exe @a | Out-Null
    if ($LASTEXITCODE -ne 0) { Write-Host ("  [skip] {0}: patch failed" -f $v.n); continue }

    $img = Join-Path $dir "candidate.img"
    $h = (Get-FileHash $img -Algorithm SHA256).Hash
    $same = ($h -eq $phoneHash)
    if ($same) { $match = $v }
    Write-Host ("  [{0}] {1,-34} {2}" -f $(if ($same) { "MATCH" } else { "     " }), $v.n, $h)
}

Write-Host ""
if ($match) {
    Write-Host ("MATCH: the phone's install used: {0}" -f $match.n)
    if ($match.extra.Count -eq 0) {
        Write-Host ("       equivalent command: ksupatch.exe {0} {1} {2}   (the defaults already match)" -f $Apk, $Stock, $Kmi)
    } else {
        Write-Host ("       equivalent command: ksupatch.exe {0} {1} {2} {3}" -f $Apk, $Stock, $Kmi, ($match.extra -join " "))
    }
} else {
    Write-Host "No switch combination matches. Showing the entry-level diff against the closest"
    Write-Host "candidate so the remaining difference is visible:"
    Write-Host ""
    $closest = Get-ChildItem "out\phone\*\candidate.img" | Select-Object -First 1
    & python "tools\diff_images.py" $Phone $closest.FullName
}

Pop-Location
