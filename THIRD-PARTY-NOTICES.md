# 第三方告知

IMB-C 自有原始碼採 MIT；以下第三方元件仍依各自授權。歷史段落保留其當時版本與驗收脈絡，不限制 MIT 原始碼的公開散布。FreeType 2.14.3 選 FTL；署名見 FREETYPE-NOTICE.md，完整條款見隨包 `licenses/native/freetype/docs/FTL.TXT`。

## v1.2 新增元件

v1.2 additions (2026-10-28):
TIFF: libtiff 4.7.2 (Sam Leffler; Silicon Graphics, Inc.), libtiff license.
liblzma 5.8.4: Copyright (C) The XZ Utils authors and contributors, 0BSD for liblzma (not the upstream full-SBOM LGPL classification).
Zstandard 1.5.7: Copyright Meta Platforms, Inc. and affiliates, BSD-3-Clause selected.
ImageMagick JPEG XL delegate: libjxl 0.12.0, Copyright the JPEG XL Project Authors, BSD-3-Clause; Brotli 1.2.0, Copyright the Brotli Authors, MIT; Highway, Apache-2.0 (full notices retained conservatively).
Dedicated cjxl/djxl: official libjxl 0.12.0 Windows x64 static release. License set includes libjxl, Brotli, Highway, giflib, libjpeg-turbo, libpng, libwebp, sjpeg, skcms and zlib. Only cjxl.exe and djxl.exe are shipped; both import Windows KERNEL32.dll only.
Full new native license texts: licenses/v12-native and licenses/jxl-tools (including AUTHORS and PATENTS). Existing lcms/zlib/JPEG/WebP licenses remain in licenses/native.
Rust SHA-256: sha2 0.10.9, block-buffer 0.10.4, crypto-common 0.1.7, digest 0.10.7, cpufeatures 0.2.17, typenum 1.20.1 (MIT selected); generic-array 0.14.7 (MIT). Full copyright and license texts: licenses/rust/<crate-version>.
No Microsoft Runtime license change: existing explicit recipient acceptance process still applies before delivery.


以下保留 v1.1.1 基線告知；現行實包 PE 清單以 NATIVE-DEPENDENCIES-v1.2.json 為準。

v1.1.1 compliance-r1 現況：以 RELEASE-GATES.md、RUNTIME-RELEASE-REVIEW.md、COMPLIANCE-REVIEW.md 及 NATIVE-DEPENDENCIES-v1.1.json 為準。全部二進位沿用已審查的 v1.1.1 development candidate；相對舊版，CLI／AVIF／PSD coder 已重建。CLI 最新修改來源見 licenses/avif/modified-source.zip；licenses/cli-notice-source.zip 僅保留較早修復材料。

# 第三方告知與授權選擇

本套件為 v1.1.1 compliance-r1 文件補正候選，可依 RUNTIME-HANDOFF.md 的先同意後交付流程提供好友。

IMB-C 使用 ImageMagick 7.1.2-34 Q16-HDRI x64 Dynamic CLI。
ImageMagick 版權屬 ImageMagick Studio LLC；完整授權及上游告知位於 licenses/ImageMagick-LICENSE.txt、licenses/ImageMagick-THIRD-PARTY-NOTICES.txt。
本包保留 PNG、JPEG、WebP、AVIF（內部 heic 名稱）、PSD 共五個 coder modules；ImageMagick 核心仍依賴其他函式庫。
NATIVE-DEPENDENCIES-v1.1.json 記錄實際 28 個 PE 檔 SHA-256 與靜態 import 表，不包括 Windows 自有系統 DLL。
上游完整 SBOM 包含未隨包元件，不能當作本包 DLL 清單。

Slint 1.18.1 及適用 Slint 元件選用 Slint Royalty-free 2.0（不是 GPL 選項）。
它是有條件授權：此桌面程式整合 AboutSlint，且由頂層說明選單進入。
完整原文保存在 licenses/rust 各 Slint 元件的 LICENSES 目錄；不得移除署名。

Rust 實際目標依賴（含建置工具的保守上限）見 licenses/RUST-INVENTORY.csv。
對 MIT OR Apache-2.0 等選擇式授權，預設選 MIT；其他 OR 式選擇順序為 MIT、BSD、Apache-2.0、Zlib、0BSD、CC0、Unlicense。
AND 式授權須同時遵守，尤其 Apache-2.0 AND MIT 與 Unicode-3.0。
完整文字與原有 NOTICE 一併保存；少數 crate 未附文字，已依 .cargo_vcs_info.json 的固定提交從官方原始碼補齊，來源見 licenses/RUST-NOTICE-SOURCES.json。
此清單包含 build/proc-macro 元件；不主張全部都連結進最終 EXE。

Native 元件版本、版權、授權依據見 licenses/NATIVE-INVENTORY.json 與 licenses/native。
GLib 2.64.3、FriBidi 1.0.17 為 LGPL-2.1-or-later；liblqr 0.4.2 的建置檔列 LGPL-3.0-or-later，實際檔頭指定 LGPL v3，因此本包按 LGPL v3 保守處理。
libffi、PCRE 等靜態或內嵌相依的來源與原有聲明亦保留在對應原始碼 ZIP 中。

新增 heif.dll：libheif 1.23.6，LGPL-3.0-or-later；靜態連入 libaom 3.15.2，BSD-2-Clause，另附 PATENTS、AUTHORS 與 aom-third-party 原告知。授權／固定來源／修改來源見 licenses/avif，重建與替換見 AVIF-REBUILD.md。此候選不交付 OpenH264、libde265、x265 codec 實作；不能把上游完整 SBOM 當作實包。AOM 專利授權不構成全球零專利風險保證。

Microsoft VC Runtime：隨包四種名稱、五份原檔，DLL FileVersion 14.51.36247.0，來自 Community 2026 VC/Redist/MSVC/14.51.36231/x64 的 CRT／OpenMP；非從 System32 取用。實包與來源雜湊見 evidence/release-review-v1.1.1/IMB-C-v1.1.1-runtime-audit.json 及 RUNTIME-RELEASE-REVIEW.md。
發行者的個人資格依既有本人自述紀錄沿用；正式 Stable 通道、簽章及來源證據另存，不把安裝事實當資格證明。再散布依 Community 2026 與 REDIST 清單；接收者使用依完整 Runtime EULA（2025-10-01／Cpp_v14_ENU.1033），原件在 licenses/microsoft。
接收文件見 RUNTIME-RECIPIENT-TERMS.md；好友交付依 RUNTIME-HANDOFF.md 先同意、後給包。Microsoft 元件不改採 LGPL；LGPL 修改／替換／偵錯及來源權利仍保留。

本文件未取代各授權原文；任何未通過的檢核仍列於 RELEASE-GATES.md。


IMB-C v1.3.0 additional ImageMagick modules (2026-10-29):
IM_MOD_RL_miff_.dll, IM_MOD_RL_meta_.dll, IM_MOD_RL_rgb_.dll.
Unmodified official ImageMagick 7.1.2-34 Q16-HDRI modules, same MagickCore SHA-256 as the baseline.
License: ImageMagick License; existing full license/notice copies remain provided.
Corresponding source: licenses/ImageMagick-7.1.2-34-selected-corresponding-source.zip, coders/miff.c, meta.c, rgb.c.
Imports use already bundled MagickCore, bzip2, lzma, zlib and the existing Microsoft Runtime; no new library or Runtime license terms.
Exact hashes/provenance: NATIVE-ADDITIONS-v1.3.json. Rust dependency versions unchanged.

2026-10-30：FreeType 固定版原文與致謝、Microsoft DLL 來源及本人個人資格完成複核；公開二進位仍受接收流程限制。詳 LICENSE-REVIEW-v1.3.0.md。
