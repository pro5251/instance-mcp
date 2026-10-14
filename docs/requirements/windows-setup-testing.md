# Windows setup：發布路徑與測試策略（討論整理）

> 整理自 2026-10-14 的討論。搭配 spec [`windows-setup.md`](windows-setup.md)（v0.5）閱讀。
> VM 實際操作步驟與測試 prompt 在
> [`poc/reverse-attach-linux/windows/VM-ACCEPTANCE.md`](../../poc/reverse-attach-linux/windows/VM-ACCEPTANCE.md)。

## 1. 發布路徑：為什麼 fork 改走 `release.yml`

**背景**：upstream 的 `release.yml` 由 `v*` tag 觸發。macOS job 簽章、公證後建立 Release，Linux 與 Windows job 等 Release 出現後再補上傳。fork 沒有 Apple 簽章 secrets，所以 macOS job 一定失敗，其他 job 會空等 30 分鐘後失敗。

**v0.3 的做法**：另外開一個 fork 專用的 `winpoc-release.yml`（`winpoc-v*` tag），送 upstream PR 時排除。

**v0.5 的做法（目前）**：改用 `release.yml` 本身，驗證的就是併回 upstream 後的真實路徑。

| 修改 | 在 upstream | 在 fork |
|---|---|---|
| macOS job 加 `if: github.repository_owner == 'openabdev'` | 照常執行，建立 Release | 跳過 |
| 新增 `fork-release` job（條件相反） | 跳過 | 建立 prerelease |
| Linux、Windows job | 不變 | 不變，照原本的設計等 Release 再上傳 |
| Windows job 的 static CRT 檢查（`dumpbin`） | 執行 | 執行 |

- 與 upstream 的 `release.yml` 相比只有新增，原有內容 0 行刪除。
- fork 的 tag 用後綴（`v0.8.0-winpoc.1`），不佔用作者的正式版號。
- setup 改成抓「最新、附有 Windows ZIP 與 `.sha256` 的 `v*` release」，作者那些只有 macOS／Linux 的 Release 會被略過。

**在 fork 修改會不會影響作者？** 不會。fork 的 workflow、tag、Release 都只存在於 fork，`GITHUB_TOKEN` 也只能寫入 fork。只有兩個時間點要注意：

- **送 upstream PR 時**：`fork-release` 和 macOS 的 `if:` 在 upstream 不改變行為，要保留或移除由作者決定。
- **同步 upstream 時**：作者若修改 `release.yml`，fork 可能要自己解衝突。

**實際驗證（2026-10-14）**：`v0.8.0-winpoc.1`，run `38534602012`。`fork-release`、Linux amd64／arm64、Windows 全部 success，macOS 照設計 skipped，Release 有 6 個檔案。

## 2. 「發布」不等於「安裝」

| 步驟 | 做了什麼 | 這台電腦是否已安裝 |
|---|---|---|
| 推分支、推 tag、看 Actions | 在 GitHub 上產生 Release | 否 |
| 執行 `scripts\setup-winpoc.cmd` | 下載、驗證、安裝 node，註冊到偵測到的 AI client | 是 |
| 檢查（排程工作、`healthz`、`claude mcp list`） | 獨立確認 | — |

setup 自己也會做一部分檢查：等 `/healthz` 回 `ok`，並用各 client 的 `mcp get`／`mcp list` 確認註冊。步驟見 [`docs/windows-setup.md`](../windows-setup.md) 的 Release and install, end to end 一節。

**開發機實測（2026-10-14）**：兩個排程工作在執行、`healthz` 回 `ok`、`tools/list` 有 9 個本機工具加 32 個 `browser_*`、Claude Code `✔ Connected`。第一次 `claude mcp get` 曾出現一次 `tools fetch failed · Request timed out`，再跑一次就正常。Codex、Copilot CLI、VS Code Insiders 的註冊格式都被真的 CLI 接受；Gemini 因為開發機的安裝損壞而無法驗證。

## 3. 測試分層

| 層 | 內容 | 狀態 |
|---|---|---|
| 1. Linux 回歸 | cargo fmt、clippy、test、`smoke.sh` | 每次修改都跑 |
| 2. 假 GitHub smoke | `smoke_setup.sh`（26 項）、`smoke_install.sh`（76 項），經 `-ApiBase` 指向 `mock_github.py`，各 client 用 stub | 每次修改都跑 |
| 3. 真 Release 發布 | 推 `v*` tag，由 `release.yml` 產出 | 已驗證一次 |
| 4. 真機安裝 | 開發機從真 Release 安裝 | 已驗證一次（開發帳號） |
| 5. 乾淨環境驗收 | 見下一節 | 待做 |

## 4. 剩下的驗收項目可以怎麼做

| 方式 | 能自動驗證的 | 驗不到的 | 成本 |
|---|---|---|---|
| **A. GitHub Actions Windows runner** | 全新機器從真 Release 安裝；5 個 client 用真的 CLI 註冊（含 Gemini）；`healthz`、`tools/list`、`browser_navigate`；解除安裝 | 沒裝 VC++ redistributable、一般使用者權限、真正的登出再登入 | 公開 repo 免費 |
| **B. Hyper-V VM** | 一般使用者帳號、沒有 VC++ redistributable、登出再登入自動啟動、Gemini 實機註冊；可用 checkpoint 重複測 | SmartScreen 的畫面提示要人看 | 自己的機器 |
| **C. Windows Sandbox** | 用完即丟的乾淨桌面 | 不能登出、執行帳號是系統管理員 | 開發機未啟用，啟用需要系統管理員權限 |

**A 的準確度**：驗證「安裝流程」很準，代表「一般使用者電腦」不準。

- **準的部分**：走真 Release、真的 sha256、真的 CLI，每次都是全新機器。
- **不準的部分**：
  - runner 是 Windows Server，不是桌面版
  - 執行帳號是系統管理員
  - 預裝了大量工具，包含 Node、Python、Git、VC++ redistributable
  - 能不能啟動「互動式登入」的排程工作和有畫面的 Chromium，要實際跑過才知道
  - client 都沒登入
  - 沒有公司 proxy

**A 的費用**：fork 是公開 repo，標準 GitHub 託管 runner 不收費、沒有分鐘數上限。改成私有 repo（Windows 分鐘數按 2 倍計）或改用大型 runner 才會收費。

**沒辦法完全自動化的**：SmartScreen 的提示是給人看的畫面。能自動驗證的是它背後的機制：檢查「來自網際網路」的標記（`Zone.Identifier`）有沒有被 setup 移除。

## 5. 決定與後續

- **目前**：用 VM（方式 B）做乾淨環境驗收。VM 裡安裝 Claude Code，由它依 `VM-ACCEPTANCE.md` 的分段 prompt 執行並回報；畫面操作（雙擊、SmartScreen、登出再登入）由人做。
- **之後可選**：方式 A 做成每次發布都跑的 CI 回歸測試（ticket 10，尚未開）。第一次跑可能有項目因 runner 環境不支援而失敗，屆時標成「CI 不適用」並說明原因，不靜默跳過。
- **兩種方式都不取代的**：乾淨帳號的人工驗收。它能抓出「我的電腦剛好有、別人沒有」的問題。
