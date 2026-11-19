# 建置 IMB-C v2.0.0

使用 Windows x64、Rust 1.100.0 MSVC 工具鏈、MSVC 與 Windows SDK。參考建置使用 MSVC 14.52.37025、Windows SDK 10.0.28000.0 與 Slint 1.18.1。

進入來源包的 `source` 目錄執行：

```powershell
cargo build --locked --release --bin imb-c --bin imb-c-cli
```

首次建置需有 Cargo 相依快取或可連線至 crates.io。`.cargo/config.toml` 設定靜態 CRT。以 `RUSTFLAGS` 或 `CARGO_ENCODED_RUSTFLAGS` 覆寫時，須保留 `-C target-feature=+crt-static`。`build.rs` 會以 Windows SDK 的資源編譯器為兩個 EXE 加入版本資訊。

使用相符的 Portable runtime，依 `RUNTIME-MANIFEST-v1.4.json` 逐項核對 58 個檔案的 SHA-256。清單檔名沿用引擎版本命名，對應相同的 ImageMagick 7.1.2-35 與 JXL runtime。保留 Portable 的目錄結構，在副本中換入已建置的兩支 EXE。重新產生覆蓋整個 Payload 的 `SHA256SUMS.txt`，清單排除其本身，格式為 `小寫 SHA-256`、兩個空格及 `/` 分隔的相對路徑。

WiX 7.0.0 與同版 `WixToolset.UI.wixext` 可建置 MSI，命令與升級原則見 `installer/wix7-v2.0.0/README.md`。來源包的 `native-materials/licenses/` 包含對應來源、第三方條款與原生重建材料。替換 LGPL 元件請依 [替換方式](../LGPL-REPLACEMENT.md) 更新內嵌清單後重建。
