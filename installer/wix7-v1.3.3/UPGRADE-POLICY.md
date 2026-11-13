# v1.3.3 安裝路徑政策
相同 UpgradeCode（3EAD7AE1-2433-4AB2-B14B-70A661D06781），新的 ProductCode 823B3A63-FB39-4175-8094-F8010EDDFE34，ProductVersion 1.3.3（1.3.2 的 ProductCode 為 61903C56-091E-486A-B0A9-581C67C010B8；開發期間的 1.3.3 候選包另用 78E77014-…、32E539D8-…、B2823901-…，未對外發布，正式版不沿用）。MajorUpgrade 移除 1.3.2 及更早版本並阻止降版；安裝路徑、元件 GUID 推導與條款同意流程沿用 v1.3.0，未變更。
以下為 v1.3.0 原政策，沿用：保留完整授權與標準 UI。
使用 RegistrySearch(directory)+DirectorySearch 僅接受既存舊路徑；使用 Type 51 SetProperty 在 AppSearch 之後、CostInitialize 之前設定 INSTALLFOLDER。避免舊 Type 35 SetDirectory 在 CostFinalize 前才判斷 NOT INSTALLFOLDER 而遇到預設值已初始化。
Sequence=first 避免 UI 選定後在執行階段重置；明示 INSTALLFOLDER 參數或 UI 修改優先。修復仍由現有 Windows Installer component 資料與安裝狀態維持。
舊磁碟/目錄不存在時搜索不命中，回到預設目錄；先前隔離測試已通過整個目錄遺失修復，本次安裝邏輯不變。
完整解除安裝後重裝：評估後本版不保留永久 MSI 元件或 HKLM 路徑殘留，完整解除安裝會移除安裝紀錄。使用者的 LocalAppData 偏好仍可保留。更新/修復沿用路徑與卸載後重裝記憶是不同情境。
不得對正式安裝進行自動測試。沒有隔離環境時只宣稱編譯、表格/時序與 ICE 驗證，列出端到端安裝測試限制。

正式版使用新 ProductCode 60A17426-9610-4EC4-BD3B-65E6BFC78A46；候選未對外發布。同版候選須先卸載，不開啟 AllowSameVersionUpgrades。正式封裝只增補材料／更新條款文字，不更動安裝路徑與升級控制。
