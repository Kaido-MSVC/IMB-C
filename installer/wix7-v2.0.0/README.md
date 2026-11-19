# IMB-C v2.0.0 x64 MSI

使用 WiX 7.0.0 與 `WixToolset.UI.wixext` 7.0.0。以已核對的完整 Portable 目錄建置，輸出位置須是新檔案和新產生目錄。

```powershell
.\installer\wix7-v2.0.0\Build.ps1 -PayloadRoot <Portable> -IconFile .\assets\app.ico -Generated <產生目錄> -Output <MSI路徑>
```

正式版沿用穩定家族 UpgradeCode `3EAD7AE1-2433-4AB2-B14B-70A661D06781`、共用元件 GUID、安裝狀態與預設路徑。`MajorUpgrade` 在 `InstallInitialize` 後執行並阻止降版。一律使用新 ProductCode，候選版家族只偵測，請先移除已安裝的候選 MSI。維持完整 Runtime 條款畫面與執行階段同意檢查。
