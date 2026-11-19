# IMB-C 變更紀錄

## v2.0.0

- 更新主畫面、設定、輔助視窗與「關於 IMB-C」。加入即時外觀、語言切換與偏好保存，並修正英文設定側欄標籤被截斷。
- 提供 Windows x64 CLI、JSON Lines 輸出，以及選用的外部圖片分析。將轉檔狀態與外部分析的判定、退出碼分開。
- 原生轉檔程序使用獨立 AppContainer，從 SHA-256 核對的本機 runtime 快取執行。來源與輸出由主程式管理，支援透過本機暫存處理非本機 NTFS 的來源與輸出。
- 修正多個執行個體同時儲存偏好，以及手動偵測 iat 與批次分析同時進行時的競態。
- 沿用正式 MSI 升級家族，可升級 v1.3.3。已安裝的 1.4 RC 系列須先移除。

詳見 [v2.0.0 版本說明](RELEASE-NOTES-2.0.0.md)。

## v1.4 候選版

- [v1.4.0-rc.2](https://github.com/Kaido-MSVC/IMB-C/releases/tag/v1.4.0-rc.2) 為歷史候選版，後續正式版為 [v2.0.0](https://github.com/Kaido-MSVC/IMB-C/releases/tag/v2.0.0)。MSI 升級步驟請見 [v2.0.0 安裝說明](INSTALL.md)。

## v1.3.3 與更早版本

- [v1.3.3](https://github.com/Kaido-MSVC/IMB-C/releases/tag/v1.3.3)
- [v1.3.2](https://github.com/Kaido-MSVC/IMB-C/releases/tag/v1.3.2)
- [v1.3.1](https://github.com/Kaido-MSVC/IMB-C/releases/tag/v1.3.1)
