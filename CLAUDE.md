# Todi

## 커밋 규칙
- 커밋 메시지에 `Co-Authored-By: Claude ...` 같은 AI 공동 작성자 줄을 **넣지 않는다.**
  (GitHub에서 커밋이 Claude와의 공동 작업으로 표시되기 때문)
- PR 설명에도 `🤖 Generated with Claude Code` 줄을 넣지 않는다.

## 릴리스 규칙
- 새 기능이 들어간 릴리스는 `sticker-tauri/src/guide.js`의 `GUIDE` 배열 **맨 뒤**에 새 버전 블록을 추가한다.
  (version은 `tauri.conf.json`과 동일하게. 업데이트한 사용자에게는 그 버전 내용만, 처음 설치한 사용자에게는 전체가 보인다.)
- 설명에 쓰는 스크린샷은 실제 화면을 캡처해 `sticker-tauri/src/guide/`에 2배 해상도 PNG로 넣는다.
