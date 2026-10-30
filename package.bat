@echo off
setlocal

rem ============================================================================
rem  Assemble the release directory and zip from the four binaries in release\.
rem
rem  Usage:  package.bat
rem
rem  Run build.bat first for the Windows x64 binary, then place the x86, ARM64
rem  and Linux builds in release\ with the exact names listed below.
rem
rem  Produces:
rem    dist\ksupatch-<version>\             four binaries + docs + SHA256SUMS.txt
rem    dist\ksupatch-<version>.zip          the release archive
rem
rem  This file is deliberately ASCII-only (a .bat with non-ASCII text is read
rem  in the console code page and turns into garbage).
rem ============================================================================

set "VERSION="
for /f "tokens=2 delims== " %%V in ('findstr /b "version" "%~dp0Cargo.toml"') do set "VERSION=%%~V"
if not defined VERSION set "VERSION=dev"

set "NAME=ksupatch-%VERSION%"
set "DIST=%NAME%"
set "ZIP=%NAME%.zip"

for %%F in (
    "ksupatch-windows-x64.exe"
    "ksupatch-windows-x86.exe"
    "ksupatch-windows-arm64.exe"
    "ksupatch-linux-x86_64"
) do (
    if not exist "release\%%~F" (
        echo missing release\%%~F
        echo build the four targets first, or see README.md
        exit /b 1
    )
)

if exist "dist\%DIST%" rmdir /s /q "dist\%DIST%"
mkdir "dist\%DIST%"
mkdir "dist\%DIST%\docs"

for %%F in (
    "ksupatch-windows-x64.exe"
    "ksupatch-windows-x86.exe"
    "ksupatch-windows-arm64.exe"
    "ksupatch-linux-x86_64"
) do copy /y "release\%%~F" "dist\%DIST%\%%~F" >nul

copy /y "README.md" "dist\%DIST%\README.md" >nul
copy /y "LICENSE" "dist\%DIST%\LICENSE" >nul
copy /y "THIRD-PARTY.md" "dist\%DIST%\THIRD-PARTY.md" >nul
copy /y "docs\ksu-branches.md" "dist\%DIST%\docs\ksu-branches.md" >nul

powershell -NoProfile -Command "$d='dist\%DIST%'; Get-ChildItem $d -File | Sort-Object Name | ForEach-Object { '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name } | Set-Content -Encoding ascii (Join-Path $d 'SHA256SUMS.txt')"
if errorlevel 1 goto :failed

if exist "dist\%ZIP%" del /q "dist\%ZIP%"
powershell -NoProfile -Command "Compress-Archive -Path 'dist\%DIST%\*' -DestinationPath 'dist\%ZIP%' -Force"
if errorlevel 1 goto :failed

echo.
powershell -NoProfile -Command "$z=Get-Item 'dist\%ZIP%'; 'package: ' + $z.FullName; 'size   : {0:N0} bytes ({1:N3} MiB)' -f $z.Length, ($z.Length / 1MB); 'sha256 : ' + (Get-FileHash $z.FullName -Algorithm SHA256).Hash.ToLower()"
endlocal
exit /b 0

:failed
echo.
echo PACKAGE FAILED
endlocal
exit /b 1
