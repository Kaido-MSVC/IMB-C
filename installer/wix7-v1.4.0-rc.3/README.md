# RC3 MSI 建置

使用 WiX 7.0.0 與 WixToolset.UI.wixext 7.0.0。將 RC3 Portable 解壓至獨立目錄，並以同一個完整內容目錄作為 `-PayloadRoot`。

```powershell
pwsh ./installer/wix7-v1.4.0-rc.3/Build.ps1 -PayloadRoot <payload> -IconFile assets/app.ico -Generated <新目錄> -Output <新 MSI 路徑>
```

產生器先逐檔核對 Portable 清單，再建立安裝定義與完整授權對話方塊。Build.ps1 執行 WiX 編譯與 ICE 驗證。

沿用 RC2 的候選版 UpgradeCode 與家族互斥規則。RC3 使用專屬的 ProductCode、元件身分、程式目錄、開始功能表與登錄機碼。v1.3.3 屬於獨立的穩定版家族。MSI 會拒絕選用穩定版登錄所記載的安裝目錄。使用者偏好由應用程式管理，未列入 MSI 移除的檔案。
