# IMB-C v1.3.2

Copyright (c) 2026 Kaido-MSVC。IMB-C 應用程式原始碼採 [MIT 授權](LICENSE)。ImageMagick、libjxl、Slint、FreeType、Microsoft VC++ Runtime 等第三方元件各有自己的授權與散布條件，並未改為 MIT；詳見 [第三方授權清單](THIRD-PARTY-LICENSES-v1.3.0.md)。文件中的 `licenses/` 路徑指完整二進位封包，原始碼儲存庫中的原文材料位於 `third-party/materials/`；大型相應來源封存另由完整來源包提供。

## 繁體中文介紹

IMB-C 是 Windows x64 的本機批次圖片轉換工具，以 Rust 和 Slint 開發，使用 ImageMagick 處理圖片。可輸出 WebP、PNG、JPEG、AVIF、PSD、TIFF 和 JPEG XL（JXL）。批次處理會保留原始檔案，同名輸出會自動編號。

轉換預設保留 ICC 色彩描述檔，也可指定目標 ICC 進行色彩轉換。程式會解碼驗證每份輸出，位深與無損支援依輸出格式而異。另有獨立的 JXL 工具可封裝與還原原始 JPEG 位元組，封裝時會還原並比對 SHA-256。介面可切換繁體中文、簡體中文、英文及日文，並記住選擇。

## English overview

IMB-C is a local batch image converter for Windows x64, built with Rust and Slint and powered by ImageMagick. It outputs WebP, PNG, JPEG, AVIF, PSD, TIFF, and JPEG XL (JXL). Source files are preserved, and output names are numbered automatically to avoid overwriting files.

IMB-C retains ICC profiles by default and can convert colors to a selected target profile. It decodes each output to verify it. Bit depth and lossless support depend on the output format. A dedicated JXL tool can pack and reconstruct the original JPEG bytes, with SHA-256 verification on packing. The interface supports Traditional Chinese, Simplified Chinese, English, and Japanese and remembers your selection.

The application code is [MIT-licensed](LICENSE). Bundled components have separate terms. Read the [third-party notices](THIRD-PARTY-LICENSES-v1.3.0.md) and the [Microsoft Runtime terms](#microsoft-runtime-條款) below before using the package.

## 下載

從 [v1.3.2 Release](https://github.com/Kaido-MSVC/IMB-C/releases/tag/v1.3.2) 下載 Portable ZIP、Windows x64 MSI、完整相應來源 ZIP 及 `SHA256SUMS.txt`。GitHub 自動產生的 Source code ZIP 不包含大型第三方相應來源；需要重建或履行 LGPL 散布義務時，請使用另附的完整相應來源包與封包內的來源／替換材料。安裝與升級方式見 [INSTALL.md](INSTALL.md)。1.3.2 的改善見 [1.3.2 Release Notes](RELEASE-NOTES-1.3.2.md)，相對 1.3.0 的修正與行為變更見 [1.3.1 Release Notes](RELEASE-NOTES-1.3.1.md)。

附件內保留部分歷史 Personal-Use／candidate 名稱、好友交付與當時審查文字；正式發行頁及本 README 取代發行者自行規定的舊交付程序，並不修改第三方授權原文，也不表示第三方元件一律可以任意轉交。未簽章，請先核對 SHA-256。

### 未簽章：核對 SHA-256 與 SmartScreen 放行

IMB-C 的 exe 與 MSI 沒有程式碼簽章，所以 Windows 可能顯示「Windows 已保護您的電腦」（SmartScreen），安裝時的使用者帳戶控制也會顯示「未知的發行者」。請先核對檔案，確認無誤再放行。

1. **核對 SHA-256**：在下載資料夾開啟 PowerShell，執行
   ```powershell
   Get-FileHash .\IMB-C-v1.3.2-x64.msi -Algorithm SHA256
   ```
   把輸出的 Hash 與同一個 Release 的 `SHA256SUMS.txt` 對應列比較（大小寫不影響）。也可以用 `certutil -hashfile IMB-C-v1.3.2-x64.msi SHA256`。Portable ZIP 與來源 ZIP 用同樣方式核對。**不相符就不要執行**，請重新從 Release 頁下載。
2. **SmartScreen 放行**：核對相符後，在「Windows 已保護您的電腦」視窗按「其他資訊」，確認檔名後按「仍要執行」。
3. **Portable 解除封鎖**：解壓前先在 ZIP 上按右鍵 →「內容」→ 勾選「解除封鎖」→「確定」，避免解壓出的每個檔案都被標記成來自網際網路。也可以在 PowerShell 執行 `Unblock-File .\IMB-C-v1.3.2-Portable.zip`。

## Microsoft Runtime 條款

使用隨附 Microsoft Runtime 前，請閱讀並同意[完整 Runtime 使用條款](third-party/materials/microsoft/Runtime-2026.txt)。依官方條款，使用軟體表示接受；若不同意，請勿使用。MSI 安裝時另有完整條款及同意畫面。Microsoft Runtime 維持專有授權，不適用本專案 MIT。

後續散布者須自行具備適用的 Microsoft 再散布權利，並要求其散布者及外部使用者同意至少同等保護 Microsoft 元件的條款；本次下載不另授予 Microsoft 元件的獨立再散布權。參閱 [Community 原文](third-party/materials/microsoft/Community-2026.txt) 與 [2026 REDIST](https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution)。LGPL 元件的修改、替換及為偵錯修改所需反向工程權利仍保留。

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
自有程式碼 MIT 署名為 Kaido-MSVC。FreeType 2.14.3 固定版原文已核對，Microsoft Runtime 發行者個人資格由本人確認，五份 DLL 與正式 Redist 相同。歷史材料核對詳 LICENSE-REVIEW-v1.3.0.md；本次公開發行的告知與使用條款見上方「Microsoft Runtime 條款」，不沿用歷史逐一好友交付流程。

## 2026-11-10 第二輪 Bug 修正（維持 1.3.0）

依 [Claude 複查](REVIEW-2026-11-10.md) 修復第一輪回歸。第二輪的 metadata-only coder 白名單已由第三輪修正取代，現行 ICC 探測與保護方式見下節；不會以「探測不到」為由略過必要比對。

PSD v1 的空白 Alpha 名稱資源（1006／1045）只在私有來源副本略過；遇到不認得的資源簽章、尾端殘餘、最後一筆奇數資料未補齊時，保留原始位元組交給引擎判斷。色彩區／資源區的檔案邊界及已宣告資源資料長度仍受檢查。PSB v2 尚未支援，沒有新增 v1.4 功能。

無 ICC 且無 CICP 的合法 AVIF（包含舊版 1.3.0 輸出）可以繼續轉檔，但會顯示「色彩資訊未驗證」與缺少標記警告。像素樣本相同不代表原始色域、傳輸曲線或顯示外觀相同；不會把解碼器產生的 JXL ICC 當作原始 ICC 已驗證。明確帶有不支援的 HDR／廣色域／錯誤 CICP 仍拒絕，沒有 ICC 的來源也不能套用目標 ICC。

單檔轉檔／驗證仍共用 600 秒總期限。啟動前檢查私有 policy，並核對官方 `-list policy` 的生效規則及應被禁止的 RGB coder。Windows 登錄的系統 policy 仍可能被載入：偵測到額外生效規則時會停止並提示，不更改系統設定。**這是干擾偵測與緩解，尚非完整隔離**；隱藏規則與檢查後設定被更動的風險仍見 [第二輪交接報告](BUGFIX-2026-11-10-R2.md)。

### 暫存檔與清理

- 預設使用 `%TEMP%\imb-c-work-*` 存放私有中繼、驗證資料與來源相容副本。像素／ICC 驗證現在使用匿名、獨占建立的暫存檔，透過檔案控制代碼與 stdout 傳遞；Windows 會在最後一個控制代碼關閉後刪除，其他程序持有時會延後。
- 小型目標 ICC 快照仍需要檔名，使用隨機名稱、獨占建立與安全相對檔名。Windows 工作路徑過長時，引擎工作目錄／快取／ICC 快照會改放本機 `%LOCALAPPDATA%\imb-c-cwd-*`，與原工作一起清理；長路徑及 UNC 的匿名資料仍由 Rust 控制代碼讀寫。若沒有可用的短工作路徑會回報錯誤。
- 正常完成、失敗、取消及逾時，均先停止子程序、關閉檔案再清理。**已完成驗證並發布輸出後，只有清理失敗會顯示成功加警告**，警告保留輸出及暫存位置，避免把同一檔誤算為轉檔失敗。
- 引擎快取上限 8 GiB 不包含所有中繼、驗證及目的地輸出；目前沒有全工作共用磁碟預算或寫入硬上限。接近 400 MP 的工作仍可能需要約 16–25 GB，兩個同時工作約 30–50 GB（程式結構估算，非本輪壓力實測）；大圖請預留足夠空間。
- 異常退出仍可能留下工作目錄、引擎快取、小型具名 ICC、目的地 `.tmp*`，以及尚未結束的 `magick.exe`／JXL 子程序；匿名檔須等所有持有者結束才刪除。手動清理前先確認相關工作與子程序已停止，只處理警告所指或能確認屬於該次工作且未發布的暫存，不要掃除其他程式的全域 `magick-*`。目前沒有自動崩潰回收。

完整修改、實測結果與未執行項目見 [第二輪 Bug 修正報告](BUGFIX-2026-11-10-R2.md)。第一輪報告保留為歷史紀錄，不能取代本輪結果。

## 2026-11-10 第三輪回歸修正（維持 1.3.0）

ICC 探測不再依副檔名或 coder 白名單使用 `-ping`。原生 PNG 先以有邊界檢查的 chunk 掃描、跳過像素資料；只有能證明沒有 ICC 或不明／文字中繼資料時才使用 metadata-only 探測。帶 iCCP、可能含舊式 raw profile 的 PNG，以及 WebP、JXL、TIFF、AVIF 等其他格式，使用完整讀取取得 profile 資訊。引擎可能需要解碼像素，這是避免漏報的相容性代價。若 PNG 明明含 iCCP 而引擎無法讀出 ICC，會拒絕轉檔，不能當成無描述檔。

來源 ICC 擷取後保存其原始雜湊；最終輸出必須與來源或明確選取的目標 ICC 完全相同。來源沒有 ICC 時才保留原有 JXL 合成描述檔例外。RGB ICC 的 PNG 輸出明確使用 RGBA 儲存，避免中性 RGB 像素被自動編成灰階 PNG 而失去 ICC；位深、透明度與樣本無損驗證照常執行。灰階儲存搭配 RGB ICC 的來源若轉 JXL 時被引擎替換描述檔，會拒絕並清除未發布輸出。

WebP 使用 RGB 色彩模型，預設不接受 Gray 或其他非 RGB ICC。要保留原樣本與描述檔，請用 PNG／TIFF／JXL；若接受色彩轉換及量化／有損，請明確選擇 RGB 目標 ICC，並**自行關閉「WebP 真正無損」**。只選 RGB 目標並不保證成功：本輪灰階樣本轉 sRGB 後不能由 8-bit WebP 精確表示，維持真正無損會被拒絕。程式不自動更改設定，不刪除 ICC，不放寬驗證。沒有色彩標記的合法 AVIF 仍可成功加警告；一般無 ICC 的 PNG 不因此被列為缺少 AVIF 色彩資訊。

### 批次警告報告

既有即時日誌仍限制 200 行／64 KiB。批次結束另顯示無警告成功數、成功但有警告數、警告訊息總數、失敗與取消數。有警告時，在輸出資料夾以隨機名稱獨占建立 `IMB-C-warnings-*.txt`，逐筆保存完整警告與來源路徑；沒有警告不建立報告。「查看警告報告」使用每頁約 32 KiB 的 UTF-8 分頁，保留先前已被即時日誌捲掉的警告，也可用文字編輯器讀取整份檔案。

警告報告是供使用者保留的結果紀錄，**不會當暫存檔自動刪除**，請依需要自行清理。若報告建立／寫入失敗，會保留明確錯誤與有限長度的警告摘要，停止後續工作；已完成的輸出仍按成功計數，但介面會顯示報告不完整，不隱藏儲存問題。完整 GUI 批次人工操作本輪未完成；警告計數、日誌捲動、分頁與儲存錯誤已有自動回歸。

本輪測試與剩餘風險見 [第三輪交接報告](BUGFIX-2026-11-10-R3.md)。policy 隱藏規則／競態、共用磁碟預算、寫入硬上限及崩潰回收仍未解決，沒有新增 v1.4 功能。

## 2026-11-10 最後相容性檢查與 GUI 驗收準備

**灰階 JXL→WebP 預設無損路徑仍安全拒絕，沒有宣稱已恢復。** JXL 即使沒有內嵌 ICC，也有完整色彩定義；解碼器可能把列舉灰階定義合成 Gray ICC。隨包 ImageMagick 只提供解出的 profile，未提供內嵌／合成來源旗標，所以訊息改稱「解碼器提供的色彩資訊，可能合成，不能斷定真的內嵌」。不能以沒有內嵌 ICC 為由丟掉必要的色彩定義。把 Gray ICC 原樣寫進 RGB WebP、略過 ICC 比對或將有量化誤差的 RGB 轉換標成真正無損，都不是本版允許的恢復方式。

目前可用替代路徑是保留 PNG／TIFF／JXL，或明確選 RGB 目標並手動關閉 WebP 真正無損（接受色彩轉換及量化／有損）。新的灰階／精度錯誤有繁中、簡中、英文、日文一致操作提示，底層其他錯誤仍依既有規則保留原文。

人工 GUI 煙霧測試以人工產生的素材執行（150 張無 ICC／CICP 的 AVIF 與 5 張 PNG，不含使用者原始圖片）。相對 1.3.0 的修正、行為變更與已知限制見 [1.3.1 Release Notes](RELEASE-NOTES-1.3.1.md)。
