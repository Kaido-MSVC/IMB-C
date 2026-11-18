# IMB-C v1.4.0-rc.2

> 預發行候選版（Pre-release），尚非正式版本。

Windows x64 的預發行候選版，僅供個人使用或少量分享，請只處理受信任的圖片。

## 這版有哪些變更

- 加入 CLI、JSON 轉檔報告及選用的外部 iat 驗證。iat 預設停用，隨附封包未包含 iat。自行啟用時，iat 會以自己的程序解析圖片，原生引擎的 AppContainer 保護範圍不涵蓋 iat。
- 支援讀取 PSB v2 的已存合成圖，也可輸出 PSB v2。輸出包含單一圖層及合成影像，帶透明度的來源會拒絕寫出 PSD／PSB。
- ImageMagick 和 JPEG XL 引擎在受限制的 Windows AppContainer 中執行，各工作使用獨立的暫存空間。交付前驗證輸出，遇到同名檔會避開覆寫。
- 修正 Runtime 快取回收的刪除範圍、磁碟預留重複計算、交付中途取消及暫存清理失敗的回報，也處理了特定外部工具關閉 GUI 後的程序殘留。

### Runtime 更新（本次重建）

- 隨附的 Microsoft Visual C++ Runtime 由 14.51.36247.0 更新為 **14.52.37029.0**（Visual Studio 2026 Stable default toolset），不再舊於建置引擎與程式所用的 MSVC 14.52。
- `imb-c.exe` 與 `imb-c-cli.exe` 改用靜態 CRT，不再依賴可再散布的 Microsoft DLL；程式根目錄不再隨附 `vcruntime140.dll`，DLL 搜尋只允許 System32。
- Microsoft Runtime 條款改以標準方式提供：MSI 在安裝時的授權對話方塊要求勾選同意，Portable ZIP 隨附條款全文。GUI 不再於首次啟動顯示條款視窗，CLI 也移除了 `runtime-terms` 指令與 `--accept-runtime-terms` 參數，轉檔不再需要同意紀錄。先前 RC2 建置留下的 `%LOCALAPPDATA%\IMB-C\runtime-terms-acceptance.json` 不再使用，程式也不會刪除它。

## 驗收與已知限制

| 項目 | 結果 |
| --- | --- |
| **H1／P1 磁碟上限** | 原生暫存採容量預留、定期檢查及超限終止。沒有檔案系統硬配額，兩次檢查之間仍可能超額。 |
| **100 MP JXL** | 未成功驗收。先前在編碼前因可用的系統認可記憶體（committed memory）不足而拒絕，未產生輸出。本次沒有重跑大圖。 |
| **乾淨 Windows 的 MSI 安裝生命週期** | **SKIP**。完整安裝、修復、解除安裝、升級及 SmartScreen 尚未在乾淨 Windows 驗證。 |
| **SMB／NAS** | 本機 SMB 迴路和模擬刪除失敗的回收測試通過。實際網路中斷及非本機 NAS 未驗證。 |

本次重建的驗收結果見封包內的 `docs/v1.4-rc2-runtime-acceptance.md`。

執行檔與 MSI 均未簽章。取得封包後，請先以 SHA-256 對照同一批附件的 `SHA256SUMS.txt`。MSI 使用 RC2 產品系列與預設安裝目錄 `%ProgramFiles%\IMB-C 1.4 RC2`。若已安裝先前的 RC2 MSI，請先解除安裝再安裝本版；安裝程式偵測到同系列的其他版本會停止。安裝程式數值版號為 `1.4.0`，GUI、CLI 與 MSI 顯示名稱標示 `1.4.0-rc.2`。應用程式仍會使用同一使用者的 IMB-C 偏好與快取位置。

## 授權

IMB-C 自有程式碼採 MIT。隨附元件依各自授權提供。

封包含 Microsoft Visual C++ Runtime（`ImageMagick` 目錄內四份未修改的 DLL）。下載、安裝或使用即表示您同意隨附的 `RUNTIME-RECIPIENT-TERMS.md`，其中附有 Microsoft 官方條款全文；若不同意，請勿安裝或使用。請勿單獨抽取或再散布這些 DLL。[Microsoft Runtime 條款](https://visualstudio.microsoft.com/license-terms/vs2026-ga-visualcpp-v14-redist-runtime/)

LGPL 元件（GLib、FriBidi、liblqr、libheif）的修改、替換與重建權利保留。來源包附對應來源、固定版本的 JPEG XL、Brotli 及 Highway 快照與建置方式；替換方式見 `LGPL-REPLACEMENT.md`。
