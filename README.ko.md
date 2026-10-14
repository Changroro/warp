# Warp 개인 포크

[English](README.md) · **한국어**

[warpdotdev/warp](https://github.com/warpdotdev/warp)를 기반으로 SSH 작업 흐름을 Termius 수준으로 끌어올린 개인 포크예요. Warp의 원래 기능은 그대로 두고, 아래 기능을 더했어요. 업스트림 소개는 [README.md](README.md) 아래쪽에 있어요.

## SSH 매니저

**→ 호스트를 `~/.ssh/config`에 그대로 저장해요.** Warp에서 추가하거나 이름을 바꾼 호스트는 다른 터미널의 `ssh <별칭>`에서도 그대로 쓸 수 있어요. 손으로 쓴 호스트도 패널에 나타나요. 알 수 없는 지시어와 주석, `Include`/`Match` 블록은 건드리지 않고, 저장할 때마다 이전 파일을 `~/.ssh/config.warp.bak`에 남겨요.

**→ 호스트 패널.** 즐겨찾기, 태그 그룹, 필터, 마지막 접속 시각과 열린 연결 수를 보여 줘요. 여러 호스트를 골라 한 번에 열 수 있어요.

**→ 런처 (`Cmd/Ctrl+Shift+H`).** 저장된 호스트를 검색하거나 `user@host:port`를 입력해 바로 접속해요. 접속하면서 저장할 수도 있어요. 새 탭 메뉴에도 SSH 항목이 있어요.

**→ 호스트의 Warp 탭을 그대로 열기.** 호스트에 마우스를 올리고 레이아웃 버튼을 누르면, 그 서버에서 돌고 있는 WarpOss의 탭을 이름, 색, 분할 모양, 작업 디렉터리까지 똑같이 열어요. 각 창은 그 서버에 SSH로 접속해 같은 디렉터리로 들어가요. 원격 세션 DB는 `sqlite3 -readonly`로 읽기만 하고, 이 탭을 닫아도 서버 쪽 세션에는 영향이 없어요.

**→ 원격 파일 탐색.** SSH 세션에서 Warp의 파일 트리로 원격 파일을 열어 볼 수 있어요.

**→ 이미지 붙여넣기.** SSH 탭에 클립보드 이미지를 붙여넣으면 원격 `/tmp`에 권한 `0600`으로 올리고 그 경로를 입력해요. 원격에서 돌리는 Claude Code나 Codex에 이미지를 넘길 때 써요.

**→ 자동 재연결.** 연결이 끊기면 세션 바에 이유를 보여 주고, 카운트다운 후 최대 5번 다시 접속해요. 명령줄에 입력 중인 내용이 있으면 재연결하지 않아요.

**→ 저장한 비밀번호를 안전하게 입력해요.** 비밀번호는 시스템 키체인(macOS) 또는 보안 저장소(Linux)에 보관해요. 접속할 때는 `ssh`가 WarpOss를 `SSH_ASKPASS` 도우미로 불러서 그 호스트의 `user@host` 비밀번호 질문에 **처음 한 번만** 답해요. 호스트 키 확인, 점프 호스트, OTP 같은 다른 질문은 터미널에 그대로 물어봐요. OpenSSH 8.4 이상이 필요해요.

## 세션 보관함

세로 탭 패널에서 탭을 닫아도 어디서 작업하던 세션인지 기록이 남아요. 탭 제목, 작업 디렉터리, git 브랜치, 쓰던 CLI 에이전트, 마지막 사용 시각을 보관하고, 누르면 그 디렉터리에서 다시 열려요. 드래그로 순서를 바꿀 수 있고, 탭 검색에도 잡혀요.

## 설치

[Releases](https://github.com/Changroro/warp/releases)에서 `fork-v*` 빌드를 받아요.

- Linux x86_64: `.deb` 또는 `.AppImage`
- macOS Apple Silicon: `.dmg` (애드혹 서명이라 처음 실행이 막히면 `xattr -dr com.apple.quarantine /Applications/WarpOss.app`)

> [!IMPORTANT]
> 원격 파일 탐색과 "호스트의 Warp 탭 열기"는 **Settings → Warpify → Warpify SSH Sessions**가 켜져 있어야 동작해요. 탭 열기는 원격 서버에 `sqlite3`와 WarpOss가 있어야 해요.

## 브랜치

`master`는 업스트림을 그대로 따라가고, 기능은 `feat/*`에서 만들어 `dev`에 합쳐요. 실제로 쓰는 빌드는 `dev`예요. 동기화와 릴리스 방법은 [FORK.md](FORK.md)에 있어요.

<!-- 스크린샷: 호스트 패널, 런처, 원격 탭 열기 -->
