# IMB-C v1.3.0 最後授權審查（2026-10-30）

|項目|判斷|依據與界線|
|---|---|---|
|MIT 署名／metadata|GO|本人指定 Copyright (c) 2026 Kaido-MSVC；LICENSE、README、Cargo.toml 一致。MIT 只涵蓋自有程式碼。|
|FreeType 2.14.3|GO|選 FTL；固定 VER-2-14-3 的 LICENSE.TXT、docs/FTL.TXT 原 bytes 與必要致謝完整保留，附屬 BDF/PCF、zlib、HarfBuzz 告知亦附上。年份 2026 依固定版 freetype.h。|
|Microsoft DLL 來源|GO|五份／四名稱、14.51.36247.0；均與正式 Community 2026 VC/Redist/MSVC/14.51.36231/x64 CRT 或 OpenMP 相同，簽章 Valid，非 preview/debug_nonredist。|
|Microsoft 發行者資格|GO（依本人聲明）|2026-10-30 本人確認個人開發、非代公司／組織，並已接受正式版 Community 條款。個人條款允許開發自身應用；安裝或簽章本身不能證明資格。|
|自有原始碼 GitHub 候選|GO（授權範圍）|保留第三方告知與授權；精簡來源不含 Microsoft Runtime DLL。公開二進位時仍須同步供應完整 LGPL 相應來源與替換材料。此判斷不等於已公開。|
|按既有流程個別交付二進位|GO（有條件）|接收者先讀完整固定 Runtime 條款並明確同意，再交付完整封包；不得虛構同意。|
|直接在 GitHub 公開 Portable／MSI|NO-GO|既有好友交付文件要求先同意後交付；任意公開下載尚未實作相容的接收流程。MSI 安裝時有同意機制，但現行文件明定不能取代交付前流程。|

## 固定官方證據

- FreeType: https://github.com/freetype/freetype/tree/VER-2-14-3 ; docs/FTL.TXT SHA-256 `5a5ee54c5001bbad1cdc1a57cc3dd4c42199b2da09d39c7ee41fab002d02967f`；LICENSE.TXT `bd36c8b474855fa294c2ec5c184544478ef3720aad37d65a6296a4f264fd2d3b`。完整來源 URL/hash 見 FreeType/OFFICIAL-SOURCES.json。
- Community 官方入口：https://visualstudio.microsoft.com/license-terms/vs2026-ga-community/ 。當日下載官方 DOCX 與隨包副本相同，SHA-256 `02ac07637f4a513a807cd3721c7620433fc483bc049d25784b0c10c54b05e1fb`。Individual License 及 Distributable Code 條款為發行依據。
- Runtime 官方入口：https://visualstudio.microsoft.com/license-terms/vs2026-ga-visualcpp-v14-redist-runtime/ 。當日下載原件與隨包相同，SHA-256 `08651651a7602fc7c0e2763de0fde1ff9f868df2780597cd1775ee9d6441c783`。此接收者使用條款本身不授予再散布權；發行權來自 Community 與 REDIST。
- REDIST：https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution 。有效授權者可隨程式散布 VC/redist 指定未修改原檔；預發行與 debug_nonredist 排除。

Community 條款另要求應用具主要功能、外部使用者與散布者同意至少同等保護的條款，並由發行者負擔所載補償／防禦義務；不得暗示 Microsoft 背書，亦不得使 Microsoft 元件受要求公開原始碼或允許修改的授權拘束。這些限制只適用 Microsoft 元件，不剝奪 LGPL 替換／偵錯權利。

## 剩餘界線

公開二進位仍需完成符合上述條款的公開接收流程，或另提供經驗證、不含 Microsoft DLL 的發行方式；本輪未更動應用功能以實作這項流程。正式產品識別 MSI 仍未直接安裝於現有正式環境；沿用前輪隔離生命週期驗收，不重跑轉檔。未簽章、專利及各地強制法律風險不因本審查消失。這是對指定版本及證據的發行判斷，不是全球法律或專利清權保證。
