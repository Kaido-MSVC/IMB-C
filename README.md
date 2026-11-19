# IMB-C v2.0.0

IMB-C 是 Windows x64 的批次圖片轉檔工具，提供 Slint 圖形介面與 `imb-c-cli.exe`。請使用受信任的圖片，並保留原始檔。

- 可轉換 PNG、TIFF、WebP、JPEG、AVIF、JPEG XL（JXL）與 PSD 等格式。實際位深、透明度與色彩描述檔保留依來源、格式及編碼選項而定。
- 完成轉檔後會重新解碼驗證。無損驗證失敗會拒絕發布該輸出，同名檔會另行編號。一般像素轉換與 JPEG⇄JXL 原始位元組封裝各有各自的驗證。
- 主畫面、設定與輔助視窗可即時切換語言及外觀。「說明」選單的「關於 IMB-C」提供版本與第三方署名。
- CLI 的外部圖片分析結果與轉檔狀態分開。iat 預設停用，請從 [Image Analysis Tool 正式版](https://github.com/Kaido-MSVC/ImageAnalysisTool/releases/tag/v1.0.0) 下載後自行啟用。自動化用法請見 [CLI JSON 契約](docs/v1.4-cli-json-contract.md)，或執行 `imb-c-cli.exe --help`。

## 安裝與可攜使用

從 [v2.0.0 正式發行頁](https://github.com/Kaido-MSVC/IMB-C/releases/tag/v2.0.0) 取得 Windows x64 套件。

- [MSI 安裝版](https://github.com/Kaido-MSVC/IMB-C/releases/download/v2.0.0/IMB-C-v2.0.0-x64.msi)
- [Portable 免安裝版](https://github.com/Kaido-MSVC/IMB-C/releases/download/v2.0.0/IMB-C-v2.0.0-Portable-x64.zip)
- [原始碼 ZIP](https://github.com/Kaido-MSVC/IMB-C/releases/download/v2.0.0/IMB-C-v2.0.0-Source.zip)
- [SHA256SUMS.txt](https://github.com/Kaido-MSVC/IMB-C/releases/download/v2.0.0/SHA256SUMS.txt)，用於核對下載檔的 SHA-256

**從 1.4 RC 系列 MSI 升級時，請先關閉程式並在 Windows「已安裝的應用程式」解除安裝該候選版，再安裝 v2.0.0 MSI。**

安裝或解壓縮前請先核對雜湊。Windows 可能對未簽章的 MSI 和程式顯示 SmartScreen 或未知發行者提示。完整步驟、升級行為與偏好位置請見 [安裝說明](INSTALL.md)。

本封包含四份 Microsoft Visual C++ Runtime DLL。MSI 會顯示完整接收條款並要求明確同意。使用 Portable 所附 Runtime 即表示同意。請先閱讀 [完整接收條款](RUNTIME-RECIPIENT-TERMS.md) 與 [Microsoft 官方 Runtime 條款](https://visualstudio.microsoft.com/license-terms/vs2026-ga-visualcpp-v14-redist-runtime/)。

## 已知限制

圖片引擎在 Windows AppContainer 執行。轉檔暫存使用容量預留與定期監控，仍可能在檢查間隔內超額。大圖請預留足夠本機儲存空間。100 MP JXL 未通過先前的驗收。真實 SMB 斷線、非本機 NAS 與乾淨 Windows 的 v2.0.0 MSI 安裝生命週期仍待驗證。詳見 [版本說明](RELEASE-NOTES-2.0.0.md)。

## 原始碼與授權

Windows 建置使用 Rust 1.100.0、MSVC 與 Windows SDK。Rust 套件版本依 `Cargo.lock`，Slint 為 1.18.1。參閱 [建置說明](docs/build-2.0.0.md) 及 [變更紀錄](CHANGELOG.md)。

IMB-C 自有程式碼依 [MIT](LICENSE) 授權。外部元件依 [第三方告知](THIRD-PARTY-NOTICES.md) 所列條款提供，Microsoft DLL 適用其專有授權。LGPL 對應來源、建置材料及替換方式見 [LGPL 對應來源與替換方式](LGPL-REPLACEMENT.md)。
