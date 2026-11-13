# v1.1 私有引擎重建／替換

## 2026-11-13 ImageMagick 7.1.2-35（IMB-C 1.3.3）

IMB-C 1.3.3 隨附的 ImageMagick 為 7.1.2-35：以 `scripts/build-imagemagick-7.1.2-35.py` 從官方 Windows 原始碼重建 MagickCore、MagickWand、magick.exe 與 10 個白名單 coder，並重新套用下方的 AVIF（heic coder 只註冊 AVIF、codec 執行緒 1）與 PSD 修改及私有組態隔離補丁。libheif 1.23.7、libaom 3.15.2 不變。來源與雜湊見 `third-party/materials/security-20261113/ImageMagick-7.1.2-35-PROVENANCE.json`。編譯使用 MSVC `/experimental:deterministic /pathmap`，不內嵌本機建置路徑；libjxl／Brotli（`scripts/build-jxl-brotli-native.py`）亦同。下方 7.1.2-34 敘述為歷史紀錄。

## 2026-11-12 安全修補（1.3.3 前置）

當時開發 runtime 已使用 libheif 1.23.7、libaom 3.15.2 與同版重建的 ImageMagick 7.1.2-34（隔天升級為 7.1.2-35，見上節）。來源、雜湊、私有組態隔離補丁及建置紀錄見 `third-party/materials/avif/SOURCES-1.23.7.json`、`third-party/materials/security-20261112/`、`scripts/prepare-security-native.py`、`scripts/build-isolated-magick-core.py`。請勿執行下方歷史 v1.1 流程來覆寫目前 runtime。
## 歷史 v1.1 固定來源與流程

固定來源：libheif 1.23.6、libaom 3.15.2；原始封存與 SHA-256 見 source/IMB-C-v1.2.0-source.zip 內 third-party/materials/avif、隨包 licenses/avif/SOURCES.json。
保留 ImageMagick 7.1.2-34 Q16-HDRI x64 動態核心與 PNG/JPEG/WebP 模組；重建 AVIF coder、PSD coder、授權告知 CLI。
HEIC 模組名稱僅為 ImageMagick 內部 AVIF 模組名稱。此變體只註冊 AVIF；HEIC/HEIF/AVCI 不註冊。libheif 保留 AVIF 所需容器程式碼，不含 HEVC/AVC codec 實作。

開發來源根目錄執行 `python -X utf8 scripts/rebuild-v11-engine.py`（Python 3.14；VS Community 18、MSVC 14.51.36231、Windows SDK 10.0.28000.0；CMake 為 VS 內附版本）。工具預設路徑列於 scripts，工具鏈位置不同時可修改；不需下載其他 codec、不安裝套件或改系統 PATH。
產物：work/heif-install/bin/heif.dll、work/rebuilt-modules/{magick.exe,IM_MOD_RL_heic_.dll,IM_MOD_RL_psd_.dll}。
僅在自己的候選副本替換相應檔案；保持 x64、Q16-HDRI、動態 CRT 與匯出相容，重新檢查依賴與主要格式。不要覆蓋原 v1.0.1 或舊 Personal-Use。
未追求位元組完全可重現（編譯時間戳／路徑可不同）。新 libheif 使用標準 heif.dll 名稱，AVIF coder 已依其 import library 重建，未將檔案改名冒充舊 CORE_RL_heif_.dll。

libheif 修改：AOM encoder 預設 threads=1；停用其他 codec、外掛載入與平行 tile 解碼。AVIF coder 明確 num_codec_threads=1。ImageMagick/OpenMP thread=1 與 Rayon 最多兩個工作分別保留。
PSD 修改：指定合成圖輸入時拒絕無合成圖檔案；單圖層寫入後為合成圖 RGB 混白，保留 alpha，對應讀取時的 alpha-unblend。修改後來源另附 licenses/avif/modified-source.zip。
CLI 已新增 libheif LGPL-3.0-or-later 告知、作者、授權與來源／替換位置；磁碟 THIRD-PARTY-RUNTIME.txt 亦包含 libheif／libaom。v1.1.1 EXE 內編入的 GUI 摘要仍為舊內容，本輪未改 UI 或重編；不將它描述為已新增告知。libaom 保留 BSD-2-Clause、PATENTS 與第三方告知。
允許修改、替換 LGPL 元件及為偵錯修改所需的反向工程；Microsoft Runtime 條款不限制此權利。Runtime 本身保持正式 Redist 原檔及原交付同意流程。

