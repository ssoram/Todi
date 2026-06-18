@echo off
chcp 65001 >nul
setlocal

set "REPO=ssoram/Todi"

set "DIR=%LOCALAPPDATA%\Todi"
set "EXE=%DIR%\Todi.exe"

if exist "%EXE%" (
    echo Todi를 실행합니다...
    start "" "%EXE%"
    exit
)

echo Todi를 설치 중...
if not exist "%DIR%" mkdir "%DIR%"

for /f "tokens=*" %%i in ('powershell -Command "(Invoke-RestMethod https://api.github.com/repos/%REPO%/releases/latest).assets | Where-Object { $_.name -like '*.exe' -and $_.name -notlike '*uninstall*' } | Select-Object -First 1 -ExpandProperty browser_download_url"') do set "URL=%%i"

if "%URL%"=="" (
    echo 다운로드 링크를 찾지 못했습니다.
    pause
    exit /b 1
)

powershell -Command "Invoke-WebRequest '%URL%' -OutFile '%EXE%'"

echo 설치 완료! 실행합니다...
echo (이후 업데이트는 자동으로 적용됩니다)
start "" "%EXE%"
exit
