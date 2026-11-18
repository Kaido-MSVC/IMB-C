# IMB-C 1.4.0-rc.2：Microsoft Runtime 交付說明

適用條款：`RUNTIME-RECIPIENT-TERMS.md`（固定版本 `IMBC-RUNTIME-1.4.0-rc.2-r3`），內含 Microsoft 官方條款全文；官方 DOCX 原件在 `licenses/microsoft/Runtime-2026.docx`。

## Microsoft 條款的要求

依 Visual Studio Community 2026 條款第 5 節 Distributable Code：隨包 DLL 不得修改；只能隨具有重要主要功能的應用一起散布；散布者須要求外部終端使用者及其他散布者同意保護程度不低於 Microsoft 條款的條款；發行者對 Microsoft 負擔補償義務；不得散布 Debug、preview 或 Unreleased 元件，也不得暗示 Microsoft 背書。條款沒有規定同意的形式、時點或紀錄方式。

## IMB-C 的做法

| 交付方式 | 條款如何提供與同意 |
| --- | --- |
| MSI | 標準授權對話方塊顯示完整條款（`RUNTIME-RECIPIENT-TERMS.md` 排第一份），須勾選同意才能安裝；靜默安裝須提供 `ACCEPTTERMS=1` |
| Portable ZIP | 條款檔位於封包根目錄；條款載明使用即表示同意，不同意請勿使用 |
| 發行頁／交付訊息 | 須寫明封包含 Microsoft Visual C++ Runtime、使用即表示同意隨附條款，並附上 `RUNTIME-RECIPIENT-TERMS.md` 與官方條款連結 |

應用程式本身不顯示條款視窗、不記錄同意狀態，也不因此限制轉檔。1.4.0-rc.2 先前版本留下的 `%LOCALAPPDATA%\IMB-C\runtime-terms-acceptance.json` 不再讀取，也不會被刪除；不需要時可自行刪除。

## 發行者交付

- 只交付完整封包（Portable ZIP 或 MSI），不要單獨提供或拆出 Microsoft DLL，也不要移除授權、來源、NOTICE、PATENTS 或重建材料。
- 發行頁建議文字：「本封包含 Microsoft Visual C++ Runtime（ImageMagick 目錄內四份未修改的 DLL）。下載、安裝或使用即表示您同意隨附的 `RUNTIME-RECIPIENT-TERMS.md` 及 Microsoft 官方條款：https://visualstudio.microsoft.com/license-terms/vs2026-ga-visualcpp-v14-redist-runtime/ 」
- 不需要逐人取得回覆。
- LGPL 元件的修改、替換、偵錯與再散布權利不受 Microsoft 條款限制，見 `LGPL-REPLACEMENT.md`。
