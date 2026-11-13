//! committed-memory 硬限制（Job Object）與跨執行個體並行預算。不是像素快取或 RSS 限制。
//! 2026-11-13：預算改由 instance_ledger 跨程序登記，同一台電腦上多個 IMB-C 程序
//! 不再各自以同一份可用 commit 核准工作。公式維持原保守版本（見交接文件）。
use super::instance_ledger;
use std::io;
const MIB: u64 = 1024 * 1024;
#[repr(C)]
#[derive(Default)]
struct MemoryStatus {
    length: u32,
    load: u32,
    total_phys: u64,
    avail_phys: u64,
    total_page: u64,
    avail_page: u64,
    total_virtual: u64,
    avail_virtual: u64,
    avail_extended: u64,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
}

pub(super) struct Lease(
    pub(super) usize,
    #[allow(dead_code)] instance_ledger::Reservation,
);
impl Lease {
    pub(super) fn acquire(
        requested_mib: u64,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let requested = match std::env::var("IMBC_ENGINE_MEMORY_MIB") {
            Ok(value) => value
                .parse::<u64>()
                .ok()
                .filter(|n| (256..=65536).contains(n))
                .ok_or("IMBC_ENGINE_MEMORY_MIB 須為 256–65536 的整數；不允許取消記憶體防護")?,
            Err(std::env::VarError::NotPresent) => requested_mib,
            Err(_) => return Err("IMBC_ENGINE_MEMORY_MIB 無法讀取".into()),
        } * MIB;
        let ledger = instance_ledger::global().map_err(|e| e.to_string())?;
        loop {
            check()?;
            let mut status = MemoryStatus {
                length: std::mem::size_of::<MemoryStatus>() as u32,
                ..Default::default()
            };
            if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            // 不依賴分頁檔自動擴充：僅採用當下 available commit，另留至少 2 GiB。
            let ceiling =
                (status.total_phys * 3 / 4).min(status.avail_page.saturating_sub(2048 * MIB));
            if requested > status.total_phys * 3 / 4 {
                return Err(format!(
                    "此操作需要設定 {} MiB committed-memory 上限，超過本機實體記憶體的 75%；請使用記憶體較充足的設備，或依實測以 IMBC_ENGINE_MEMORY_MIB 指定較小的受控上限",
                    requested / MIB
                ));
            }
            match ledger
                .try_reserve(instance_ledger::Resource::Memory, requested, ceiling)
                .map_err(|e| format!("跨執行個體記憶體預留失敗：{e}"))?
            {
                Ok(reservation) => return Ok(Self(requested as usize, reservation)),
                Err(0) => {
                    return Err(format!(
                        "系統可用 committed-memory 不足：操作上限 {} MiB，當下可安全提供 {} MiB；未啟動引擎",
                        requested / MIB,
                        ceiling / MIB
                    ));
                }
                // 其他工作（可能在其他 IMB-C 程序）持有預留：等待其釋放；總期限與取消由 check 控制。
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(40)),
            }
        }
    }
}

// 先以保守的有限級距處理未知／未量測操作；超出驗證範圍保留受控手動上限。
// 級距是操作預算，不宣稱能由標頭預測任意 codec 的最壞配置量。
pub(super) fn profile(pixels: u64, depth: u64, channels: u64, args: &[&str]) -> u64 {
    let encode = if args.first() == Some(&"identify") {
        ""
    } else {
        args.last().copied().unwrap_or("")
    };
    let decode = args
        .iter()
        .find(|s| s.contains(":-"))
        .copied()
        .unwrap_or("");
    let large = pixels > 20_000_000;
    let wide = depth > 8 || channels > 3;
    match (encode, decode, large) {
        ("avif:-", _, true) => {
            if wide {
                10240
            } else {
                9216
            }
        }
        ("jxl:-", _, true) => 8192,
        ("webp:-", _, true) => 4096,
        (_, "avif:-", true) => 4096,
        (_, "jxl:-", true) => 4096,
        (_, "webp:-", true) => 3072,
        ("avif:-" | "jxl:-", _, false) => 3072,
        _ => 2048,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoder_metadata_is_not_mistaken_for_encoder_budget() {
        use super::profile;
        assert_eq!(std::mem::size_of::<super::MemoryStatus>(), 64);
        assert_eq!(profile(100_000_000, 8, 4, &["identify", "avif:-"]), 4096);
        assert_eq!(profile(100_000_000, 8, 4, &["avif:-", "miff:-"]), 4096);
        assert_eq!(profile(100_000_000, 8, 4, &["miff:-", "avif:-"]), 10240);
        assert!(
            profile(100_000_000, 16, 4, &["miff:-", "avif:-"])
                >= profile(100_000_000, 8, 3, &["miff:-", "avif:-"])
        );
    }
}
