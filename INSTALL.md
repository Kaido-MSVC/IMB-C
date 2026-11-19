# 安裝 IMB-C v2.0.0

1. 在下載資料夾以 `Get-FileHash .\IMB-C-v2.0.0-x64.msi -Algorithm SHA256` 計算雜湊，與同批 `SHA256SUMS.txt` 比對。Portable 與 Source ZIP 也用相同方式核對。雜湊不符時請重新取得檔案。
2. 關閉正在執行的 IMB-C。若已安裝 IMB-C 1.4 RC 系列，先在 Windows 的「已安裝的應用程式」解除安裝該候選版。
3. 開啟 MSI，閱讀及接受完整 [Microsoft Runtime 接收條款](RUNTIME-RECIPIENT-TERMS.md)，選擇安裝位置後完成安裝。安裝程式會新增開始功能表捷徑，可選擇建立桌面捷徑。

MSI 沿用正式版升級家族。從 v1.3.3 等較舊正式 MSI 升級時會移除舊版程式，並預設沿用仍存在的既有安裝位置。全新安裝預設使用 `%ProgramFiles%\IMB-C`。使用者偏好位於 `%LOCALAPPDATA%\IMB-C`，安裝程式的移除清單只包含安裝程式所管理的資源。升級前可備份偏好。

靜默安裝前須閱讀並明確接受條款，再使用 `msiexec /i IMB-C-v2.0.0-x64.msi /qn ACCEPTTERMS=1`。

Portable 使用前請先閱讀完整 Runtime 接收條款，使用所附 Microsoft Runtime 即表示同意。核對 ZIP 後完整解壓到新資料夾，再開啟 `imb-c.exe`。CLI 請從 PowerShell 或命令提示字元執行 `imb-c-cli.exe --help`。請保留執行檔、`ImageMagick`、`JXL`、授權與來源材料的相對位置。Portable 與 MSI 會讀取同一份使用者偏好。
