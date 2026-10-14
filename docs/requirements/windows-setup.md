# Windows 一步安裝 + MCP client 自動註冊（spec v0.5）

> v0.5 2026-10-14: Q3 reversed - the fork releases through release.yml itself (v* tags), with a
> fork-only stand-in for the macOS job, to exercise the real merge-back path (ticket 09).
> v0.4 2026-10-14: Q1 widened from Claude Code only to five local MCP clients (ticket 08).
> Recorded 2026-10-08; decisions settled 2026-10-14 against the author's conventions; rewritten
> to the to-spec template 2026-10-14.

**Status:** ready-for-agent

**相關：** Windows POC spec v0.4（本機 `.scratch/windows-poc/spec.md`，名稱依其 §13.2 POC 名稱）、POC tickets 11（Playwright upstream）、14（packaging）、15（手動驗收）

---

## Problem Statement

拿到這個 repo 的 Windows 使用者，沒辦法把 hands node 裝起來，也沒辦法讓本機的 AI client 直接用它。

- 在 repo 裡雙擊 Windows 安裝入口會直接失敗：「missing … oab-imcp-winpoc.exe beside this script」。安裝程式要求 exe 在旁邊，但 exe 只存在於 release ZIP，repo 裡沒有。
- 想要 release ZIP 也拿不到：fork 從來沒有發過 Release。既有的 release workflow 中，Windows job 要等 macOS job 建好 Release，但 macOS job 需要 fork 沒有的 Apple 簽章 secrets，所以 Windows ZIP 永遠發不出來。正式發行用的 MSVC 版本也因此從來沒有編過。
- 就算裝好了，瀏覽器工具（`browser_*`）也接不起來：安裝程式不會安裝 Playwright MCP，不會啟動它，也不會讓 node 轉發給它；但 Windows README 寫的是「會自動接上」。
- 裝好之後，使用者還要自己抄 URL 和 token，手動設定 Claude Code、Codex、GitHub Copilot、Gemini 等本機 AI client。
- 使用者不一定有 Rust、MSVC 或 gh，也不想研究這些。

## Solution

使用者 clone repo 之後，雙擊一個 setup 入口就好（或請 AI 照安裝指南執行）。setup 會：

1. 從 fork 的 GitHub Release 下載最新的 Windows POC ZIP，比對 sha256 後解壓。
2. 執行 ZIP 裡的安裝程式，保證 exe 和安裝程式是同一版。安裝程式會：裝好 node、開機自動啟動、產生或保留 token、確認健康狀態、有 Tailscale 時設定 `tailscale serve`。
3. 偵測到 Node.js 時，安裝固定版本的 Playwright MCP，用獨立的開機工作啟動它，並讓 node 把 `browser_*` 轉發過去；沒有 Node.js 就跳過並印出提示。
4. 對偵測到的每個本機 MCP client（Claude Code、Codex CLI、GitHub Copilot CLI、VS Code（Copilot Chat，含 Insiders）、Gemini CLI），自動把這台 node 註冊成 user 層級的 MCP server（本機 loopback URL 加 bearer token）；Claude Code 找不到時印出可以直接貼上的指令。
5. 最後印出摘要：URL、token 位置、browser 狀態、每個 client 的註冊狀態與 token 存放位置。

Release 由 `release.yml` 產生，也就是日後併回 upstream 的同一個 workflow：推 `v*` tag 時，Linux 與 Windows job 照原樣建置並上傳。fork 沒有 Apple 簽章 secrets，所以在 fork 上 macOS job 會跳過，改由一個只在 fork 執行的小 job 建立 prerelease；upstream 的行為不變。

## User Stories

1. 身為 clone 了 repo 的 Windows 使用者，我想雙擊一個檔案就裝好 hands node，這樣不用了解 Rust、MSVC 或 release 流程。
2. 身為 Windows 使用者，我想要不需要系統管理員權限就能完成安裝，這樣在公司電腦上也能用。
3. 身為 Windows 使用者，我想要安裝程式自動抓最新版本，這樣不用自己去 Releases 頁面找檔案。
4. 身為 Windows 使用者，我想要能指定安裝某個版本，這樣新版有問題時可以退回舊版。
5. 身為 Windows 使用者，我想要安裝程式在下載後先驗證 sha256，這樣被竄改或下載不完整的檔案不會被安裝。
6. 身為 Windows 使用者，我想要雜湊不符時安裝立刻中止並刪掉下載的檔案，這樣不會留下可疑的檔案。
7. 身為沒有網路，或碰到 GitHub API rate limit 的使用者，我想要能指定一個本機的 ZIP 來安裝，這樣離線也能裝。
8. 身為 Windows 使用者，我想要找不到 Release 時看到清楚的錯誤訊息和下一步建議，這樣不會卡在看不懂的 PowerShell 例外。
9. 身為 Windows 使用者，我想要下載的檔案被移除「來自網際網路」標記，這樣 SmartScreen 比較不會擋。
10. 身為 Windows 使用者，我想要 node 每次登入都自動啟動，這樣重開機後還能用。
11. 身為 Windows 使用者，我想要重新安裝時保留原本的 token，這樣已經設定好的 client 不會失效。
12. 身為 Windows 使用者，我想要安裝完成時看到 node 已經是健康狀態，這樣能確定裝好了。
13. 身為有裝 Node.js 的使用者，我想要瀏覽器工具自動設定好，這樣 agent 可以操作瀏覽器。
14. 身為沒有裝 Node.js 的使用者，我想要其他工具照常可用，並看到「裝 Node.js 後重跑 setup」的提示，這樣不會因為缺少選用元件而整個安裝失敗。
15. 身為使用者，我想要用 `-NoBrowser` 關掉瀏覽器工具，這樣不需要時可以省下 Chromium 的下載。
16. 身為使用者，我想要 Playwright 安裝失敗時只看到警告，核心工具仍然裝好，這樣主要功能不受選用功能影響。
17. 身為使用者，我想要 Playwright 固定在跟 macOS、Linux 相同的版本，這樣三個平台的 `browser_*` 行為一致。
18. 身為 Claude Code 使用者，我想要安裝完 Claude Code 就已經註冊好這台 node，這樣可以直接呼叫工具。
19. 身為 Claude Code 使用者，我想要註冊在 user scope，這樣在任何專案目錄都能用。
20. 身為 Claude Code 使用者，我想要重複安裝不會產生重複或衝突的註冊項目，這樣設定保持乾淨。
21. 身為 Claude Code 使用者，我想要註冊名稱是 POC 專用名稱，這樣不會覆蓋我自己的 MCP 設定，日後的正式版也不會跟它撞名。
22. 身為沒有 Claude Code 的使用者，我想要看到一行可以直接貼上的註冊指令，這樣之後裝了 Claude Code 也能自己加。
23. 身為使用者，我想要用 `-NoClaudeCode` 跳過註冊，這樣我可以自己管理 client 設定。
24. 身為用 Tailscale 身分登入（沒有 token）的使用者，我想要安裝程式跳過 Claude Code 註冊並告訴我原因，這樣不會註冊一個連不上的項目。
25. 身為使用者，我想要摘要提醒我 token 是以明文存在 Claude Code 的設定裡，這樣我知道它的安全邊界。
26. 身為使用者，我想要解除安裝時一併移除 node、Playwright 工作和 Claude Code 註冊，這樣電腦上不會留下殘骸。
27. 身為使用者，我想要用 `-Purge` 連 token、grants 和瀏覽器 profile 一起刪掉，這樣可以完全清除。
28. 身為使用者，我想要重新執行 setup 就能更新到新版，這樣更新跟安裝是同一個步驟。
29. 身為請 AI 代為安裝的使用者，我想要 repo 裡有一份 AI 照做就能完成安裝的指南，裡面包含驗證步驟和常見錯誤的處理方式，這樣 AI 不用自己摸索。
30. 身為 AI agent，我想要安裝指南有明確的成功判斷（健康檢查回 ok、`claude mcp list` 出現 Connected），這樣我能確定安裝完成。
31. 身為 repo 維護者，我想要在 fork 推一個 `v*` tag（例如 `v0.8.0-winpoc.1`）就經由 `release.yml` 自動產生含 Windows ZIP 的 Release，這樣驗證的就是併回 upstream 後的真實流程。
32. 身為 repo 維護者，我想要一般 push 不會觸發 release，這樣只有我決定要發布時才會發。
33. 身為 repo 維護者，我想要 Release 是用 MSVC 加 static CRT 編的，這樣使用者不需要另外裝 VC runtime，正式發行用的建置方式也能被驗證到。
34. 身為 repo 維護者，我想要 `release.yml` 在 upstream 的行為（macOS 簽章後建立 Release、Linux／Windows 補上傳、approval 閘門）完全不變，fork 專用的部分只在 fork 生效，這樣不會影響作者的 macOS 和 Linux 發行。
35. 身為 repo 維護者，我想要預設的下載來源 repo 只在一個地方設定，這樣日後併入 upstream 時只要改一處。
36. 身為 repo 維護者，我想要 Windows README 跟實際行為一致，這樣不會誤導使用者。
37. 身為 repo 維護者，我想要 repo 首頁的 Quick start 有 Windows 的入口連結，這樣使用者找得到安裝方式。
38. 身為作者，我想要 Linux 和 macOS 的行為、介面、回歸測試全部不變，這樣 Windows 的工作不會破壞已經發行的平台。
39. 身為 Codex CLI 使用者，我想要安裝完就已經在 Codex 註冊好這台 node，token 透過 Codex 官方的 bearer token 環境變數提供，這樣不用改 TOML。
40. 身為 GitHub Copilot CLI 使用者，我想要安裝完就已經在 Copilot CLI 註冊好這台 node。
41. 身為 VS Code（Copilot Chat）使用者，我想要安裝完 VS Code（或 Insiders）的使用者 `mcp.json` 就有這台 node，而且我自己的其他設定保持原樣。
42. 身為 Gemini CLI 使用者，我想要安裝完就已經在 Gemini 註冊好這台 node（user scope）。
43. 身為使用者，我想要用 `-Clients` 指定只註冊哪些 client、用 `-NoClients` 全部跳過，這樣我可以自己決定要動哪些設定。
44. 身為使用者，我想要摘要逐一列出每個 client 的狀態和 token 的存放位置，這樣我知道 token 在哪些檔案裡。
45. 身為使用者，我想要解除安裝時從每個 client 移除註冊，Codex 的 token 環境變數也一併刪除。

## Implementation Decisions

**模組**

- **Setup bootstrap（新增，放在 repo 的 scripts 目錄）**：有一個雙擊入口和一個 PowerShell bootstrap。bootstrap 只負責四件事：找到版本 → 下載 → 驗證 → 解壓，然後把剩下的參數原封不動交給 ZIP 裡的安裝程式。它本身不包含任何安裝邏輯。
  - 介面：`-Version <tag>`、`-ZipPath <path>`、`-Repo <owner/name>`、`-ApiBase <url>`（測試用，預設是 GitHub API），其餘參數直接轉給安裝程式。
  - 預設的 repo 寫在腳本開頭的變數區，POC 期間是 fork，併入 upstream 時改成 `openabdev/instance-mcp`。
  - 怎麼挑版本：從 releases 清單裡過濾 `v*`、不是 draft、而且附有 `oab-imcp-winpoc-<ver>-windows-amd64.zip` 與其 `.sha256`（`<ver>` = tag 去掉 `v`）的，依發布時間取最新的一個；有指定 `-Version` 就用指定的。沒有 Windows ZIP 的 release（例如作者較早的版本）一律略過。
  - 資產名稱照 POC 命名規則：`oab-imcp-winpoc-<ver>-windows-amd64.zip` 加上 `.sha256`。
  - 安裝程式的 exit code 原樣傳回。
- **Windows 安裝程式（修改現有的 install-winpoc）**：
  - 新增 browser 段和 MCP client 段，兩段都預設開啟。browser 用 `-NoBrowser` 關閉；client 用 `-NoClients` 全關、`-Clients claude,codex,copilot,vscode,gemini` 指定，`-NoClaudeCode` 保留為「排除 claude」。命名對照作者 Linux 安裝程式的 `--no-browser`。
  - 解除安裝要對稱：移除 Playwright 工作和每個 client 的註冊。`-Purge` 時再刪掉 Playwright 的安裝目錄、瀏覽器 profile 和輸出目錄。
  - ZIP 裡原本的雙擊入口照舊走同一個安裝程式。
- **Browser 段**：
  - 找不到 node 或 npm 就跳過。
  - 用 `--save-exact` 把 `@playwright/mcp@0.0.82` 裝到 POC 資料目錄底下，再安裝 Playwright 的 Chromium。版本跟 macOS、Linux 用同一個。
  - 註冊 POC 名稱的 Playwright Scheduled Task，設定和 node 的 Task 相同，以隱藏視窗的 PowerShell 執行 Playwright 啟動腳本。
  - node 的 Task 參數加上 `--upstream browser=http://127.0.0.1:8797/mcp`。用參數而不用環境變數，是因為 Scheduled Task 不會繼承環境變數。
  - 任何一步失敗都只發出警告：不加 `--upstream`，也不中止整個安裝。
- **Claude Code 段**：
  - 這一段是把作者 README 裡的 client 範例（`mcp add … --header "Authorization: Bearer …" --scope global`）自動化。
  - 名稱用 POC 名稱 `oab-imcp-winpoc`，scope 是 `user`，URL 用本機 loopback `http://127.0.0.1:8796/mcp`，header 帶 bearer token。
  - 為了可以重複執行：先移除同名項目（忽略錯誤），再新增，最後用 get 確認。驗證失敗只警告。
  - 找不到 `claude` 時，印出可以直接貼上的指令。
  - `-AllowLogin` 模式下沒有 token，所以不註冊並說明原因：走 loopback 時不會帶 Tailscale 身分。
- **其他 MCP client（v0.4，ticket 08）**：一張 client 表（偵測、加入、移除、確認），安裝與解除安裝都跑同一張表，名稱同樣用 `oab-imcp-winpoc`、URL 同樣是 loopback。任何 client 失敗都只警告。確認 = `mcp get`／`mcp list` 的輸出裡有這個名稱。解除安裝一律清除每個 client（不看 `-Clients`；`-NoClients` 才全部跳過），VS Code 的 `mcp.json` 即使 `code` 已不在 PATH 也會清。
  - Codex CLI：`codex mcp remove` → `codex mcp add <name> --url <url> --bearer-token-env-var OAB_IMCP_WINPOC_TOKEN` → `codex mcp get`。token 寫進使用者環境變數 `OAB_IMCP_WINPOC_TOKEN`（Codex CLI 只支援從環境變數讀 bearer token）；已開的終端機要重開。解除安裝刪除該變數。
  - GitHub Copilot CLI：`copilot mcp remove` → `copilot mcp add --transport http <name> <url> --header "Authorization: Bearer …"` → `copilot mcp get`（`~\.copilot\mcp-config.json`）。
  - VS Code（Copilot Chat）與 Insiders：VS Code 沒有移除用的 CLI，`--add-mcp` 的 JSON 參數經 PowerShell 5.1 傳給 `.cmd` 會被破壞引號，所以直接編輯使用者 `mcp.json`（`%APPDATA%\Code\User\mcp.json`、`%APPDATA%\Code - Insiders\User\mcp.json`）的 `servers.<name>`（`type: http`、`url`、`headers`），其餘內容保留，第一次修改前留備份；檔案無法解析（例如有註解）就不動它，只警告並印出要貼的片段。偵測依據是 PATH 上有 `code` / `code-insiders`。
  - Gemini CLI：`gemini mcp remove -s user` → `gemini mcp add -s user --transport http --header "Authorization: Bearer …" <name> <url>` → `gemini mcp list` 確認（Gemini 沒有 `get`；`~\.gemini\settings.json`）。指令依官方文件，開發機上的 Gemini CLI 安裝損壞，未實機驗證。
- **Playwright 啟動腳本（修改）**：安裝路徑對齊安裝程式，修正跟實際行為不符的註解。
- **Release workflow（修改 `release.yml`，v0.5）**：
  - 觸發條件不變：`v*` tag 或 workflow_dispatch。fork 用帶後綴的 tag（例如 `v0.8.0-winpoc.1`），不會佔用作者的正式版號。
  - macOS job 加上 `if: github.repository_owner == 'openabdev'`：只在 upstream 執行。
  - 新增 `fork-release` job（`if: github.repository_owner != 'openabdev'`，ubuntu）：建立 prerelease，取代 macOS job「建立 Release」的角色。它和 macOS job 的條件互斥，所以任何 repo 都只有一個 job 會建立 Release。
  - Linux 與 Windows job 不改：照原本的設計等 Release 出現後再上傳，所以 fork 驗證到的就是 upstream 的真實流程。
  - Windows job 用 MSVC 加 static CRT 編譯（windows-latest 預設 MSVC，加 `RUSTFLAGS=+crt-static`），並新增檢查：exe 不得匯入 VC runtime DLL。
  - 刪除 v0.3 的 `winpoc-release.yml`。
  - 送 upstream PR 時：`fork-release` job 和 macOS 的 `if:` 可以保留（在 upstream 不會改變行為，也方便其他 fork 驗證），或依作者意見移除。
- **文件**：
  - 新增 Windows setup 指南，結構對照作者的 Linux setup 指南，裡面包含 AI 安裝一節。
  - repo 首頁 README 的 Quick start 只加一小段 Windows (POC) 連結。
  - Windows README 修正「自動接上 Playwright」這句錯誤說明，並指向新的指南。

**限制**

- 不改 Rust 程式碼，不改作者的 release workflow，不改 macOS 和 Linux 的任何行為。
- 所有對外名稱都用 POC 名稱，並集中在腳本開頭的變數區。
- 回歸測試門檻不可退步：cargo fmt、clippy -D warnings、cargo test（基準 18 passed）、smoke.sh（基準 48/48）。

## Testing Decisions

**什麼是好的測試**：只透過使用者看得到的入口來驗證外部行為，也就是執行一次 setup，然後檢查系統狀態、HTTP 回應和 client 設定。不測 PowerShell 的內部函式，不 mock 內部模組。

**測試切入點（只用一個）**：Setup bootstrap 的入口，配合 `-ApiBase` 指向本機的假 GitHub 伺服器。

- 假伺服器提供 releases JSON、ZIP 和 sha256，可以測到這些情境：挑最新、附有 Windows ZIP 的 `v*`（略過沒有 Windows ZIP 的 release）、`-Version` 指定版本、略過 draft、雜湊不符時中止並刪檔、找不到 release 時的錯誤訊息。
- 假的 client：在 PATH 最前面放 `claude`、`codex`、`copilot`、`gemini`、`code`、`code-insiders` stub，把收到的參數記到檔案；`APPDATA` 指向暫存目錄，讓 VS Code 的 `mcp.json` 寫在那裡。用它驗證每個 client 的 remove → add → 確認順序和參數，並確認不會改到真正的 client 設定。Codex 的使用者環境變數是唯一實際寫入的狀態，測試確認安裝後存在、解除安裝後消失。
- 每次測試執行一次 setup，接著從外部檢查：
  - 兩個 Scheduled Task 是否存在、設定是否正確
  - 有 Node.js 時，node Task 是否帶 `--upstream`
  - 健康檢查是否回 ok
  - `tools/list` 是否有 9 個工具，有 Node.js 時再加上 `browser_*`
  - Claude Code stub 的紀錄
- 解除安裝的對稱性：安裝後解除安裝，確認兩個 Task 和 Claude Code 註冊都不在了；`-Purge` 時資料目錄也要一起消失。
- `-ZipPath` 只做最基本的檢查，確認離線路徑可以裝起來。它會跳過下載和驗證，所以不當作主要的測試路徑。

**沿用的既有範例**：

- 擴充現有的 Windows install smoke 腳本：它從 WSL 驅動 Windows 端的安裝程式，用 curl 檢查健康狀態和 tools 數量，也會檢查 Task 的設定和 token 的 DACL。這次把它的呼叫對象往上移到 setup bootstrap。
- 假伺服器的寫法沿用現有的 Python mock，也就是 mock upstream 和 mock switchboard 的寫法。
- 照舊用 run-all-smokes 統一執行。

**切入點以外的部分**：

- Release workflow：在 fork 推一個 `v0.8.0-winpoc.1` 經由 `release.yml` 實際跑一次（`fork-release` 建立 prerelease，Linux 與 Windows job 上傳），再用 setup（不帶 `-ApiBase`）裝這個真的 Release。這就是端到端驗收，同時也驗證 MSVC 版本和併回 upstream 後的發布路徑。
- 回歸測試：cargo 四項加 smoke.sh 照舊執行。這次不改 Rust，預期結果不會變。
- 實機手動驗收：用乾淨帳號從 clone 雙擊一次走完、SmartScreen 的行為、`browser_navigate` 實際成功。這些都加進 POC 的手動驗收清單。

## Out of Scope

- 讓 exe 本身內建 `install` 子指令，或做 MSI / winget 套件（這些屬於作者的發行決定，跟 code signing 一樣）。
- 註冊 Claude Desktop、OpenAB Connect，或 v0.4 client 表以外的 MCP client（日後加一列即可）。
- Code signing。
- 自動安裝 Rust、MSVC 或 Node.js。
- 修改作者的 release workflow，或在 upstream 發行 Windows 版本。
- 更改 POC 名稱，或改用 §13.2 正式名稱（要等作者確認）。

## Further Notes

**決策紀錄（2026-10-14，參考作者做法定案；upstream main 為 `f456d04` 0.8.0，與本機一致）**

| # | 決定 | 依據 |
|---|---|---|
| Q1 | 只註冊 Claude Code → **v0.4：Claude Code、Codex CLI、GitHub Copilot CLI、VS Code（含 Insiders）、Gemini CLI，偵測到就註冊；Codex token 走使用者環境變數** | 使用者決定（2026-10-14 擴大） |
| Q2 | exe 從 GitHub Release 下載並驗證 sha256 | 使用者決定 |
| Q3 | 另外新增 fork 專用的 release workflow → **v0.5：改用 `release.yml` 本身，macOS job 只在 upstream 跑，fork 由 `fork-release` job 建立 prerelease** | 使用者決定（2026-10-14 改為驗證真實的併回路徑） |
| Q4 | 從 repo 內的 setup 入口開始，加上 AI 安裝指南 | 使用者決定 |
| Q5 | 偵測到 Node.js 才裝 Playwright | 使用者決定；作者 Linux 安裝程式也是有 node 才裝 |
| D1 | Claude Code 註冊：POC 名稱、user scope、loopback、明文 token、預設開啟 | 作者 README 的 client 範例；loopback 加 bearer 已有 smoke 驗證 |
| D2 | 安裝指南用獨立文件，README 只放連結 | 作者 Linux setup 指南加 README Quick start 的結構 |
| D3 | setup 入口放在 scripts 目錄 | 作者的安裝腳本都在 scripts |
| D4 | 預設 repo 集中設定在一處，可以覆寫 | 跟 POC 名稱一樣「集中在一處改」 |
| D5 | ~~release workflow 不設審批閘門、只給 fork 用~~ v0.5：沿用 `release.yml` 的 `release` environment，`fork-release` 也一樣；fork 沒有設定保護規則，所以不需要核准 | fork 上 environment 會自動建立、沒有保護規則 |
| D6 | `@playwright/mcp@0.0.82` 固定版本 | 作者 macOS 和 Linux 用同一個版本 |
| D7 | exe 不內建 install 子指令 | 作者各平台都用 pkg 或安裝腳本 |
| D8 | 旗標命名為 `-NoBrowser` 和 `-NoClaudeCode` | 作者 Linux 安裝程式的 `--no-browser` |
| T1 | 測試只用一個切入點：setup 加 `-ApiBase` 假伺服器加 Claude Code stub | 使用者同意，2026-10-14 |

**送 upstream PR 時**：作者在 PR #51 規定社群 PR 要附 #openab-pty Discord 的討論連結；`release.yml` 裡的 `fork-release` job 可保留或依作者意見移除；預設 repo 要改成 upstream。

**附帶發現**：Windows checkout 讓 21 個腳本出現只有檔案模式的差異。2026-10-08 已在這個 repo 設定 `core.fileMode false`，沒有 commit 任何模式變更。

**建議的 ticket 依賴順序**：release workflow → setup bootstrap（含假伺服器測試）→ browser 段和 Claude Code 段（兩者可以平行做）→ 文件和實機驗收。
