@echo off
rem build.bat - meteor の cc65 版を 4 つの最適化の設定で作る（Windows 用）
rem 出力：meteor_<設定>.bin / .map / .lbl / .s
set CPU=-t none --cpu 65c02
call :build O0
call :build O -O
call :build Oirs -Oirs
call :build Oirs_Cl -Oirs -Cl
call :build Oirs_Cl_cs -Oirs -Cl --codesize 500
cc65 --version
goto :eof
:build
set NAME=%1
shift
set OPT=%1 %2 %3 %4
cc65 %CPU% %OPT% -o meteor_%NAME%.s meteor.c || exit /b 1
cl65 %CPU% %OPT% -C meteor.cfg -o meteor_%NAME%.bin -m meteor_%NAME%.map -Ln meteor_%NAME%.lbl crt0.s meteor.c || exit /b 1
echo built meteor_%NAME%.bin
goto :eof
