# v1.1 新增 libheif

新引擎的來源、修改、重建與替換見 AVIF-REBUILD.md、licenses/avif；以下保留原有核心函式庫說明。

v1.1.1 compliance-r1：當前狀態以 RELEASE-GATES.md、COMPLIANCE-REVIEW.md 及 NATIVE-DEPENDENCIES-v1.1.json 為準。以下 GLib／FriBidi／liblqr 重建步驟沿用已驗證基線；libheif／AVIF／PSD／CLI 修改來源及重建另見 AVIF-REBUILD.md。歷史 NO-GO 保留於 evidence，不是當前阻斷。

# LGPL 來源、重建與替換

本包的 GLib、FriBidi、liblqr 以獨立 DLL 提供；沒有簽章鎖、雜湊鎖或防止替換的檢查。
IMB-C 不限制為修改 LGPL 函式庫及偵錯該等修改所需的反向工程。
替換與修改後的元件仍依各自授權提供；沒有要求公開使用者自己的圖片。

## 隨包來源

licenses/ImageMagick-7.1.2-34-selected-corresponding-source.zip 包含：
完整 ImageMagick、Configure，以及實際相依的 bzip2、freetype、fribidi、glib、harfbuzz、lcms、lqr、raqm、xml、zlib、png、jpeg-turbo、webp、ffi、jpeg-turbo-12、jpeg-turbo-16。
保留所有這些目錄內的來源、標頭、.ImageMagick 建置設定、授權及原有修改；只排除 Git 歷史與 GitHub 工作流程。
來源取自官方同版本 Windows.7z，下載檔 SHA-256 已與 GitHub asset digest 一致。
ZIP 內 SOURCE-PROVENANCE.json 記錄上游 URL、完整下載檔雜湊與選取範圍。
未以最新版上游原始碼冒充同版本對應來源。

## 重建指引

1. 將 ZIP 解壓至可寫入的開發資料夾。閱讀 README-IMBC-BUILD.md。
2. 安裝有合法使用資格的 Visual Studio C++ 桌面開發、ATL、MFC、Windows SDK；建立 x64 Release Dynamic、Q16、HDRI 組態。
3. 開啟 Configure/Configure.sln 建置 Configure 工具，執行其產生解決方案的精靈；使用已附來源，不執行會改成最新版的 clone 指令。
4. 對以下三個核心 LGPL DLL 的基線重建，只選本包附帶的 16 個相依目錄與 PNG/JPEG/WebP 模組；本版新增 AVIF／PSD 的完整重建另外依 AVIF-REBUILD.md；保留 .ImageMagick 設定中的靜態 ffi 及 JPEG 12/16 位元相依。
5. 建置產生的解決方案，輸出同 ABI 的 CORE_RL_*.dll；使用上游 LGPL 文字與 COPYING/ COPYING.LESSER。
6. 已於 2026-10-25 用隨包固定來源重建三個帶修改標記的 LGPL DLL，確認原匯出保留、實際載入及 PNG→WebP 成功；具體可重現命令見 evidence/lgpl/LGPL-REBUILD.ps1，勿執行通用 README 的 clone／更新指令。證據核對見 evidence/baseline-audit.json。本修正沿用相同 DLL 及來源，不重跑。

## 替換指引

先關閉 IMB-C，備份整個 Portable 資料夾。
以相同 x64、Q16-HDRI、同 DLL 名稱及相容匯出符號的新 DLL，替換 ImageMagick 目錄中的 CORE_RL_glib_.dll、CORE_RL_fribidi_.dll 或 CORE_RL_lqr_.dll。
如修改涉及相依 ABI，同時重建並替換相依 DLL／ImageMagick 核心。不要混用 x86 或 Static 組態。
重新驗證互轉與原檔不變。IMB-C 透過獨立 CLI 執行 ImageMagick，不需要重新連結 IMB-C 本身。
不需要改原 ImageMagick 安裝、系統 DLL 或 PATH。Windows 自有系統元件不在此替換範圍。

若將此包提供他人，須連同完整對應來源、所有聲明及授權文字一併交付；不要只發 EXE 或只有下載連結。
本次採隨包實體來源，沒有以尚未建立履行流程的「書面提供原始碼承諾」替代。
