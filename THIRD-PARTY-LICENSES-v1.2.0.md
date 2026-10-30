# IMB-C v1.2.0 第三方授權清單

本清單導覽隨包證據，不取代原文，不構成整包法律合規保證。日期 2026-10-28。

|範圍|版本/授權與完整清單位置|
|---|---|
|Slint|1.18.1，選 Royalty-free 2.0；licenses/rust 各 Slint crate/LICENSES，§2(a) 對照詳 FINAL-ACCEPTANCE-v1.2.0.md。|
|Rust 依賴|licenses/RUST-INVENTORY.csv、RUST-AUDIT.json、RUST-NOTICE-SOURCES.json；宣告 OR 不代表全選，選擇規則見 THIRD-PARTY-NOTICES.md；完整 notices 在 licenses/rust。|
|SHA-256 新增 crates|sha2 0.10.9 等七個，licenses/RUST-V1.2-ADDITIONS.json；MIT 選項及原文保留。|
|ImageMagick|7.1.2-34 Q16-HDRI，ImageMagick license；licenses/ImageMagick-LICENSE.txt、ImageMagick-THIRD-PARTY-NOTICES.txt、NATIVE-INVENTORY.json。全上游 SBOM 包含未出貨元件，實包以 NATIVE-DEPENDENCIES-v1.2.json 為準。|
|TIFF 新依賴|libtiff 4.7.2（libtiff license）、liblzma 5.8.4（0BSD）、Zstandard 1.5.7（BSD-3-Clause 選項）；licenses/v12-native。|
|JXL|libjxl 0.12.0（BSD-3-Clause）、Brotli 1.2.0（MIT）、Highway（Apache-2.0）；licenses/jxl-tools/LICENSE.*，AUTHORS/PATENTS 與 PROVENANCE.json；v12-native 保留 Brotli 原文。|
|專用 cjxl/djxl|0.12.0 官方 static；完整原附 notices 包含 libjxl/Brotli/Highway/giflib/libjpeg-turbo/libpng/libwebp/sjpeg/skcms/zlib，不聲稱所有元件均實際連入。|
|既有原生元件|licenses/NATIVE-INVENTORY.json 與 licenses/native；完整版權及版本隨原清單保留。|
|LGPL 元件|GLib 2.64.3、FriBidi 1.0.17（LGPL-2.1-or-later）、liblqr 0.4.2（保守按 LGPL v3）、libheif 1.23.6（LGPL-3.0-or-later）；完整相應來源、修改與重建材料在 licenses/、licenses/avif/，見 LGPL-REPLACEMENT.md、AVIF-REBUILD.md。libaom 3.15.2 選 BSD-2-Clause 並保留 PATENTS/AUTHORS/third-party。|
|Microsoft Runtime|14.51.36247.0，四名稱五副本，專有原文 licenses/microsoft；先依 RUNTIME-HANDOFF.md 提供完整固定條款並取得好友明確接受，再交付。|

完整隨包授權檔路徑、大小及 SHA-256 見 LICENSE-FILES-SHA256.csv；每個元件原有 NOTICE/版權與授權原文一併保留，不得以此摘要取代。
