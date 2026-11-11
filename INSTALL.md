# IMB-C v1.3.2 — 安裝與升級說明

EXE 與 MSI 均未簽章，Windows 可能顯示 SmartScreen 警告或「未知的發行者」。執行前請先依 README.md 的「未簽章：核對 SHA-256 與 SmartScreen 放行」核對下載檔。

## 安裝與解除安裝

- Portable：完整解壓 IMB-C-v1.3.2-Portable.zip，再開啟 imb-c.exe。保留 ImageMagick、JXL、DLL、licenses、source 等全部內容，不要只移動 EXE。
- MSI：執行 IMB-C-v1.3.2-x64.msi。需要 x64 Windows 10/11 與管理員權限。依序閱讀完整授權並同意、選擇安裝目錄、確認捷徑選項、安裝。預設 Program Files\IMB-C；開始功能表建立 IMB-C 捷徑，桌面捷徑預設不勾選。
- 解除安裝：Windows 設定 → 應用程式 → 已安裝的應用程式 → IMB-C v1.3.2 Personal Use（發行者 Kaido-MSVC）→ 解除安裝；也可重新執行相同 MSI 使用維護畫面。無需手動刪除程式目錄。
- 從 1.3.1 或更早版本升級：先關閉 IMB-C，再直接執行新版 MSI，不需要先解除安裝。同一 UpgradeCode、新 ProductCode；安裝程式會移除舊版，沿用有效的既有安裝位置，明示位置優先，失效的舊位置回到預設。已安裝 1.3.2 時無法再安裝較舊版本。
- 1.3.0 在「已安裝的應用程式」顯示的發行者是 IMB-C，1.3.1 起改為 Kaido-MSVC；升級後只會留下 1.3.2 一筆。
- 本 MSI 不啟用同版 major upgrade；不要混裝內容不同但版號同為 1.3.2 的安裝包。完整解除安裝後不永久記住 MSI 路徑，使用者 LocalAppData 偏好保留。
- Portable 升級：關閉 IMB-C，刪除舊的程式資料夾，再解壓 1.3.2。偏好設定保存在 %LOCALAPPDATA%\IMB-C\preferences.ini，不受影響。
- 必要 MSVC runtime 已隨應用私有部署；不另修改或升級系統 runtime。Windows UCRT／系統 DLL 由受支援 Windows 提供。

## 完整材料

Portable 與 MSI 均直接內含完整第三方授權／告知、Microsoft Runtime 條款、LGPL 對應來源及重建／替換指引。licenses/wix-7.0.0 包含 WiX 7.0.0 原授權、OSMFEULA、官方完整來源 ZIP，以及本版修改過的安裝定義產生器（IMB-C-v1.3.2-authoring）。source 內含 1.3.2 應用程式與安裝定義來源 ZIP（IMB-C-v1.3.2-app-source.zip），以及沿用的 1.3.0 第三方相應來源 ZIP（第三方元件未變動）。無需另找外附補充包才能取得必要材料。
MSI 授權畫面含完整 Runtime 原文與第三方條款，包括 WiX 原文。
WiX 衍生／修改的安裝 UI 定義依隨附 MS-RL 提供，未將應用程式或其他元件改授權。

## 使用條款

公開發行的告知與 Microsoft Runtime 使用條款見 README.md 的「Microsoft Runtime 條款」。RUNTIME-HANDOFF.md、RUNTIME-RECIPIENT-TERMS.md 等好友交付文件、歷史 v1.2 文件與候選驗收報告保留作沿革，不再是現行交付程序。

## 功能界線

HDR 僅聲明已驗證 ICC PQ／HLG 路徑的像素與 ICC；不宣稱完整 HDR metadata、CICP-only、CLLI、mastering display、gain map、全部 EXIF/XMP 或所有檢視器顯示均保留；不含 tone mapping。JPEG 原始 bytes 完整還原僅適用專用 JPEG↔JXL 工具。1.3.2 的修正、行為變更與已知限制見 RELEASE-NOTES-1.3.2.md。
