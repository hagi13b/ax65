@echo off
setlocal enabledelayedexpansion
rem Build ax65 and compile the samples. All output goes to compiler\logs\ .
rem Usage: double-click, or run "tests\build_all.bat" in the compiler folder.
cd /d "%~dp0\.."
if not exist logs mkdir logs

echo [1/4] cargo build --release
cargo build --release > logs\build.txt 2>&1
echo exit=%ERRORLEVEL% >> logs\build.txt
if not exist target\release\ax65.exe goto end

echo [2/4] cargo test
cargo test --release > logs\test.txt 2>&1
echo exit=%ERRORLEVEL% >> logs\test.txt

echo [3/4] meteor -O0 -O1 -O2 -Os
for %%L in (O0 O1 O2 Os) do (
  target\release\ax65.exe build --project ..\sample\meteor -t v2 -%%L -o build\%%L --emit bin,woz,sym,lst,report > logs\meteor_%%L.txt 2>&1
  echo exit=!ERRORLEVEL! >> logs\meteor_%%L.txt
)
target\release\ax65.exe check --project ..\sample\meteor --all-targets > logs\meteor_check.txt 2>&1

echo [4/4] meteor7 -O2
target\release\ax65.exe build --project ..\sample\meteor7 -t v2 -O2 -o build\meteor7_O2 --emit bin,woz,sym,lst,report > logs\meteor7_O2.txt 2>&1

:end
echo Done. Results are in compiler\logs and compiler\build.
pause
