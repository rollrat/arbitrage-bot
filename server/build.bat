@echo off
REM Docker 이미지 빌드 스크립트 (Windows)
REM 사용법: build.bat [태그]

setlocal

set IMAGE_NAME=perp-scanner-server
set TAG=%1
if "%TAG%"=="" set TAG=latest
set FULL_IMAGE_NAME=%IMAGE_NAME%:%TAG%

echo 🔨 Docker 이미지 빌드 중...
echo 이미지 이름: %FULL_IMAGE_NAME%

docker build -t %FULL_IMAGE_NAME% .

if %ERRORLEVEL% EQU 0 (
    echo ✅ 빌드 완료: %FULL_IMAGE_NAME%
    echo.
    echo 실행 방법:
    echo   docker run -p 12090:12090 %FULL_IMAGE_NAME% perp-scanner-server
    echo   docker run -v /path/to/data:/app/data %FULL_IMAGE_NAME% analysis-chat --files /app/data/file.json
) else (
    echo ❌ 빌드 실패
    exit /b 1
)

