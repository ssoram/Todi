#!/bin/bash

REPO="ssoram/Todi"

APP_DIR="$HOME/Applications"
APP_PATH="$APP_DIR/Todi.app"
DMG_PATH="/tmp/Todi.dmg"

if [ -d "$APP_PATH" ]; then
    echo "Todi를 실행합니다..."
    open "$APP_PATH"
    exit 0
fi

echo "Todi를 설치 중..."

URL=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" \
  | grep "browser_download_url.*\.dmg" \
  | head -1 \
  | cut -d '"' -f 4)

if [ -z "$URL" ]; then
    echo "다운로드 링크를 찾지 못했습니다."
    exit 1
fi

curl -L "$URL" -o "$DMG_PATH"

hdiutil attach "$DMG_PATH" -quiet
mkdir -p "$APP_DIR"
cp -R /Volumes/Todi/*.app "$APP_DIR/"
hdiutil detach /Volumes/Todi -quiet
rm "$DMG_PATH"

echo "설치 완료! 실행합니다..."
echo "(이후 업데이트는 자동으로 적용됩니다)"
open "$APP_PATH"
