# Windows 一步安裝 + Claude Code 自動註冊（spec v0.1 草稿）

> Recorded 2026-10-08. Draft; to be revised.

**Status:** needs-triage（草稿：§1–§4 為使用者已決定事項；§5 起為提案，尚未逐段確認，改天再調整）

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
| G2 | fork `pro5251/instance-mcp` 沒有任何 Release；`release.yml` 的 `windows` job 等 macOS job 建 release（`:156`），fork 無 Apple secrets（`:32-40`）→ Windows ZIP 永遠發不出來 | `.github/workflows/release.yml` |
| G3 | `browser_*` 在 Windows 沒接通：安裝腳本不做 `npm install @playwright/mcp`／`npx playwright install chromium`（與 `pw-mcp.ps1:3-4` 註解不符）；無 Task 啟動 `pw-mcp.ps1`；node Task 沒帶 `--upstream` | `install-winpoc.ps1`、`pw-mcp.ps1` |
| G4 | `windows/README.md` 宣稱「node picks up `pw-mcp.ps1` automatically」，與實作不符 | `windows/README.md` |
| G5 | 安裝只印 URL/token，不設定任何 MCP client | `install-winpoc.ps1:93-98` |

背景補充：Playwright browser upstream 是作者既有功能（macOS `poc/pw-mcp/` 0.1.0、Linux #29 以 `MCP_UPSTREAM` env）。Windows 用 `--upstream` 旗標因 Scheduled Task 不繼承環境變數（`install-winpoc.ps1:56-57`）。

## 2. 目標

**誰：** 拿到這個 repo（clone，不是 ZIP）的 Windows 使用者／測試者；不假設有 Rust、MSVC、gh。

**成功標準：**
1. 雙擊 repo 根目錄一個檔案（或請 AI 照 README 指引執行），無需 admin。
2. 結束時：node 健康（`/healthz` = ok）、開機自動啟動、token 已產生並保留。
3. Claude Code 已自動註冊此 node，`claude mcp list` 可見且連線成功。
4. 有 Node.js 時 `browser_*` 可用；沒有時其餘工具照常，印提示。
5. 可重複執行（更新）與可完整移除。

**非目標：** exe 自身安裝子指令（問題 4 選項 B，延後）；Claude Desktop／OpenAB Connect 註冊；code signing；自動安裝 Rust/MSVC 或 Node.js。

## 3. 硬性限制（CLAUDE.md）

- 不動 macOS（Swift）與 Linux 行為；**不改作者的 `release.yml`**、不改 Rust 程式碼。
- 名稱依 `.scratch/windows-poc/spec.md` §13.2 POC 名稱。
- 回歸門檻不退步：`cargo fmt --check`、`cargo clippy --release -- -D warnings`、`cargo test --release`（18 passed）、`smoke.sh`（48/48）。
- 不經詢問不 push；功能分支開發。

## 4. 已決定事項

| 問題 | 決定 |
|---|---|
| Q1 註冊哪個 MCP client | **Claude Code** only |
| Q2 exe 來源 | **從 GitHub Release 下載**（fork `pro5251/instance-mcp`），sha256 驗證 |
| Q3 Release 怎麼產生 | **新增獨立 workflow** `release-windows-poc.yml`，`winpoc-v*` tag 或 `workflow_dispatch` 觸發，自己 `gh release create`；不動 `release.yml` |
| Q4 安裝入口 | **repo 根目錄 `setup-windows.cmd`** + README 的 AI 安裝指引；只改 PowerShell/cmd |
| Q5 Playwright | **一起修；偵測到 node/npm 才裝**，否則跳過並提示 |

補充（回覆使用者疑問）：Release workflow 只在推 `winpoc-v*` tag 或手動觸發時跑，一般 push 不跑。使用者拿到的是最新 Release；安裝一律用 ZIP 內的 `install-winpoc.ps1`，確保 exe 與安裝腳本同版。

---

## 5. 架構（提案，第 1 段已呈現、未確認）

```
[repo] setup-windows.cmd ──► setup-windows.ps1 (bootstrap, repo 內)
          │ 1. 查 pro5251/instance-mcp 最新 winpoc-v* release（GitHub REST API，免 gh）
          │ 2. 下載 ZIP + .sha256 → 比對 → 解壓到 %TEMP%\oab-imcp-winpoc-<ver>\
          ▼
   ZIP 內 install-winpoc.ps1 -Install -RegisterClaudeCode
          │ a. 既有：exe、token、Scheduled Task、/healthz、tailscale serve
          │ b. 新：Playwright（§7）
          │ c. 新：Claude Code 註冊（§8）
          ▼
   印 ready 摘要（URL、token 位置、browser 狀態、Claude Code 狀態）
```

### 5.1 元件

| 檔案 | 新/改 | 職責 |
|---|---|---|
| `setup-windows.cmd`（repo 根） | 新 | 雙擊入口；`powershell -NoProfile -ExecutionPolicy Bypass -File setup-windows.ps1 %*`；結尾 `pause` |
| `setup-windows.ps1`（repo 根） | 新 | 只負責「取得正確版本 ZIP → 驗證 → 解壓 → 呼叫 ZIP 內 installer」；不含安裝邏輯 |
| `.github/workflows/release-windows-poc.yml` | 新 | 編譯、打包、建 release、上傳 |
| `poc/reverse-attach-linux/windows/install-winpoc.ps1` | 改 | 加 Playwright 段、`-RegisterClaudeCode` 段，uninstall 對稱 |
| `poc/reverse-attach-linux/windows/pw-mcp.ps1` | 改（可能） | 對齊安裝路徑；修正註解 |
| `poc/reverse-attach-linux/windows/README.md` | 改 | 修 G4；加從 repo 安裝的說明 |
| `README.md`（repo 根）或 `docs/windows-setup.md` | 改/新 | Windows quick start + AI 安裝指引（§9）。放哪裡待定（見 §12） |

## 6. setup-windows.ps1（提案）

**參數：**
- `-Version <tag>`：鎖定版本（預設最新 `winpoc-v*`）
- `-ZipPath <path>`：用本機 ZIP，跳過下載（離線／測試／CI）
- `-Repo <owner/name>`：預設 `pro5251/instance-mcp`
- 其餘參數原樣轉給 `install-winpoc.ps1`（`-Token`、`-AllowLogin`、`-SkipTailscale`、`-NoStart`、`-SkipPlaywright`、`-SkipClaudeCode`）

**流程：**
1. `GET https://api.github.com/repos/<repo>/releases?per_page=30` → 過濾 `tag_name -like 'winpoc-v*'` 且非 draft → 取最新（或 `-Version` 指定）。
2. 找資產 `oab-imcp-winpoc-<ver>-windows-amd64.zip` 與 `.zip.sha256`。
3. 下載到 `%TEMP%\oab-imcp-winpoc-<ver>\`；`Get-FileHash -Algorithm SHA256` 比對，不符即中止並刪檔。
4. `Expand-Archive`；`Unblock-File` 解出的檔案（移除 MOTW，降低 SmartScreen 干擾）。
5. 執行 `<dir>\install-winpoc.ps1 -Install -RegisterClaudeCode <轉發參數>`；傳回其 exit code。

**錯誤處理：** 無網路／API rate limit／找不到 release → 明確訊息並提示 `-ZipPath`；雜湊不符 → 中止；installer 失敗 → 原樣傳出訊息與 exit code。

## 7. Playwright（提案）

在 `Do-Install` 健康檢查之前：

1. `-SkipPlaywright` 或找不到 `node`/`npm` → 跳過，摘要顯示 `browser: skipped (install Node.js, rerun setup)`。
2. `npm install --prefix %LOCALAPPDATA%\oab-imcp-winpoc\pw-mcp @playwright/mcp@<固定版本>`（版本與 ticket 11 一致）。
3. `npx --prefix ... playwright install chromium`。
4. 複製 `pw-mcp.ps1` 到 `$ProgDir`；註冊 Scheduled Task `\OpenAB-POC\oab-imcp-winpoc-pw`（§13.2 名稱），設定同 node Task，執行 `powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File pw-mcp.ps1`。
5. node Task 參數加 `--upstream browser=http://127.0.0.1:8797/mcp`。
6. 任一步失敗 → 警告、不加 `--upstream`、不中止整體安裝（核心工具優先）。

Uninstall：停止並移除 pw Task；`-Purge` 時連同 `pw-mcp\`、`pw-profile\`、`pw-output\` 刪除。

## 8. Claude Code 註冊（提案，尚未呈現給使用者確認）

- 條件：`-RegisterClaudeCode` 且 `Get-Command claude` 存在；否則印出可貼上的指令。
- 名稱：`oab-imcp-winpoc`（與 serverInfo.name 一致）。
- Scope：`user`。
- URL：**loopback** `http://127.0.0.1:8796/mcp`（本機 client 不經 Tailscale）。
- 冪等：先 `claude mcp remove oab-imcp-winpoc -s user`（忽略錯誤），再
  `claude mcp add --transport http -s user oab-imcp-winpoc http://127.0.0.1:8796/mcp --header "Authorization: Bearer <token>"`。
- 驗證：`claude mcp get oab-imcp-winpoc` 成功即視為完成。
- `-AllowLogin` 模式（無 token）：不自動註冊，印說明。
- Uninstall：`claude mcp remove oab-imcp-winpoc -s user`。

**安全注意：** token 會以明文寫入 Claude Code 使用者設定（`~/.claude.json`）。摘要需提示此點；token 本身已只綁 loopback/tailscale。

## 9. AI 安裝指引（提案）

README 中一段給 AI agent 照做的步驟，與雙擊流程走同一路徑：

1. 確認 Windows、PowerShell 5.1+ 可用。
2. 在 repo 根目錄執行 `powershell -NoProfile -ExecutionPolicy Bypass -File .\setup-windows.ps1`。
3. 驗證：`Invoke-WebRequest http://127.0.0.1:8796/healthz` 回 `ok`；`claude mcp list` 含 `oab-imcp-winpoc` 且 Connected。
4. 常見失敗與對策表（無 release → `-ZipPath`；SmartScreen；Node 缺 → browser skipped；`claude` 不在 PATH）。

## 10. release-windows-poc.yml（提案）

- 觸發：`push: tags: ['winpoc-v*']`、`workflow_dispatch`（input `tag`）。
- `permissions: contents: write`；`runs-on: windows-latest`；`RUSTFLAGS: -C target-feature=+crt-static`。
- 步驟：checkout tag → rust stable → `cargo build --release`（`poc/reverse-attach-linux`）→ 打包（沿用 `release.yml:257-268` 的內容與檔名規則，`<ver>` = tag 去掉 `winpoc-v`）→ `gh release create <tag> --prerelease --title ... <zip> <zip.sha256>`（已存在則 `gh release upload --clobber`）。
- Actions 版本 pin SHA，與 `release.yml` 一致。
- 不使用 `environment: release`（fork 無此環境）；待確認是否需要審批閘門。

## 11. 測試

- **單元／靜態：** `setup-windows.ps1` 的 release 選擇與雜湊比對拆成函式，用 Pester 或 smoke 腳本以假 release JSON 測。
- **整合：** 擴充 `windows/smoke_install.sh`：用 `-ZipPath`（本機打包的 ZIP）跑 `setup-windows.ps1 -NoStart`；檢查 Task、pw Task（有 node 時）、`--upstream` 參數、`claude mcp get`（有 claude 時，或以 stub `claude.cmd` 置於 PATH 驗參數）。
- **Uninstall 對稱：** 安裝 → 移除 → Task、pw Task、Claude Code 註冊皆不存在。
- **回歸門檻：** §3 四項不退步（本變更不碰 Rust，預期不受影響）。
- **手動：** 乾淨 Windows 帳號從 clone 雙擊 `setup-windows.cmd` 一次走完；加入 `MANUAL-ACCEPTANCE.md`。
- **Release：** 在 fork 推 `winpoc-v0.0.1-rc1` 驗證 workflow。

## 12. 未決問題

1. §8 Claude Code 註冊細節（名稱、loopback vs tailscale URL、token 明文風險）尚未與使用者確認。
2. AI 安裝指引放 repo 根 `README.md`（作者的檔案，送 upstream 時有爭議）還是 `docs/windows-setup.md`／`windows/README.md`？
3. `setup-windows.*` 放 repo 根目錄是否符合作者慣例（根目錄目前無平台專屬入口）；替代：`scripts/setup-windows.*`。
4. 預設 repo 寫死 `pro5251/instance-mcp`；若日後併入 upstream 應改為 `openabdev/instance-mcp`——是否參數化即可？
5. Release workflow 是否需要審批閘門（environment）。
6. `@playwright/mcp` 固定版本號（沿用 ticket 11）。
7. 問題 4 選項 B（exe 內建 `install` 子指令）是否列為後續 ticket。

## 13. 附帶發現（與本功能無關）

工作樹 21 個 `.sh`/`.py` 顯示 `M` 但 0 insertions/0 deletions，只有檔案模式（執行權限）不同，是 Windows checkout 造成的。誤 commit 會拿掉作者的執行權限。建議 `git config core.fileMode false`。
