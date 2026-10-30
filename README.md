# IMB-C v1.3.0

Copyright (c) 2026 Kaido-MSVC。IMB-C 應用程式原始碼採 [MIT 授權](LICENSE)。ImageMagick、libjxl、Slint、FreeType、Microsoft VC++ Runtime 等第三方元件各有自己的授權與散布條件，並未改為 MIT；詳見 THIRD-PARTY-LICENSES-v1.3.0.md。文件中的 `licenses/` 路徑指完整二進位封包，原始碼儲存庫中的原文材料位於 `third-party/materials/`；大型相應來源封存另由完整來源包提供。此儲存庫尚未公開，正式發行封包亦須完成獨立驗收。

解壓完整可攜資料夾，執行 imb-c.exe。保留 ImageMagick、JXL、licenses、source 及完整條款。

- AVIF 品質 1–100，預設 90、4:4:4；100 不保證無損，也不是保留畫質百分比。本組建高位深 AVIF 測得為 12-bit。
- WebP 預設真正無損，包含透明像素 RGB，限可由 8-bit 精確表達的來源；高精度來源會拒絕。需要降位深可關閉無損，或改選 PNG/TIFF/JXL。
- PNG/TIFF/JXL 保留 16-bit，採 32-bit 浮點 MIFF 私有中繼，並解碼驗證。32-bit 浮點 TIFF/JXL 另有小圖往返驗證。
- PSD 讀取已存合成圖、輸出平面影像，移除強制 8-bit/sRGB；保留來源位深及 ICC。透明合成可能有數值誤差，會報告、不保證無損。PSD 解碼的額外精度轉 PNG/TIFF/JXL 時使用 16-bit。
- ICC 預設保留。可選目標 ICC 執行色彩轉換，取消選檔可清除；來源無 ICC 不猜測色域。此功能不含 HDR tone mapping。
- 每筆輸出均檢查解碼、像素長度、位深、ICC；無損失敗不保存。日誌 max / RMSE 為標準化樣本誤差，不是主觀畫質分數。有色彩轉換/JPEG 白底時，與要求轉換後的基準比較。
- 完成通知可關閉；取消不通知。來源及輸出資料夾分開記憶，輸出格式與選項持久保存到 %LOCALAPPDATA%\IMB-C\preferences.ini。相容讀取舊程式旁 imb-c-ui.ini；無效設定回預設，失效目錄回選擇流程。
- JPEG↔JXL 原始位元組封裝/還原仍由獨立 JXL 工具執行，保留 SHA-256 驗證；一般像素轉換不是 JPEG bytes 還原。

HDR 驗證範圍：Rec.2100 PQ/HLG 的 16-bit 合成樣本經 ImageMagick/PNG/JXL 往返，像素與各自 4324-byte ICC 一致。這不是所有 CICP/CLLI/EXIF/HDR gain map 或所有檢視器顯示的保證。請先保留來源，不把未驗證的 HDR 容器路徑當完整保留保證。

MSI 更新路徑時序已修正；完整內容使用獨立產品/元件/登錄/捷徑識別，已實測安裝、1.2.1→1.3.0 升級、修復、啟動及卸載。未在現有正式安裝上測試升級。完整卸載後不保留永久 MSI 路徑紀錄；個人偏好設定保留在使用者資料夾。未簽章。

MIT 授權允許公開散布 IMB-C 自有程式碼。隨附二進位與第三方材料的權利須另依各自條款確認；既有 Personal-Use 封包的交付文件屬歷史紀錄，不能用來限制 MIT 程式碼。第三方授權基礎見 THIRD-PARTY-LICENSES-v1.2.0.md，v1.3 增補見 NATIVE-ADDITIONS-v1.3.json 與 THIRD-PARTY-NOTICES.md。

## 開發與重建
Rust/cargo 1.99.1（不升級）；edition 2024，依 Cargo.lock 使用原依賴。先從本正式版完整可攜包還原 runtime：`python -X utf8 scripts/prepare-v13-runtime.py <解壓資料夾>`，再執行 `cargo build --locked --release --bin imb-c`。使用既有 MSVC/Windows SDK。不得使用舊 prepare-dev-runtime.py 覆寫 v1.3 policy。
必要回歸：`cargo run --locked --bin verify-v13 -- <專案絕對路徑>`；一般測試 `cargo test --locked --bin imb-c`。Windows 原子保存需正常桌面權限。32-bit 測試另設 IMBC_TEST_ENGINE 後執行被忽略的 v13_float_precision。

安全界線：僅有 HDR/廣色域 CICP 而無 ICC 的 AVIF，目前明確拒絕，避免誤標為 sRGB。已驗證的 PQ/HLG ICC 路徑可用。

2026-10-29 最終驗收：8-bit 來源套用目標 ICC 時，PNG/TIFF/JXL/PSD 改用 16-bit，避免輸出量化使無損驗證拒絕。驗證以要求轉換後的基準比較。含 EXIF/XMP/ICC/COM 的 baseline/progressive JPEG 經專用 JXL 工具後 bytes 與 SHA-256 完全一致。新增可重現測試 `cargo run --locked --bin acceptance-v13 -- <專案絕對路徑>`。詳 ACCEPTANCE-v1.3.0.md。

## 2026-10-30 最後授權審查
自有程式碼 MIT 署名為 Kaido-MSVC。FreeType 2.14.3 固定版原文已核對，Microsoft Runtime 發行者個人資格由本人確認，五份 DLL 與正式 Redist 相同。詳 LICENSE-REVIEW-v1.3.0.md。公開來源候選可供審閱；含 Runtime 的 Portable/MSI 仍沿用先同意後交付流程，不能直接當成無條件公開下載的發行包。
