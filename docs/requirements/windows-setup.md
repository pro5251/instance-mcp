# Windows 一步安裝 + Claude Code 自動註冊（spec v0.2）

> Recorded 2026-10-08; open questions resolved 2026-10-14 against the author's conventions (§12).

**Status:** ready-for-agent（§1–§4 使用者決定；§12 依作者做法定案；下一步 `/to-tickets`）

**相關：** `.scratch/windows-poc/spec.md`（v0.4，名稱依 §13.2）、tickets 11（Playwright）、14（packaging）

---

## 1. 背景與問題

2026-10-08 在 repo 內直接雙擊 `poc/reverse-attach-linux/windows/install.cmd` 失敗：

```
missing D:\instance-mcp\poc\reverse-attach-linux\windows\oab-imcp-winpoc.exe beside this script
```

根因：`install.cmd` → `install-winpoc.ps1:51-52` 要求 exe 在腳本旁，但 exe 只存在於 release ZIP（`release.yml:253-265` 編 `reverse-attach.exe` 改名後打包）。repo 本身沒有 exe，所以從 clone 下來的 repo 無法安裝。

同時查到的既有缺口：

| # | 缺口 | 位置 |
|---|---|---|
| G1 | repo 無 exe；`install.cmd` 只能在 ZIP 內用 | `install-winpoc.ps1:51-52` |
| G2 | fork `pro5251/instance-mcp` 沒有任何 Release；`release.yml` 的 `windows` job 等 macOS job 建 release（`:156`），fork 無 Apple secrets（`:32-40`）→ Windows ZIP 永遠發不出來；MSVC 版因此從未編過 | `.github/workflows/release.yml` |
| G3 | `browser_*` 在 Windows 沒接通：安裝腳本不做 `npm install @playwright/mcp`／`npx playwright install chromium`（與 `pw-mcp.ps1:3-4` 註解不符）；無 Task 啟動 `pw-mcp.ps1`；node Task 沒帶 `--upstream` | `install-winpoc.ps1`、`pw-mcp.ps1` |
| G4 | `windows/README.md` 宣稱「node picks up `pw-mcp.ps1` automatically」，與實作不符 | `windows/README.md` |
| G5 | 安裝只印 URL/token，不設定任何 MCP client | `install-winpoc.ps1:93-98` |

背景補充：Playwright browser upstream 是作者既有功能（macOS `poc/pw-mcp/` 0.1.0、Linux #29 以 `MCP_UPSTREAM` env）。Windows 用 `--upstream` 旗標因 Scheduled Task 不繼承環境變數（`install-winpoc.ps1:56-57`）。

## 2. 目標

**誰：** 拿到這個 repo（clone，不是 ZIP）的 Windows 使用者／測試者；不假設有 Rust、MSVC、gh。

**成功標準：**
1. 雙擊 `scripts\setup-windows.cmd`（或請 AI 照 `docs/windows-setup.md` 指引執行），無需 admin。
2. 結束時：node 健康（`/healthz` = ok）、開機自動啟動、token 已產生並保留。
3. Claude Code 已自動註冊此 node，`claude mcp list` 可見且連線成功。
4. 有 Node.js 時 `browser_*` 可用；沒有時其餘工具照常，印提示。
5. 可重複執行（更新）與可完整移除。

**非目標：** exe 自身安裝子指令（D7）；Claude Desktop／OpenAB Connect 註冊；code signing；自動安裝 Rust/MSVC 或 Node.js。

## 3. 硬性限制（CLAUDE.md）

- 不動 macOS（Swift）與 Linux 行為；**不改作者的 `release.yml`**、不改 Rust 程式碼。
- 名稱依 `.scratch/windows-poc/spec.md` §13.2 POC 名稱。
- 回歸門檻不退步：`cargo fmt --check`、`cargo clippy --release -- -D warnings`、`cargo test --release`（18 passed）、`smoke.sh`（48/48）。
- 不經詢問不 push；功能分支 `feat/windows-setup` 開發。

## 4. 使用者已決定事項（2026-10-08）

| 問題 | 決定 |
|---|---|
| Q1 註冊哪個 MCP client | **Claude Code** only |
| Q2 exe 來源 | **從 GitHub Release 下載**（fork `pro5251/instance-mcp`），sha256 驗證 |
| Q3 Release 怎麼產生 | **新增獨立 workflow** `release-windows-poc.yml`，`winpoc-v*` tag 或 `workflow_dispatch` 觸發，自己 `gh release create`；不動 `release.yml` |
| Q4 安裝入口 | **repo 內 `setup-windows.cmd`**（位置見 D3）+ AI 安裝指引（位置見 D2）；只改 PowerShell/cmd |
| Q5 Playwright | **一起修；偵測到 node/npm 才裝**，否則跳過並提示 |

補充：Release workflow 只在推 `winpoc-v*` tag 或手動觸發時跑，一般 push 不跑。使用者拿到的是最新 Release；安裝一律用 ZIP 內的 `install-winpoc.ps1`，確保 exe 與安裝腳本同版。

---

## 5. 架構

```
[repo] scripts\setup-windows.cmd ──► scripts\setup-windows.ps1 (bootstrap)
          │ 1. 查 <Repo> 最新 winpoc-v* release（GitHub REST API，免 gh）
          │ 2. 下載 ZIP + .sha256 → 比對 → 解壓到 %TEMP%\oab-imcp-winpoc-<ver>\
          ▼
   ZIP 內 install-winpoc.ps1 -Install          （ZIP 內 install.cmd 走同一入口）
          │ a. 既有：exe、token、Scheduled Task、/healthz、tailscale serve
          │ b. 新：browser（§7），預設開，-NoBrowser 關
          │ c. 新：Claude Code 註冊（§8），預設開，-NoClaudeCode 關
          ▼
   印 ready 摘要（URL、token 位置、browser 狀態、Claude Code 狀態）
```

### 5.1 元件

| 檔案 | 新/改 | 職責 |
|---|---|---|
| `scripts/setup-windows.cmd` | 新 | 雙擊入口；`powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup-windows.ps1" %*`；結尾 `pause` |
| `scripts/setup-windows.ps1` | 新 | 只負責「取得正確版本 ZIP → 驗證 → 解壓 → 呼叫 ZIP 內 installer」；不含安裝邏輯 |
| `.github/workflows/release-windows-poc.yml` | 新（fork 專用） | 編譯、打包、建 release、上傳（§10） |
| `poc/reverse-attach-linux/windows/install-winpoc.ps1` | 改 | 加 browser 段、Claude Code 段；uninstall 對稱 |
| `poc/reverse-attach-linux/windows/pw-mcp.ps1` | 改 | 對齊安裝路徑；修正註解 |
| `poc/reverse-attach-linux/windows/README.md` | 改 | 修 G4；指向 `docs/windows-setup.md` |
| `docs/windows-setup.md` | 新 | 完整安裝指南 + AI 安裝指引（§9），對應作者的 `docs/linux-setup.md` |
| `README.md` | 改（小） | Quick start 加一段 **Windows (POC)**，連到 `docs/windows-setup.md` |

## 6. setup-windows.ps1

**參數：**
- `-Version <tag>`：鎖定版本（預設最新 `winpoc-v*`）
- `-ZipPath <path>`：用本機 ZIP，跳過下載（離線／測試／CI）
- `-Repo <owner/name>`：預設值放在腳本開頭變數區，POC 期間為 `pro5251/instance-mcp`；併入 upstream 時改成 `openabdev/instance-mcp`（只改一處，同 §13.2 原則）
- 其餘參數原樣轉給 `install-winpoc.ps1`（`-Token`、`-AllowLogin`、`-SkipTailscale`、`-NoStart`、`-NoBrowser`、`-NoClaudeCode`）

**流程：**
1. `GET https://api.github.com/repos/<repo>/releases?per_page=30` → 過濾 `tag_name -like 'winpoc-v*'` 且非 draft → 取最新（或 `-Version` 指定）。作者的 Linux quick start 同樣用 releases API 查版本（README「Quick start」）。
2. 找資產 `oab-imcp-winpoc-<ver>-windows-amd64.zip` 與 `.zip.sha256`。
3. 下載到 `%TEMP%\oab-imcp-winpoc-<ver>\`；`Get-FileHash -Algorithm SHA256` 比對，不符即中止並刪檔。
4. `Expand-Archive`；`Unblock-File` 解出的檔案（移除 MOTW，降低 SmartScreen 干擾）。
5. 執行 `<dir>\install-winpoc.ps1 -Install <轉發參數>`；傳回其 exit code。

**錯誤處理：** 無網路／API rate limit／找不到 release → 明確訊息並提示 `-ZipPath`；雜湊不符 → 中止；installer 失敗 → 原樣傳出訊息與 exit code。

## 7. Browser（Playwright）

依作者 Linux 安裝器（`scripts/install-linux.sh`：「installs @playwright/mcp if node is present」、`--no-browser`）：

1. `-NoBrowser` 或找不到 `node`/`npm` → 跳過，摘要顯示 `browser: skipped (install Node.js, rerun setup)`。
2. `npm install --prefix %LOCALAPPDATA%\oab-imcp-winpoc\pw-mcp --save-exact @playwright/mcp@0.0.82`（與 macOS `poc/pw-mcp/install.sh`、Linux `docs/linux-setup.md` 同一個 pin）。
3. `npx --prefix ... playwright install chromium`（同 macOS；Windows 無 distro Chromium）。
4. 複製 `pw-mcp.ps1` 到 `$ProgDir`；註冊 Scheduled Task `\OpenAB-POC\oab-imcp-winpoc-pw`（§13.2 名稱），設定同 node Task，執行 `powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File pw-mcp.ps1`。
5. node Task 參數加 `--upstream browser=http://127.0.0.1:8797/mcp`。
6. 任一步失敗 → 警告、不加 `--upstream`、不中止整體安裝（核心工具優先）。

Uninstall：停止並移除 pw Task；`-Purge` 時連同 `pw-mcp\`、`pw-profile\`、`pw-output\` 刪除。

## 8. Claude Code 註冊

作者的做法：安裝器不碰任何 client 設定；README 提供一行 `kiro-cli mcp add --name <host>-mcp --url <url> --header "Authorization: Bearer $(cat token)" --scope global`，token 最終也以明文存在 client 設定。本功能（Q1）把同一行自動化，行為與作者文件一致：

- 預設執行；`-NoClaudeCode` 關閉。`Get-Command claude` 不存在 → 跳過，印出可貼上的指令。
- 名稱：`oab-imcp-winpoc`（§13.2 POC 名稱；與使用者既有的 MCP 項目不衝突，uninstall 可安全移除）。
- Scope：`user`（對應作者範例的 `--scope global`）。
- URL：**loopback** `http://127.0.0.1:8796/mcp`。client 與 node 同機，不經 Tailscale；loopback + bearer 已由 `windows/smoke_install.sh`（「serves 9 tools with the bearer token」）驗證。
- 冪等：先 `claude mcp remove oab-imcp-winpoc -s user`（忽略錯誤），再 `claude mcp add --transport http -s user oab-imcp-winpoc http://127.0.0.1:8796/mcp --header "Authorization: Bearer <token>"`（Claude Code 2.1.296 `claude mcp add --help` 已確認語法）。
- 驗證：`claude mcp get oab-imcp-winpoc` 成功即視為完成；失敗只警告，不中止安裝。
- `-AllowLogin` 模式（無 token）：不自動註冊，印說明（loopback 沒有 `Tailscale-User-Login`）。
- Uninstall：`claude mcp remove oab-imcp-winpoc -s user`。

**安全注意：** token 以明文寫入 `~/.claude.json`，與作者 README 的 client 範例相同。摘要提示此點；token 只在 loopback／tailnet 有效。

## 9. 安裝指南與 AI 安裝指引

放 `docs/windows-setup.md`，結構對應作者的 `docs/linux-setup.md`；repo 根 `README.md` Quick start 只加短段落連過去（同 macOS／Linux 的寫法）。

AI 安裝指引一節，與雙擊流程走同一路徑：

1. 確認 Windows、PowerShell 5.1+ 可用。
2. 在 repo 根目錄執行 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\setup-windows.ps1`。
3. 驗證：`Invoke-WebRequest http://127.0.0.1:8796/healthz` 回 `ok`；`claude mcp list` 含 `oab-imcp-winpoc` 且 Connected。
4. 常見失敗與對策表（無 release → `-ZipPath`；SmartScreen；Node 缺 → browser skipped；`claude` 不在 PATH）。

## 10. release-windows-poc.yml（fork 專用）

- 觸發：`push: tags: ['winpoc-v*']`、`workflow_dispatch`（input `tag`）。fork 的 Actions 已啟用（`gh api repos/pro5251/instance-mcp/actions/permissions` → `enabled: true`）。
- `permissions: contents: write`；`runs-on: windows-latest`；`RUSTFLAGS: -C target-feature=+crt-static`。
- 步驟：checkout tag → rust stable → `cargo build --release`（`poc/reverse-attach-linux`）→ 打包（沿用 `release.yml` windows job 的內容與檔名規則，`<ver>` = tag 去掉 `winpoc-v`）→ `gh release create <tag> --prerelease --title ... <zip> <zip.sha256>`（已存在則 `gh release upload --clobber`）。
- Actions 版本 pin SHA，與 `release.yml` 一致。
- **不設 `environment`**：作者的 `release` environment 只放行 `v*` ref，`winpoc-v*` 不符；fork 只有擁有者能推 tag，不需要審批閘門。
- **不送 upstream**：upstream 的 Windows 發行走 `release.yml` 既有 windows job（隨 `v*` 與 macOS／Linux 一起發）。送 PR 時排除此檔。

## 11. 測試

- **單元／靜態：** `setup-windows.ps1` 的 release 選擇與雜湊比對拆成函式，以假 release JSON 測。
- **整合：** 擴充 `windows/smoke_install.sh`：用 `-ZipPath`（本機打包的 ZIP）跑 `setup-windows.ps1 -NoStart`；檢查 Task、pw Task（有 node 時）、`--upstream` 參數、Claude Code 註冊（PATH 放 stub `claude.cmd` 記錄參數，不改真實設定）。
- **Uninstall 對稱：** 安裝 → 移除 → Task、pw Task、Claude Code 註冊皆不存在。
- **回歸門檻：** §3 四項不退步（本變更不碰 Rust，預期不受影響）。
- **手動：** 乾淨 Windows 帳號從 clone 雙擊 `scripts\setup-windows.cmd` 一次走完；加入 `MANUAL-ACCEPTANCE.md`。
- **Release：** 在 fork 推 `winpoc-v0.0.1-rc1` 驗證 workflow，同時補上 G2「MSVC 版從未編過」的缺口。

## 12. 決策紀錄（2026-10-14，依作者做法定案）

upstream `openabdev/instance-mcp` main 為 `f456d04`（0.8.0），與本機一致，無需 pull。

| # | 問題 | 決定 | 依據 |
|---|---|---|---|
| D1 | Claude Code 註冊細節 | 名稱 `oab-imcp-winpoc`、user scope、loopback URL、明文 token、預設開＋`-NoClaudeCode` | 作者 README client 範例（`mcp add … --header Bearer … --scope global`）；loopback bearer 已有 smoke 驗證 |
| D2 | AI 安裝指引放哪 | `docs/windows-setup.md`；`README.md` Quick start 加短段落 | 作者 `docs/linux-setup.md` ＋ README Quick start 結構 |
| D3 | setup 腳本放哪 | `scripts/setup-windows.{cmd,ps1}` | 作者安裝腳本都在 `scripts/`（`install-linux.sh`、`install-prebuilt.sh`）；根目錄沒有平台入口 |
| D4 | 預設 repo | 腳本開頭變數 `pro5251/instance-mcp`，`-Repo` 可覆寫 | §13.2「名稱集中一處」原則 |
| D5 | Release 審批閘門 | 不設 environment；workflow 僅限 fork | 作者 `release` environment 只放行 `v*`；upstream 走 `release.yml` |
| D6 | `@playwright/mcp` 版本 | `0.0.82`，`--save-exact` | 作者 macOS／Linux 同一 pin |
| D7 | exe 內建 `install` 子指令 | 不開 ticket，列非目標 | 作者各平台都用原生安裝形式（pkg／shell script）；Windows 用 PowerShell 腳本已對齊；MSI/winget 屬作者的發行決定（同 code signing） |
| D8 | 旗標命名 | `-NoBrowser`（原提案 `-SkipPlaywright`）、`-NoClaudeCode` | 作者 `install-linux.sh` 的 `--no-browser` |

**送 upstream PR 時注意：** 作者 PR #51 要求社群 PR 附 #openab-pty Discord 討論連結；`release-windows-poc.yml` 與 `-Repo` 預設值要調整（D4、D5）。

## 13. 附帶發現（與本功能無關）

工作樹 21 個 `.sh`/`.py` 只有檔案模式差異（Windows checkout 造成）。2026-10-08 已設定本 repo `core.fileMode false`，未 commit 任何模式變更。
