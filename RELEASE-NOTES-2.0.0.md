# IMB-C v2.0.0

Windows x64 批次圖片轉檔工具。v2.0.0 接續 v1.4 RC 系列，包含新版介面、CLI 與轉檔引擎的程序隔離。

- 主畫面、設定和輔助視窗可即時切換語言與外觀。英文設定側欄寬度會依標籤調整。「說明」選單提供「關於 IMB-C」與第三方署名。
- CLI 提供批次轉檔及 JSON Lines。選用的 iat 結果與轉檔狀態分開，請依 [CLI JSON 契約](docs/v1.4-cli-json-contract.md) 判讀。
- ImageMagick 與 JPEG XL 工具在獨立的 Windows AppContainer 中執行。程式以 SHA-256 驗證原生 runtime，先在受控暫存完成輸出驗證，再發布至目的地。
- 修正多執行個體儲存偏好，以及手動偵測 iat 與批次工作競態。

## 安裝與授權

正式 MSI 會升級既有 v1.3.3 等正式 MSI。若曾安裝 1.4 RC MSI，請先移除該候選版。請從同批 `SHA256SUMS.txt` 核對未簽章 MSI 和 ZIP 附件，操作步驟見 [安裝說明](INSTALL.md)。

完整封包含四份 Microsoft Visual C++ Runtime 14.52.37029.0 DLL。請閱讀 [接收條款](RUNTIME-RECIPIENT-TERMS.md) 和 [Microsoft 官方 Runtime 條款](https://visualstudio.microsoft.com/license-terms/vs2026-ga-visualcpp-v14-redist-runtime/)。MSI 會要求明確同意，使用 Portable 所附 Runtime 即表示同意。LGPL 對應來源、告知及 [替換方式](LGPL-REPLACEMENT.md) 隨完整封包提供。

## 已知限制

請只處理受信任的圖片。原生暫存採容量預留與定期檢查，仍可能在檢查間隔內超額。100 MP JXL 未通過先前的驗收。實際 SMB 斷線、非本機 NAS 與乾淨 Windows 的本版 MSI 安裝生命週期尚待驗證。
