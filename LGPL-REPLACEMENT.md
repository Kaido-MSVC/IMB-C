# IMB-C v2.0.0 LGPL 對應來源與替換方式

IMB-C 透過獨立程序呼叫 ImageMagick。下列 DLL 由 ImageMagick 或其委派模組動態連結。IMB-C 保留接收者修改及替換 LGPL 函式庫，以及為偵錯修改所需的反向工程權利。Microsoft Runtime 條款適用於 Microsoft 元件。

| 函式庫 | 適用版本 | 直接使用者 |
| --- | --- | --- |
| GLib 2.64.3，`CORE_RL_glib_.dll` | LGPL-2.1-or-later | liblqr、HarfBuzz |
| FriBidi 1.0.17，`CORE_RL_fribidi_.dll` | LGPL-2.1-or-later | Raqm |
| liblqr 0.4.2，`CORE_RL_lqr_.dll` | 採 LGPL v3。建置中繼資料為 -or-later，交付來源檔頭指定 v3 | ImageMagick MagickCore |
| libheif 1.23.7，`heif.dll` | LGPL-3.0-or-later | ImageMagick AVIF coder。libaom 3.15.2 靜態連入 heif.dll |

GLib、FriBidi 的 `COPYING` 和 liblqr 的 `COPYING`、`COPYING.LESSER` 位於 `licenses/native/` 對應目錄，libheif 的條文位於 `licenses/avif/libheif-COPYING`。來源包的授權目錄是 `native-materials/licenses/`。

## 對應來源

- GLib、FriBidi、liblqr 及其 Windows 建置設定隨 `ImageMagick-7.1.2-34-selected-corresponding-source.zip` 提供。v2.0.0 沿用這批 DLL，ImageMagick 應用程式及 coder 則改用 7.1.2-35。使用該 ZIP 的 `README-IMBC-BUILD.md` 及隨附 Configure，以 x64 Release Dynamic、Q16-HDRI 產生專案。使用已附版本，保留 GLib 的建置組態及靜態 libffi。
- ImageMagick 7.1.2-35 已修改的來源、來源索引、私有組態及建置材料位於 `licenses/security-20261113/`，AVIF／PSD 與組態隔離材料另見 `licenses/security-20261112/`。libheif 1.23.7 的上游來源、修改後來源、libaom 3.15.2 及 `SOURCES-1.23.7.json` 位於 `licenses/avif/`。重建時使用實際交付版本的修改與組態。見 [AVIF 重建紀錄](AVIF-REBUILD.md)。

## 使用隨附的 v2.0.0 執行檔

已交付的 GUI／CLI 把原生引擎 SHA-256 清單編入執行檔。冷快取會拒絕與內嵌摘要不符的相容 DLL，已有合格快取時則會繼續使用舊 DLL。變更 Portable 外部的 JSON、改用其他安裝位置，或把替換 DLL 放進快取，都無法使原 GUI／CLI 載入新 DLL。單獨執行 ImageMagick 可以測試替換的 ABI。

## 以修改後的 DLL 重建自己的 IMB-C

1. 解壓來源包及 Portable 到兩個新目錄。用對應來源建置相同 x64 ABI 和 DLL 名稱，檢查原本的匯出符號、相依項目及目標操作。若修改改變 ABI，也須重建消費該 ABI 的 ImageMagick 模組或核心。
2. 在這份 Portable 副本放入重建的 DLL，於解壓的 `source/RUNTIME-MANIFEST-v1.4.json` 更新其對應的 SHA-256。逐項核對其餘 runtime 雜湊。保留原封包與其已驗收的執行檔。
3. 依 [v2.0.0 建置說明](docs/build-2.0.0.md) 建置 GUI 與 CLI（靜態 CRT 由 `.cargo/config.toml` 設定）。把新 EXE 與相符的 runtime 放在同一份測試副本。隨附的 Microsoft Runtime 條款只涉及 Microsoft DLL，不限制 LGPL 函式庫的替換。使用小圖確認轉檔、來源 SHA-256 與實際快取的替換 DLL。清單雜湊改變後會使用另一個快取版本目錄，執行時仍會驗證 SHA-256。
4. 若需交付，連同適用條文、告知、實際對應來源及可重建或重新連結的材料一起提供。針對 LGPL 2.1 同時保留第 4、6 節要求的內容。針對 LGPL 3.0 保留第 4 節要求的對應應用程式碼與最小對應來源，並在適用時提供安裝資訊。修改的函式庫依各自授權交付。

參考 [GNU LGPL 2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html) 與 [GNU LGPL 3.0](https://www.gnu.org/licenses/lgpl-3.0.html)。
