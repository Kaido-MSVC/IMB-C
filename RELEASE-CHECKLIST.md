# IMB-C 發布檢查清單

依 v1.3.1 的發布準備流程（本機 `RELEASE-PREP-1.3.1.md`）整理。每次發布照順序執行；未執行或不適用的項目要寫明原因。以下 `X.Y.Z` 指本次版號，`P.Q.R` 指上一個已發布版本。

**Tag、Push、上傳 Release 一律在第 1～9 節全部完成並經確認後才做。**

## 1. 工作目錄

- [ ] 在 GitHub clone 的工作目錄作業，`git status` 乾淨，`git fetch` 後確認與 `origin/main` 的差異都是本次要發布的 commit。
- [ ] local-only 檔案（`待修正.md`、`BUGFIX-*`／`REVIEW-*`／`RELEASE-PREP-*` 報告、`dist\`、`work\`、`target\`）沒有進入版控：`git status --ignored` 確認它們列在 ignored。
- [ ] 新增的根目錄 `.md` 若要公開，先在 `.gitignore` 加例外（`/*.md` 預設排除）。
- [ ] 安裝定義：v1.3.2 起 `installer\wix7-vX.Y.Z\` 的 5 個來源檔（`make-msi-source.py`、`Build.ps1`、`inspect-msi.ps1`、`README.md`、`UPGRADE-POLICY.md`）納入版控；產生的 `Release.wxs`、`Installer-Terms.*`、`TERMS-INVENTORY.json` 由 `.gitignore` 排除。新版本從上一版的資料夾複製，只改版本識別並換新 ProductCode。v1.3.0／v1.3.1 的定義封存在本機 `work\archive\installer\`（不在版控）。

## 2. 版號一致性

- [ ] `Cargo.toml` 的 `version = "X.Y.Z"`。
- [ ] `Cargo.lock` 只更新本專案那一列：`cargo update -p imb-c --offline`，確認 diff 只有 1 行。
- [ ] `assets/app.rc`：`FILEVERSION`／`PRODUCTVERSION` 為 `X,Y,Z,0`，`FileVersion`／`ProductVersion` 字串為 `X.Y.Z`。
- [ ] 主視窗標題與 About 來自 `env!("CARGO_PKG_VERSION")`，不需改程式碼。
- [ ] `installer\wix7-vX.Y.Z\make-msi-source.py` 的 Package `Name`／`Version`，以及 `inspect-msi.ps1` 的預期版號。
- [ ] README 標題、`RELEASE-NOTES-X.Y.Z.md`、`INSTALL.md`、`PORTABLE-README.md` 的版號。
- [ ] 殘留檢查：在 Cargo、資源檔、`src/`、`scripts/`、`installer\wix7-vX.Y.Z\`、README 搜尋 `P.Q.R` 與 `P,Q,R`，剩下的只能是歷史敘述或既有檔名。

## 3. 發布者與 MSI 識別

- [ ] `assets/app.rc` 有 `CompanyName "Kaido-MSVC"`。
- [ ] `make-msi-source.py` 的 `Manufacturer='Kaido-MSVC'`；`inspect-msi.ps1` 檢查 Manufacturer。
- [ ] **UpgradeCode 沿用** `{3EAD7AE1-2433-4AB2-B14B-70A661D06781}`。
- [ ] ProductCode 為新產生的值，且不等於任何已發布版本（`inspect-msi.ps1` 檢查不得等於上一版）。
- [ ] MajorUpgrade 沿用：`AllowSameVersionUpgrades='no'` 與 DowngradeErrorMessage。

## 4. 打包用的 README（N4）

封包內的 README 以**版控中的版本**為準，不使用本機另存或手動修改的副本。

- [ ] 打包前先 `git fetch`，確認 `git diff origin/main -- README.md` 沒有未預期的差異（GitHub 網頁上直接修改過的內容要先合併回來）。
- [ ] 放進 Portable／MSI／Source.zip 的 README 取自 `git show HEAD:README.md`。
- [ ] 打包後比對：從封包解出的 README 與 `git show HEAD:README.md` 內容相同（`core.autocrlf=true` 時以忽略換行符方式比對）。
- [ ] `LICENSE`、`INSTALL.md`、`THIRD-PARTY-NOTICES.md` 等其他公開文件同樣取自版控。

## 5. 建置

- [ ] 準備私有 runtime（從上一版 Portable 解壓後的根目錄還原並校驗清單）：

  ```powershell
  python -X utf8 scripts\prepare-v13-runtime.py "<Portable 解壓後的根目錄>"
  ```

- [ ] 以 `scripts\build-release.ps1` 建置（含 `--remap-path-prefix`，不要改用一般 `cargo build --release` 的 exe 打包）。
- [ ] `build-release.ps1` 以 vswhere 找到 VS、透過 `VsDevCmd.bat -arch=amd64 -host_arch=amd64` 載入環境，開頭會印出實際使用的 MSVC 工具組版本與 link.exe 路徑。把印出的版本記在建置紀錄裡。
- [ ] `cargo build --release --locked` 沒有警告。

## 6. 測試

### 6.1 一般測試

```powershell
cargo test --release --locked --bin imb-c
```

### 6.2 需要 runtime 的 ignored 測試（N5，每次發布固定執行一次）

在專案根目錄執行。`IMBC_TEST_ENGINE` 一律用絕對路徑，指向本次要封裝的 runtime：

```powershell
$root = (Get-Location).Path
$env:IMBC_TEST_ENGINE = Join-Path $root 'target\release\ImageMagick\magick.exe'
$env:IMBC_TEST_UNC_ROOT = '\\localhost\' + ($root -replace '^([A-Za-z]):', '$1$') + '\work\<本次工作目錄>'
$env:IMBC_TEST_ASSETS = '<含 rukamizugi.psd 的資料夾>'
cargo test --release --locked --bin imb-c -- --include-ignored --skip r2_child_temp_fixture --test-threads=1
```

- 先建立 `work\<本次工作目錄>`；`IMBC_TEST_UNC_ROOT` 需要可存取的本機系統管理共用（例如 `D$`）。
- `IMBC_TEST_ASSETS` 是含 `rukamizugi.psd` 的資料夾；未設定時 `real_csp_psd_conversion_and_source_preservation` 會印出原因並跳過。
- `r2_child_temp_fixture` 是父測試啟動的子程序入口，單獨執行沒有意義，一律 `--skip`。
- ignored 測試都在 `--bin imb-c`。`verify-v12`、`verify-v13`、`acceptance-v13` 是驗收用 CLI，沒有 `#[test]`，`cargo test --bin verify-v13 -- --ignored` 會執行 0 項。
- 記錄 runtime 版本、通過／失敗數與日誌位置。

**標註為舊版引擎的測試**：

| 測試 | 標註 | 說明 |
| --- | --- | --- |
| `v121_small_formats_and_failure_stages` | v1.2.1 候選引擎 | 以 v1.3 runtime 執行通過（v1.3.1 與 2026-11-11），保留在上面的固定指令內 |
| `psd_composite_and_transparency` | 已封裝的 v1.1 引擎 | 同上 |
| `imagemagick_matrix_and_failures` | 已封裝的 ImageMagick | 同上；未設定 `IMBC_TEST_ENGINE` 時改用 `locate_engine()` |

更換 runtime 後，如果這三項因為綁定舊引擎的行為而失敗，不要改用新 runtime 硬改預期值：另以對應的舊版引擎（`releases\` 內的已封裝版本）單獨執行該案例並記錄路徑，再用新 runtime 驗證同一能力。其餘 ignored 測試都標註為 v1.3 runtime。

### 6.3 其他

- [ ] `cargo audit`，記錄結果；有公告時判斷是否影響 exe 實際連結的 crate。
- [ ] 有改 Python 腳本時執行 `tests\test_prepare_runtime.py`。
- [ ] GUI 手動確認：主視窗標題、About、實際選檔轉檔、完成通知。人工產生的 GUI 煙霧測試素材在本機 `work\test-assets\`（不在版控）：`gui-smoke-avif-r2\`（150 張無 ICC／CICP 的 AVIF 與 5 張 PNG）、`gui-smoke-avif\`（第一版，其中的 AVIF 是 0 位元組，不要使用）、`ui-smoke\`，步驟參考 `work\test-assets\GUI-SMOKE-2026-11-10.md`（其中的路徑是搬移前的舊位置）。輸出請另選空白資料夾。

## 7. 授權

- [ ] 依本機授權範本資料夾 `README.md` 的規則處理；授權條文取自該資料夾，不重新下載。
- [ ] 相依有變動時，以 `cargo tree -e normal --target x86_64-pc-windows-msvc` 比對 exe 實際連結的 crate 與 `licenses/rust/<名稱>-<版本>`，同步 `RUST-INVENTORY.csv`、`LICENSE-FILES-SHA256.csv`。
- [ ] 原生 runtime 有變動時，同步 `RUNTIME-MANIFEST-*.json` 與 `THIRD-PARTY-NOTICES.md`。

## 8. 本機路徑與機密掃描

掃描範圍：Portable 解壓後所有檔案（含 exe／DLL）、MSI payload、Source.zip 與封包內的 app-source ZIP（逐檔）。

- [ ] 搜尋專案與工作目錄的絕對路徑、`C:\Users`、本機使用者名稱、測試素材資料夾、`work/`、`\\localhost`、備份副檔名（`.bak`、`.old`、`.tmp`）、`.env`、金鑰類副檔名（`.pfx`、`.p12`、`.key`、`.pem`、`.snk`）、`.pdb`／`.wixpdb`。
- [ ] exe 內的路徑應已重映射為 `.` 與 `/build-user`；有命中就不要打包。
- [ ] 第三方相應來源內的上游範例（例如 glib 測試 PEM、WiX 的 `wix.snk`）不屬於本專案，記錄即可。

## 9. 打包與 SHA-256

- [ ] Portable：ZIP 的 CRC 與 `MANIFEST-SHA256.txt` 逐檔核對；解壓到全新暫存目錄，以隔離設定（`IMBC_CONFIG_DIR`）啟動確認標題與關閉。
- [ ] MSI：`Build.ps1` 編譯、payload 雜湊核對、`wix msi validate`（ICE）exit 0、`inspect-msi.ps1` 通過；MSI File 表與 payload 一一對應。
- [ ] 與上一版 MSI 逐表比對：安裝控制表只應有預期差異（Upgrade 表版號、Property 的 ProductCode／ProductName／ProductVersion），沿用元件的 Component GUID 不變。
- [ ] `SHA256SUMS.txt` 涵蓋 Portable、MSI、Source.zip，以 `Get-FileHash` 重新核對一次。
- [ ] 未簽章時沿用 README「下載」節的 SHA-256 核對與 SmartScreen 說明。
- [ ] `.wixpdb` 等除錯資料不放進 `dist\X.Y.Z\`。

## 10. 升級與降級測試（手動）

先保留上一版 MSI，並備份 `%LOCALAPPDATA%\IMB-C\preferences.ini`。

| 步驟 | 操作 | 要確認 |
| --- | --- | --- |
| 1 | 在「已安裝的應用程式」確認 **IMB-C vP.Q.R Personal Use** 與安裝資料夾；關閉 IMB-C | — |
| 2 | `msiexec /i IMB-C-vX.Y.Z-x64.msi /l*v "%TEMP%\imbc-XYZ-upgrade.log"`，同意條款，保持預設資料夾 | 預設資料夾是原本的位置；不要求先解除舊版；未簽章時 UAC 顯示「未知的發行者」屬預期 |
| 3 | 回到「已安裝的應用程式」 | 只剩 **IMB-C vX.Y.Z Personal Use** 一筆，發行者 **Kaido-MSVC**；`imb-c.exe` 詳細資料為 X.Y.Z、公司 Kaido-MSVC |
| 4 | 從開始功能表啟動，開 About，用 GUI 轉 2 張圖（含會產生警告的檔案） | About 顯示 vX.Y.Z；轉檔成功、警告報告產生；偏好設定沿用 |
| 5 | 關閉 IMB-C，執行上一版 MSI | 顯示 `A newer version of IMB-C is already installed.` 並結束，新版不受影響 |

## 11. 發布

- [ ] 第 1～10 節完成並確認後，才建立 Tag `vX.Y.Z`、Push、建立 GitHub Release。
- [ ] Release 附件：Portable、MSI、Source.zip、`SHA256SUMS.txt`；Release Notes 取自 `RELEASE-NOTES-X.Y.Z.md`。
- [ ] 發布後從 GitHub 重新下載附件，核對 SHA-256 與 `SHA256SUMS.txt` 一致。
- [ ] 更新 `待修正.md`，把本版處理的項目移到「已完成」。
