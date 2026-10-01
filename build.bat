@echo off
setlocal

rem ============================================================================
rem  Build ksupatch, then compress the result with UPX.
rem
rem  Usage:  build.bat
rem
rem  The first build downloads the Rust dependencies; later builds work offline
rem  from the local cargo cache.
rem
rem  This file is deliberately ASCII-only: a .bat with non-ASCII text is read in
rem  the console code page and turns into garbage.
rem ============================================================================

echo cargo build --release
cargo build --release
if errorlevel 1 goto :failed

set "EXE=target\release\ksupatch.exe"
if not exist "%EXE%" goto :failed

if exist "tools\upx.exe" (
    echo.
    echo compressing with UPX...
    "tools\upx.exe" --best --lzma "%EXE%"
    if errorlevel 1 echo WARNING: UPX failed; the uncompressed binary is still usable
) else (
    echo.
    echo WARNING: tools\upx.exe not found, skipping compression
)

echo.
for %%F in ("%EXE%") do echo built : %%~fF
for %%F in ("%EXE%") do echo size  : %%~zF bytes
endlocal
exit /b 0

:failed
echo.
echo BUILD FAILED
endlocal
exit /b 1
