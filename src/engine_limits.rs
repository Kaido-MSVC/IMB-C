//! 限制經由標準串流寫入的資料；不將像素快取限額誤稱為磁碟配額。
use std::{
    io::{self, Read, Write},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// 單一讀者持有匿名管線：只讀取已可用資料，避免其他工作持有 writer 時永久等 EOF。
/// 不與其他執行緒共用此讀取控制代碼；停止通知後最多再排空約兩秒。
pub(super) struct PipeReader<'a> {
    pipe: std::fs::File,
    stopped: &'a AtomicBool,
    stopped_at: Option<std::time::Instant>,
}
impl<'a> PipeReader<'a> {
    pub(super) fn new(pipe: std::fs::File, stopped: &'a AtomicBool) -> Self {
        Self {
            pipe,
            stopped,
            stopped_at: None,
        }
    }
}
impl Read for PipeReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn PeekNamedPipe(
                pipe: *mut std::ffi::c_void,
                data: *mut u8,
                size: u32,
                read: *mut u32,
                available: *mut u32,
                left: *mut u32,
            ) -> i32;
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let idle_since = std::time::Instant::now();
        loop {
            if self.stopped.load(Ordering::Acquire) {
                let start = self.stopped_at.get_or_insert_with(std::time::Instant::now);
                if start.elapsed() >= std::time::Duration::from_secs(2) {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "程序樹停止後管線仍未關閉，已停止讀取",
                    ));
                }
            }
            let mut available = 0;
            if unsafe {
                PeekNamedPipe(
                    self.pipe.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(109) {
                    Ok(0)
                } else {
                    Err(error)
                };
            }
            if available > 0 {
                let count = buffer.len().min(available as usize);
                return self.pipe.read(&mut buffer[..count]);
            }
            // 正在串流時先讓出時間片，避免每個短暫空窗都累加毫秒級延遲。
            if idle_since.elapsed() < std::time::Duration::from_millis(1) {
                std::thread::yield_now();
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}

pub(super) struct StopReaders<'a>(pub(super) &'a AtomicBool);
impl Drop for StopReaders<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub(super) const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024 * 1024;
pub(super) const STDERR_LIMIT: u64 = 1024 * 1024;
#[cfg(test)]
thread_local! { pub(super) static TEST_OUTPUT_LIMIT: std::cell::Cell<u64> = const { std::cell::Cell::new(OUTPUT_LIMIT) }; }
#[cfg(test)]
thread_local! { pub(super) static TEST_PANIC_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(super) fn output_limit() -> u64 {
    #[cfg(test)]
    {
        TEST_OUTPUT_LIMIT.with(|n| n.get())
    }
    #[cfg(not(test))]
    {
        OUTPUT_LIMIT
    }
}
const JOB_LIMIT: u64 = 32 * 1024 * 1024 * 1024;
/// 每次引擎執行，整個子程序樹經核心 I/O 計數、扣除標準串流後的累計寫入上限。
/// 涵蓋受控目錄以外的路徑與刪除後仍開啟的檔案；記憶體對應回寫不計入，
/// 且以輪詢終止，因此只是有界超出的累計寫入防線，不是檔案系統硬配額。
pub(super) const NATIVE_WRITE_LIMIT: u64 = 32 * 1024 * 1024 * 1024;
#[cfg(test)]
thread_local! { pub(super) static TEST_NATIVE_WRITE_LIMIT: std::cell::Cell<u64> = const { std::cell::Cell::new(NATIVE_WRITE_LIMIT) }; }
pub(super) fn native_write_limit() -> u64 {
    #[cfg(test)]
    {
        TEST_NATIVE_WRITE_LIMIT.with(|n| n.get())
    }
    #[cfg(not(test))]
    {
        NATIVE_WRITE_LIMIT
    }
}
const ACTIVE_LIMIT: u64 = 64 * 1024 * 1024 * 1024;
static ACTIVE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Budget {
    used: AtomicU64,
    limit: u64,
}
impl Budget {
    #[cfg(test)]
    pub(super) fn with_limit(limit: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            limit,
        }
    }
    pub(super) fn new() -> Self {
        Self {
            used: AtomicU64::new(0),
            limit: JOB_LIMIT,
        }
    }
    pub(super) fn reserve(&self, bytes: u64) -> io::Result<()> {
        let add = |used: u64, limit: u64| used.checked_add(bytes).filter(|n| *n <= limit);
        self.used
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| add(n, self.limit))
            .map_err(|_| io::Error::other("單檔累計串流寫入超過上限"))?;
        if ACTIVE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                add(n, ACTIVE_LIMIT)
            })
            .is_err()
        {
            self.used.fetch_sub(bytes, Ordering::Relaxed);
            return Err(io::Error::other("並行工作累計串流寫入超過上限"));
        }
        Ok(())
    }
}
impl Drop for Budget {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(*self.used.get_mut(), Ordering::Relaxed);
    }
}

pub(super) fn pump(
    mut input: impl Read,
    mut output: impl Write,
    limit: u64,
    budget: &Budget,
    failed: &AtomicBool,
    piped: &AtomicU64,
) -> io::Result<()> {
    struct PanicSignal<'a>(&'a AtomicBool);
    impl Drop for PanicSignal<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                self.0.store(true, Ordering::Relaxed);
            }
        }
    }
    let _panic_signal = PanicSignal(failed);
    #[cfg(test)]
    if TEST_PANIC_NEXT.with(|v| v.replace(false)) {
        panic!("injected stream reader panic");
    }
    let result = (|| {
        let mut written = 0u64;
        let mut buf = [0u8; 65536];
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            // 已自管線讀出的位元組；控制端以 Job 寫入總量扣除此值估算原生寫入。
            piped.fetch_add(n as u64, Ordering::Relaxed);
            if failed.load(Ordering::Relaxed) {
                return Err(io::Error::other("另一串流寫入失敗"));
            }
            written += n as u64;
            if written > limit {
                return Err(io::Error::other("引擎串流寫入超過上限"));
            }
            // 先保留額度才寫入；失敗也保留計數直到工作結束。
            budget.reserve(n as u64)?;
            output.write_all(&buf[..n])?;
        }
    })();
    if result.is_err() {
        failed.store(true, Ordering::Relaxed);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cap_precedes_write_and_budget_is_shared() {
        let budget = Budget {
            used: AtomicU64::new(0),
            limit: 10,
        };
        let failed = AtomicBool::new(false);
        let piped = AtomicU64::new(0);
        let mut out = Vec::new();
        pump(&b"123456"[..], &mut out, 6, &budget, &failed, &piped).unwrap();
        assert!(pump(&b"78901"[..], &mut out, 6, &budget, &failed, &piped).is_err());
        assert_eq!(out, b"123456");
        assert_eq!(budget.used.load(Ordering::Relaxed), 6);
        // 讀出即計入，即使後續因共享額度拒絕寫入，也反映子程序實際寫到管線的量。
        assert_eq!(piped.load(Ordering::Relaxed), 11);
        let mut out = Vec::new();
        assert!(
            pump(
                &b"1234567"[..],
                &mut out,
                6,
                &Budget::new(),
                &AtomicBool::new(false),
                &AtomicU64::new(0)
            )
            .is_err()
        );
        assert!(out.is_empty());
    }
    #[test]
    fn concurrent_reservations_cannot_exceed_job_limit() {
        let budget = Budget {
            used: AtomicU64::new(0),
            limit: 100,
        };
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        let _ = budget.reserve(1);
                    }
                });
            }
        });
        assert_eq!(budget.used.load(Ordering::Relaxed), 100);
    }
    #[test]
    fn failed_storage_signals_worker_without_retry() {
        struct Fails;
        impl Write for Fails {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("injected disk failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let failed = AtomicBool::new(false);
        assert!(
            pump(
                &b"x"[..],
                Fails,
                10,
                &Budget::new(),
                &failed,
                &AtomicU64::new(0)
            )
            .is_err()
        );
        assert!(failed.load(Ordering::Relaxed));
    }
}
