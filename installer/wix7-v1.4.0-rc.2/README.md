# IMB-C 1.4.0-rc.2 MSI（Runtime 更新版）

WiX 7.0.0／UI extension 7.0.0。`make-msi-source.py` 由 `wix7-v1.3.3` 的產生器複製而來，只變更下列項目：

- 產品身分：RC2 產品系列 UpgradeCode `C897A5DB-A859-46A6-BEA9-50585AE110ED`，本次重建（簡化 Runtime 條款流程）的新 ProductCode `46BDFC27-39F0-4717-B455-9254309F59B3`（先前 RC2 MSI 依序為 `CC2F9898-9DCB-440F-AC83-0DD912B72D9C`、`4010481F-F615-48CE-8A9B-9B734351A7D7`），ProductVersion `1.4.0`，顯示名稱 `IMB-C 1.4.0-rc.2 Personal Use`，安裝目錄、捷徑與登錄機碼使用 `IMB-C 1.4 RC2`。
- 升級政策沿用 `wix7-v1.3.3/UPGRADE-POLICY.md`：同版候選不自動升級，不開啟 AllowSameVersionUpgrades。新增偵測：已安裝同一 RC2 產品系列的任何版本時，安裝程式顯示訊息並停止，請先解除安裝。
- 條款：`RUNTIME-RECIPIENT-TERMS.md`（含 Microsoft 官方條款全文）排在第一份，說明文字改為 RC2 的公開下載情境，移除好友交付描述；首次安裝須在標準授權對話方塊勾選同意（靜默安裝為 `ACCEPTTERMS=1`）。這是 Runtime 條款唯一的同意步驟，應用程式啟動時不再另外詢問。
- 產生的 `Release.wxs`、`Installer-Terms.*`、`TERMS-INVENTORY.json` 寫到另外指定的新目錄，不寫入本資料夾。

建置：`pwsh installer/wix7-v1.4.0-rc.2/Build.ps1 -PayloadRoot <payload> -IconFile assets/app.ico -Generated <新目錄> -Output <新 MSI 路徑>`。payload 必須含 `SHA256SUMS.txt`，產生器會逐檔核對。不得對正式安裝進行自動測試；乾淨 Windows 的安裝生命週期仍未驗證。
