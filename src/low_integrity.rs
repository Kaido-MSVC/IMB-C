//! 僅用於受控引擎：唯讀輸入、管線輸出、移除特權、Low IL、原子 Job 納管。
//!
//! v1.4.0-rc.2 起每次引擎呼叫另建獨立 AppContainer（Windows 8+ 正式機制）：
//! - 檔案存取除了一般 DACL 檢查外，還要通過 AppContainer SID／capability 的第二次檢查，
//!   因此未明確授權的使用者檔案（讀取亦同）、其他工作的暫存、LocalLow、無 ACL 的
//!   卸除式 FAT／exFAT 磁碟區與 SMB 共用都會被拒絕（2026-11-18 實測）。
//! - runtime 目錄只授權自訂 capability 唯讀；暫存只授權該次呼叫的 AppContainer SID。
//! - profile 在程序恢復前即刪除，不留下 AppContainer 專屬可寫資料夾。
use std::{
    ffi::{OsStr, c_void},
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    process::{Command, ExitStatus},
};
type Handle = *mut c_void;
// 僅序列化本啟動器短暫可繼承控制代碼的存活區間；其他啟動器須自行協調。
static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 普通權限外部工具：原子加入獨立 Job，明確繼承三個標準串流；不套用 Low IL Token。
pub(super) fn spawn_regular(
    executable: &std::path::Path,
    args: &[std::ffi::OsString],
    output: &File,
    error: &File,
    job: Handle,
) -> io::Result<Child> {
    if !executable.is_absolute() {
        return Err(io::Error::other("外部工具必須為絕對路徑"));
    }
    let app = wide(executable)?;
    let mut line = quote(executable.as_os_str());
    for arg in args {
        line.push(32);
        line.extend(quote(arg));
    }
    if line.contains(&0) {
        return Err(io::Error::other("外部工具引數含 NUL"));
    }
    line.push(0);
    let cwd = wide(
        executable
            .parent()
            .ok_or_else(|| io::Error::other("工具缺少父目錄"))?,
    )?;
    let _lock = SPAWN_LOCK
        .lock()
        .map_err(|_| io::Error::other("子程序建立鎖定不可用"))?;
    let null = File::open("NUL")?;
    let input = duplicate(null.as_raw_handle(), 0x80000000)?;
    let output = duplicate(output.as_raw_handle(), 0x40000000)?;
    let error = duplicate(error.as_raw_handle(), 0x40000000)?;
    let mut handles = [
        input.as_raw_handle(),
        output.as_raw_handle(),
        error.as_raw_handle(),
    ];
    let mut jobs = [job];
    let mut attrs = Attributes::new(2)?;
    attrs.set(0x20002, &mut handles)?;
    attrs.set(0x2000d, &mut jobs)?;
    let mut startup = StartupEx {
        startup: Startup {
            size: std::mem::size_of::<StartupEx>() as u32,
            flags: 0x100,
            input: handles[0],
            output: handles[1],
            error: handles[2],
            ..Default::default()
        },
        attributes: attrs.ptr(),
    };
    let mut info = ProcessInfo::default();
    // JOB_LIST 在建立時納管，沒有建立後再指派的孤兒空窗；失敗不回退。
    check(unsafe {
        CreateProcessW(
            app.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1,
            0x08080000,
            std::ptr::null_mut(),
            cwd.as_ptr(),
            &mut startup,
            &mut info,
        )
    })?;
    let _thread = unsafe { OwnedHandle::from_raw_handle(info.thread) };
    Ok(Child {
        process: unsafe { OwnedHandle::from_raw_handle(info.process) },
        stdout: None,
        stderr: None,
    })
}
#[cfg(test)]
thread_local! { pub(super) static TEST_REJECT_BEFORE_RESUME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(test)]
thread_local! {
    pub(super) static TEST_HOLD_PIPE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static TEST_HELD_PIPE: std::cell::RefCell<Option<File>> = const { std::cell::RefCell::new(None) };
}
#[repr(C)]
struct SidAttributes {
    sid: Handle,
    attributes: u32,
}
#[repr(C)]
#[derive(Default)]
struct Startup {
    size: u32,
    reserved: Handle,
    desktop: Handle,
    title: Handle,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    chars_x: u32,
    chars_y: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_bytes: Handle,
    input: Handle,
    output: Handle,
    error: Handle,
}
#[repr(C)]
struct StartupEx {
    startup: Startup,
    attributes: Handle,
}
#[repr(C)]
#[derive(Default)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}
#[repr(C)]
struct SecurityCapabilities {
    app_container: Handle,
    capabilities: *mut SidAttributes,
    count: u32,
    reserved: u32,
}
#[repr(C)]
struct Trustee {
    multiple: Handle,
    operation: u32,
    form: u32,
    kind: u32,
    name: Handle,
}
#[repr(C)]
struct ExplicitAccess {
    permissions: u32,
    mode: u32,
    inheritance: u32,
    trustee: Trustee,
}
#[link(name = "userenv")]
unsafe extern "system" {
    fn CreateAppContainerProfile(
        name: *const u16,
        display: *const u16,
        description: *const u16,
        capabilities: *const SidAttributes,
        count: u32,
        sid: *mut Handle,
    ) -> i32;
    fn DeleteAppContainerProfile(name: *const u16) -> i32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn FreeSid(sid: Handle) -> Handle;
    fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
    fn GetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        information: u32,
        owner: *mut Handle,
        group: *mut Handle,
        dacl: *mut Handle,
        sacl: *mut Handle,
        descriptor: *mut Handle,
    ) -> u32;
    fn SetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        information: u32,
        owner: Handle,
        group: Handle,
        dacl: Handle,
        sacl: Handle,
    ) -> u32;
    fn SetEntriesInAclW(
        count: u32,
        entries: *const ExplicitAccess,
        old: Handle,
        new: *mut Handle,
    ) -> u32;
    fn RegOpenKeyExW(
        key: Handle,
        subkey: *const u16,
        options: u32,
        access: u32,
        result: *mut Handle,
    ) -> i32;
    fn RegEnumKeyExW(
        key: Handle,
        index: u32,
        name: *mut u16,
        name_size: *mut u32,
        reserved: Handle,
        class: Handle,
        class_size: Handle,
        time: Handle,
    ) -> i32;
    fn RegGetValueW(
        key: Handle,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: Handle,
        data: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn CreateRestrictedToken(
        token: Handle,
        flags: u32,
        disabled_count: u32,
        disabled: *const SidAttributes,
        privilege_count: u32,
        privileges: Handle,
        restricted_count: u32,
        restricted: *const SidAttributes,
        result: *mut Handle,
    ) -> i32;
    fn ConvertStringSidToSidW(text: *const u16, sid: *mut Handle) -> i32;
    fn SetTokenInformation(token: Handle, class: u32, data: Handle, size: u32) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        data: Handle,
        size: u32,
        required: *mut u32,
    ) -> i32;
    fn GetSidSubAuthority(sid: Handle, index: u32) -> *mut u32;
    fn EqualSid(first: Handle, second: Handle) -> i32;
    fn CreateProcessAsUserW(
        token: Handle,
        app: *const u16,
        command: *mut u16,
        process_attributes: Handle,
        thread_attributes: Handle,
        inherit: i32,
        flags: u32,
        environment: Handle,
        cwd: *const u16,
        startup: *mut StartupEx,
        info: *mut ProcessInfo,
    ) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut Handle,
        size: *mut u32,
    ) -> i32;
    fn GetSecurityDescriptorDacl(
        descriptor: Handle,
        present: *mut i32,
        acl: *mut Handle,
        defaulted: *mut i32,
    ) -> i32;
    fn GetSecurityDescriptorSacl(
        descriptor: Handle,
        present: *mut i32,
        acl: *mut Handle,
        defaulted: *mut i32,
    ) -> i32;
    fn SetSecurityInfo(
        object: Handle,
        kind: u32,
        information: u32,
        owner: Handle,
        group: Handle,
        dacl: Handle,
        sacl: Handle,
    ) -> u32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateProcessW(
        app: *const u16,
        command: *mut u16,
        process_attributes: Handle,
        thread_attributes: Handle,
        inherit: i32,
        flags: u32,
        environment: Handle,
        cwd: *const u16,
        startup: *mut StartupEx,
        info: *mut ProcessInfo,
    ) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn LocalFree(pointer: Handle) -> Handle;
    fn DuplicateHandle(
        source_process: Handle,
        source: Handle,
        target_process: Handle,
        target: *mut Handle,
        access: u32,
        inherit: i32,
        flags: u32,
    ) -> i32;
    fn CreatePipe(read: *mut Handle, write: *mut Handle, attributes: Handle, size: u32) -> i32;
    fn InitializeProcThreadAttributeList(
        list: Handle,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: Handle,
        flags: u32,
        attribute: usize,
        value: Handle,
        size: usize,
        previous: Handle,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: Handle);
    fn ResumeThread(thread: Handle) -> u32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
    fn WaitForSingleObject(object: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn GetCurrentProcessId() -> u32;
    #[cfg(test)]
    fn GetProcessId(process: Handle) -> u32;
}
fn wide(value: impl AsRef<OsStr>) -> io::Result<Vec<u16>> {
    let mut value: Vec<u16> = value.as_ref().encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "子程序參數含 NUL",
        ));
    }
    value.push(0);
    Ok(value)
}
fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
struct Local(Handle);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
/// 受限 Token 的預設擁有者（TokenOwner）與預設 DACL（TokenDefaultDacl）改為使用者本人與 SYSTEM。
fn set_user_defaults(token: Handle, container: &str) -> io::Result<()> {
    let mut size = 0;
    unsafe {
        GetTokenInformation(token, 1, std::ptr::null_mut(), 0, &mut size); // TokenUser
    }
    if (size as usize) < std::mem::size_of::<SidAttributes>() {
        return Err(io::Error::other("Token 使用者大小錯誤"));
    }
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    check(unsafe { GetTokenInformation(token, 1, data.as_mut_ptr().cast(), size, &mut size) })?;
    let user = unsafe { &*data.as_ptr().cast::<SidAttributes>() }.sid;
    let user_text = sid_string(user)?;
    // AppContainer 程序必須能存取自己以預設 DACL 建立的物件，否則 DLL 初始化失敗
    // （0xC0000142）；只加入本次呼叫的 AppContainer SID，不授權其他工作。
    set_default_dacl(
        token,
        &format!("D:(A;;GA;;;{user_text})(A;;GA;;;SY)(A;;GA;;;{container})"),
    )?;
    let mut owner = user; // TOKEN_OWNER { PSID }
    check(unsafe {
        SetTokenInformation(
            token,
            4,
            (&mut owner as *mut Handle).cast(),
            std::mem::size_of::<Handle>() as u32,
        )
    })
}
fn set_default_dacl(token: Handle, sddl: &str) -> io::Result<()> {
    let text = wide(sddl)?;
    let mut sd = std::ptr::null_mut();
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &mut sd,
            std::ptr::null_mut(),
        )
    })?;
    let sd = Local(sd);
    let (mut present, mut defaulted, mut dacl) = (0, 0, std::ptr::null_mut());
    check(unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted) })?;
    if present == 0 || dacl.is_null() {
        return Err(io::Error::other("預設 DACL 建立失敗"));
    }
    let mut value = dacl; // TOKEN_DEFAULT_DACL { PACL }
    check(unsafe {
        SetTokenInformation(
            token,
            6,
            (&mut value as *mut Handle).cast(),
            std::mem::size_of::<Handle>() as u32,
        )
    })
}
fn sid(text: &str) -> io::Result<Local> {
    let mut value = std::ptr::null_mut();
    check(unsafe { ConvertStringSidToSidW(wide(text)?.as_ptr(), &mut value) })?;
    Ok(Local(value))
}
fn sid_string(sid: Handle) -> io::Result<String> {
    let mut text = std::ptr::null_mut();
    check(unsafe { ConvertSidToStringSidW(sid, &mut text) })?;
    let owner = Local(text.cast());
    let value = unsafe {
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    drop(owner);
    Ok(value)
}

/// 自訂 capability「imbc-runtime」（DeriveCapabilitySidsFromName 的固定結果，
/// 由單元測試核對）。只在 runtime 快取（`runtime_cache`）的檔案與目錄上授權唯讀，
/// 不授權 ALL APPLICATION PACKAGES，也不修改使用者的 Portable 目錄。
pub(super) const RUNTIME_CAPABILITY: &str = "S-1-15-3-1024-498898926-2095900455-2891598938-3761174777-234801964-725166658-3646080884-483418499";
/// AppContainer profile 名稱字首；含建立者 PID，供崩潰後只回收已結束程序的殘留。
const PROFILE_PREFIX: &str = "IMB-C.Engine.";

/// 單次引擎呼叫專屬的 AppContainer。profile 在子程序恢復前刪除；SID 仍用於授權
/// 本次暫存目錄與驗證子程序 Token。
struct Container {
    name: Vec<u16>,
    sid: Handle,
    sid_text: String,
    profile: std::sync::atomic::AtomicBool,
}
// SID 只在本結構存活期間讀取，不跨執行緒修改。
unsafe impl Send for Container {}
unsafe impl Sync for Container {}
impl Container {
    fn create() -> io::Result<Self> {
        use std::hash::{BuildHasher, Hasher};
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let pid = unsafe { GetCurrentProcessId() };
        for _ in 0..4 {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            let text = format!("{PROFILE_PREFIX}{pid}.{:016x}", hasher.finish());
            let name = wide(&text)?;
            let mut sid = std::ptr::null_mut();
            let hr = unsafe {
                CreateAppContainerProfile(
                    name.as_ptr(),
                    name.as_ptr(),
                    wide("IMB-C isolated image engine job")?.as_ptr(),
                    std::ptr::null(),
                    0,
                    &mut sid,
                )
            };
            // 已存在代表名稱碰撞；絕不沿用，以免與其他工作共用 SID。
            if hr == 0x800700b7u32 as i32 {
                continue;
            }
            if hr != 0 {
                return Err(io::Error::other(format!(
                    "無法建立 AppContainer profile（HRESULT {hr:#010x}）"
                )));
            }
            // 先交給 Drop 管理，後續任何失敗都會刪除 profile 並釋放 SID。
            let mut container = Self {
                name,
                sid,
                sid_text: String::new(),
                profile: std::sync::atomic::AtomicBool::new(true),
            };
            container.sid_text = sid_string(sid)?;
            if !container.sid_text.starts_with("S-1-15-2-") {
                return Err(io::Error::other("AppContainer SID 格式異常"));
            }
            return Ok(container);
        }
        Err(io::Error::other("AppContainer 名稱連續碰撞，已拒絕啟動"))
    }
    /// 刪除 profile 資料夾與登錄資料；已建立的程序 Token 不受影響。
    fn delete_profile(&self) -> io::Result<()> {
        if self
            .profile
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            let hr = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
            if hr != 0 {
                self.profile
                    .store(true, std::sync::atomic::Ordering::Release);
                return Err(io::Error::other(format!(
                    "AppContainer profile 刪除失敗（HRESULT {hr:#010x}）"
                )));
            }
        }
        Ok(())
    }
}
impl Drop for Container {
    fn drop(&mut self) {
        let _ = self.delete_profile();
        if !self.sid.is_null() {
            unsafe {
                FreeSid(self.sid);
            }
        }
    }
}

/// 在既有 DACL 合併一筆允許 ACE（GRANT_ACCESS；不移除其他項目）。
fn add_allowed_ace(
    path: &std::path::Path,
    trustee: Handle,
    permissions: u32,
    inheritance: u32,
) -> io::Result<()> {
    let name = wide(path)?;
    let (mut dacl, mut descriptor) = (std::ptr::null_mut(), std::ptr::null_mut());
    let error = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            1,
            4,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let descriptor = Local(descriptor);
    let entry = ExplicitAccess {
        permissions,
        mode: 1, // GRANT_ACCESS
        inheritance,
        trustee: Trustee {
            multiple: std::ptr::null_mut(),
            operation: 0,
            form: 0, // TRUSTEE_IS_SID
            kind: 5, // TRUSTEE_IS_WELL_KNOWN_GROUP
            name: trustee,
        },
    };
    let mut updated = std::ptr::null_mut();
    let error = unsafe { SetEntriesInAclW(1, &entry, dacl, &mut updated) };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let updated = Local(updated);
    drop(descriptor);
    let error = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            1,
            4,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            updated.0,
            std::ptr::null_mut(),
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(())
}
/// 回收崩潰遺留、建立者程序已不存在的 IMB-C AppContainer profile。
/// 建立者仍存在（或 PID 無法判定）一律保留；只刪除本程式命名格式的項目。
pub(super) fn recover_stale_profiles() -> Vec<String> {
    let mut removed = Vec::new();
    let Ok(path) = wide(
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings",
    ) else {
        return removed;
    };
    let mut key = std::ptr::null_mut();
    if unsafe {
        RegOpenKeyExW(
            0x80000001usize as Handle,
            path.as_ptr(),
            0,
            0x20019,
            &mut key,
        )
    } != 0
    {
        return removed;
    }
    let own = unsafe { GetCurrentProcessId() };
    let mut stale = Vec::new();
    for index in 0..4096 {
        let mut name = [0u16; 256];
        let mut size = name.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                name.as_mut_ptr(),
                &mut size,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            break;
        }
        let mut moniker = [0u16; 256];
        let mut bytes = (moniker.len() * 2) as u32;
        let value = wide("Moniker").unwrap_or_default();
        if unsafe {
            RegGetValueW(
                key,
                name.as_ptr(),
                value.as_ptr(),
                2,
                std::ptr::null_mut(),
                moniker.as_mut_ptr(),
                &mut bytes,
            )
        } != 0
        {
            continue;
        }
        let text = String::from_utf16_lossy(
            &moniker[..moniker
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(moniker.len())],
        );
        let lower = text.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix(&PROFILE_PREFIX.to_ascii_lowercase()) else {
            continue;
        };
        let Some(pid) = rest.split('.').next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        if pid != own && !process_alive(pid) {
            stale.push(text);
        }
    }
    unsafe {
        RegCloseKey(key);
    }
    for name in stale {
        if let Ok(wide_name) = wide(&name)
            && unsafe { DeleteAppContainerProfile(wide_name.as_ptr()) } == 0
        {
            removed.push(name);
        }
    }
    removed
}
fn process_alive(pid: u32) -> bool {
    let handle = unsafe { OpenProcess(0x00100000, 0, pid) }; // SYNCHRONIZE
    if handle.is_null() {
        // 87：PID 不存在。其他錯誤（如拒絕存取）視為仍存在而保留。
        return io::Error::last_os_error().raw_os_error() != Some(87);
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) != 0 }
}

pub(super) struct Sandbox {
    token: OwnedHandle,
    container: Container,
}
impl Sandbox {
    pub(super) fn new() -> io::Result<Self> {
        let mut original = std::ptr::null_mut();
        check(unsafe { OpenProcessToken(GetCurrentProcess(), 0x008b, &mut original) })?; // QUERY | DUPLICATE | ASSIGN_PRIMARY | ADJUST_DEFAULT
        Self::from_token(unsafe { OwnedHandle::from_raw_handle(original) }, false)
    }
    /// 本次呼叫的 AppContainer SID（S-1-15-2-…）。
    #[cfg(test)]
    pub(super) fn container_sid(&self) -> &str {
        &self.container.sid_text
    }
    /// 測試用：重現 T-2 的 Token 狀態而不需提升權限——預設 DACL 只授權「父程序啟用、
    /// 子程序 deny-only」的群組與 SYSTEM，與已提升權限時的 Administrators 相同。
    #[cfg(test)]
    pub(super) fn from_elevated_style_token() -> io::Result<Self> {
        let mut original = std::ptr::null_mut();
        check(unsafe { OpenProcessToken(GetCurrentProcess(), 0x008b, &mut original) })?;
        Self::from_token(unsafe { OwnedHandle::from_raw_handle(original) }, true)
    }
    fn from_token(original: OwnedHandle, simulate_elevated: bool) -> io::Result<Self> {
        let administrators = sid("S-1-5-32-544")?;
        // 測試模擬：提升權限時「父程序啟用、子程序 deny-only」的是 Administrators；一般權限下
        // 以 INTERACTIVE 扮演同一角色，並讓預設 DACL 只授權它與 SYSTEM（不含使用者本人）。
        // 正式建置只傳入第一項（Administrators）。
        let interactive = sid("S-1-5-4")?;
        let disabled = [
            SidAttributes {
                sid: administrators.0,
                attributes: 0,
            },
            SidAttributes {
                sid: interactive.0,
                attributes: 0,
            },
        ];
        let count = if cfg!(test) && simulate_elevated {
            2
        } else {
            1
        };
        let mut token = std::ptr::null_mut();
        check(unsafe {
            CreateRestrictedToken(
                original.as_raw_handle(),
                1,
                count,
                disabled.as_ptr(),
                0,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                &mut token,
            )
        })?;
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        verify_admin_disabled(token.as_raw_handle())?;
        #[cfg(test)]
        if simulate_elevated {
            set_default_dacl(token.as_raw_handle(), "D:(A;;GA;;;IU)(A;;GA;;;SY)")?;
        }
        // 已提升權限的系統管理員 Token 預設擁有者是 Administrators、預設 DACL 只授權
        // Administrators 與 SYSTEM；Administrators 改為 deny-only 後，子程序無法存取自身物件，
        // DLL 初始化失敗（0xC0000142）。改為與一般權限 Token 相同：擁有者與預設 DACL 指向使用者本人。
        // 先建立 AppContainer，預設 DACL 才能授權本次 SID；建立失敗即拒絕啟動。
        let container = Container::create()?;
        set_user_defaults(token.as_raw_handle(), &container.sid_text)?;
        let low = sid("S-1-16-4096")?;
        let mut label = SidAttributes {
            sid: low.0,
            attributes: 0x20,
        };
        check(unsafe {
            SetTokenInformation(
                token.as_raw_handle(),
                25,
                (&mut label as *mut SidAttributes).cast(),
                (std::mem::size_of::<SidAttributes>() + 12) as u32,
            )
        })?;
        Ok(Self { token, container })
    }
    /// 讓本次 AppContainer 唯讀開啟 IMB-C 為本工作產生、需以路徑傳給引擎的單一檔案
    /// （例如目標 ICC 快照）。只授權此 SID，不開放所在目錄或其他工作。
    pub(super) fn grant_file_read(&self, path: &std::path::Path) -> io::Result<()> {
        add_allowed_ace(path, self.container.sid, 0x80000000, 0) // GENERIC_READ，不繼承
    }
    /// 僅對本次新建且仍被原始控制代碼鎖定的空目錄呼叫。
    pub(super) fn grant_directory(&self, dir: &File) -> io::Result<()> {
        // DACL 只多授權本次呼叫的 AppContainer SID；其他工作的 AppContainer 無法存取。
        // AppContainer Token 固定為 Low IL，仍需 Low 標籤才能寫入。
        let sddl = wide(format!(
            "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;{})S:(ML;OICI;NW;;;LW)",
            self.container.sid_text
        ))?;
        let mut sd = std::ptr::null_mut();
        check(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut sd,
                std::ptr::null_mut(),
            )
        })?;
        let sd = Local(sd);
        let (mut present, mut defaulted) = (0, 0);
        let (mut dacl, mut sacl) = (std::ptr::null_mut(), std::ptr::null_mut());
        check(unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted) })?;
        check(unsafe { GetSecurityDescriptorSacl(sd.0, &mut present, &mut sacl, &mut defaulted) })?;
        for (mask, description) in [(0x80000004, "DACL"), (0x10, "LABEL")] {
            let error = unsafe {
                SetSecurityInfo(
                    dir.as_raw_handle(),
                    1,
                    mask,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    dacl,
                    sacl,
                )
            };
            if error != 0 {
                return Err(io::Error::other(format!(
                    "{description}: {}",
                    io::Error::from_raw_os_error(error as i32)
                )));
            }
        }
        Ok(())
    }
    pub(super) fn spawn(&self, command: &Command, input: &File, job: Handle) -> io::Result<Child> {
        if !std::path::Path::new(command.get_program()).is_absolute() {
            return Err(io::Error::other("隔離引擎需要絕對路徑"));
        }
        // 一律從已驗證、已鎖定的 runtime 快取啟動；Portable 目錄不需要也不會被授權。
        let executable = super::runtime_cache::resolve(std::path::Path::new(command.get_program()))
            .map_err(|e| io::Error::other(format!("無法準備引擎 runtime：{e}")))?;
        let executable = executable.as_os_str();
        let app = wide(executable)?;
        // 所有受控引擎採用 Windows CRT 引數規則，不透過 shell。
        let mut line = quote(executable);
        for arg in command.get_args() {
            line.push(32);
            line.extend(quote(arg));
        }
        if line.contains(&0) {
            return Err(io::Error::other("引擎引數含 NUL"));
        }
        line.push(0);
        let mut vars = Vec::new();
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                let key = wide(key)?;
                let value = wide(value)?;
                let mut pair = key[..key.len() - 1].to_vec();
                pair.push(61);
                pair.extend_from_slice(&value[..value.len() - 1]);
                vars.push(pair);
            }
        }
        vars.sort_by_key(|v| String::from_utf16_lossy(v).to_uppercase());
        let mut env = Vec::new();
        for var in vars {
            env.extend(var);
            env.push(0);
        }
        env.push(0);
        if env.len() == 1 {
            env.push(0);
        }
        let cwd = command.get_current_dir().map(wide).transpose()?;
        let runtime = sid(RUNTIME_CAPABILITY)?;
        let mut capabilities = [SidAttributes {
            sid: runtime.0,
            attributes: 4, // SE_GROUP_ENABLED
        }];
        let mut security = SecurityCapabilities {
            app_container: self.container.sid,
            capabilities: capabilities.as_mut_ptr(),
            count: 1,
            reserved: 0,
        };
        // PROCESS_CREATION_CHILD_PROCESS_RESTRICTED：引擎不需要也不得建立子程序。
        let mut child_policy = 1u32;
        let inheritance_lock = SPAWN_LOCK
            .lock()
            .map_err(|_| io::Error::other("子程序啟動鎖異常"))?;
        let input = duplicate(input.as_raw_handle(), 0x120089)?; // FILE_GENERIC_READ，無寫入／刪除權限
        let (out_read, out_write) = pipe()?;
        #[cfg(test)]
        if TEST_HOLD_PIPE.with(|v| v.get()) {
            TEST_HELD_PIPE.with(|v| *v.borrow_mut() = Some(out_write.try_clone().unwrap()));
        }
        let (err_read, err_write) = pipe()?;
        let output = duplicate(out_write.as_raw_handle(), 0x40000000)?;
        let error = duplicate(err_write.as_raw_handle(), 0x40000000)?;
        let mut handles = [
            input.as_raw_handle(),
            output.as_raw_handle(),
            error.as_raw_handle(),
        ];
        let mut jobs = [job];
        let mut attrs = Attributes::new(4)?;
        attrs.set(0x20002, &mut handles)?; // HANDLE_LIST
        attrs.set(0x2000d, &mut jobs)?; // JOB_LIST
        attrs.set(0x20009, &mut security)?; // SECURITY_CAPABILITIES（AppContainer）
        attrs.set(0x2000e, &mut child_policy)?; // CHILD_PROCESS_POLICY
        let mut startup = StartupEx {
            startup: Startup {
                size: std::mem::size_of::<StartupEx>() as u32,
                flags: 0x100,
                input: handles[0],
                output: handles[1],
                error: handles[2],
                ..Default::default()
            },
            attributes: attrs.ptr(),
        };
        let mut info = ProcessInfo::default();
        check(unsafe {
            CreateProcessAsUserW(
                self.token.as_raw_handle(),
                app.as_ptr(),
                line.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                0x0008040c,
                env.as_mut_ptr().cast(),
                cwd.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                &mut startup,
                &mut info,
            )
        })?; // DETACHED_PROCESS | EXTENDED_STARTUPINFO | UNICODE_ENV | SUSPENDED
        // 不配置主控台：主控台程式初始化需建立 conhost.exe 子程序，與禁止子程序政策衝突
        // （實測 0xC0000142）。標準串流已是管線，引擎不需要主控台。
        drop((input, output, error));
        drop(inheritance_lock);
        let process = unsafe { OwnedHandle::from_raw_handle(info.process) };
        let thread = unsafe { OwnedHandle::from_raw_handle(info.thread) };
        let mut child = Child {
            process,
            stdout: Some(out_read),
            stderr: Some(err_read),
        };
        // 恢復前刪除 profile：子程序沒有 AppContainer 專屬可寫資料夾可用。
        let result = self
            .container
            .delete_profile()
            .and_then(|_| app_container_sid(child.process.as_raw_handle()))
            .and_then(|actual| {
                if actual == self.container.sid_text {
                    Ok(())
                } else {
                    Err(io::Error::other("引擎 AppContainer 驗證失敗"))
                }
            })
            .and_then(|_| integrity(child.process.as_raw_handle()))
            .and_then(|level| {
                admin_attributes(child.process.as_raw_handle()).and_then(|attrs| {
                    check_admin_attributes(attrs)?;
                    Ok(level)
                })
            })
            .and_then(|level| {
                #[cfg(test)]
                let reject = TEST_REJECT_BEFORE_RESUME.with(|v| v.get());
                #[cfg(not(test))]
                let reject = false;
                if level == 4096 && !reject {
                    Ok(())
                } else {
                    Err(io::Error::other("引擎 Low IL 驗證失敗"))
                }
            })
            .and_then(|_| {
                if unsafe { ResumeThread(thread.as_raw_handle()) } == 1 {
                    Ok(())
                } else {
                    Err(io::Error::other("隔離主執行緒恢復失敗"))
                }
            });
        if let Err(e) = result {
            let killed = child.kill();
            // 失敗路徑不能在終止失敗時無限等待；外層仍保留暫存並關閉 Job。
            let waited = unsafe { WaitForSingleObject(child.process.as_raw_handle(), 5000) };
            if killed.is_err() || waited != 0 {
                #[derive(Debug)]
                struct LaunchFailure {
                    primary: io::Error,
                    detail: String,
                }
                impl std::fmt::Display for LaunchFailure {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(f, "{}；{}", self.primary, self.detail)
                    }
                }
                impl std::error::Error for LaunchFailure {
                    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                        Some(&self.primary)
                    }
                }
                return Err(io::Error::new(
                    e.kind(),
                    LaunchFailure {
                        primary: e,
                        detail: format!("啟動失敗後終止={killed:?}，等待狀態={waited}"),
                    },
                ));
            }
            return Err(e);
        }
        Ok(child)
    }
}
fn duplicate(handle: Handle, access: u32) -> io::Result<OwnedHandle> {
    let mut copy = std::ptr::null_mut();
    check(unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut copy,
            access,
            1,
            0,
        )
    })?;
    Ok(unsafe { OwnedHandle::from_raw_handle(copy) })
}
fn pipe() -> io::Result<(File, File)> {
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    check(unsafe { CreatePipe(&mut read, &mut write, std::ptr::null_mut(), 65536) })?;
    Ok(unsafe { (File::from_raw_handle(read), File::from_raw_handle(write)) })
}
fn quote(value: &OsStr) -> Vec<u16> {
    let mut out = vec![34];
    let mut slashes = 0;
    for c in value.encode_wide() {
        if c == 92 {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            92,
            if c == 34 { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n(92, slashes * 2));
    out.push(34);
    out
}
struct Attributes(Vec<usize>);
impl Attributes {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size);
        }
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        check(unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), count, 0, &mut size)
        })?;
        Ok(Self(storage))
    }
    fn ptr(&mut self) -> Handle {
        self.0.as_mut_ptr().cast()
    }
    fn set<T>(&mut self, key: usize, value: &mut T) -> io::Result<()> {
        check(unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                key,
                (value as *mut T).cast(),
                std::mem::size_of::<T>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        })
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.ptr());
        }
    }
}
pub(super) fn integrity(process: Handle) -> io::Result<u32> {
    let mut token = std::ptr::null_mut();
    check(unsafe { OpenProcessToken(process, 8, &mut token) })?;
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            25,
            std::ptr::null_mut(),
            0,
            &mut size,
        );
    }
    if size < std::mem::size_of::<SidAttributes>() as u32 {
        return Err(io::Error::other("Token 標籤大小錯誤"));
    }
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            25,
            data.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    })?;
    let label = unsafe { &*data.as_ptr().cast::<SidAttributes>() };
    Ok(unsafe { *GetSidSubAuthority(label.sid, 0) })
}
/// 子程序 Token 的 AppContainer SID；不是 AppContainer 時回傳錯誤。
pub(super) fn app_container_sid(process: Handle) -> io::Result<String> {
    let mut token = std::ptr::null_mut();
    check(unsafe { OpenProcessToken(process, 8, &mut token) })?;
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let (mut is_container, mut size) = (0u32, 0u32);
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            29, // TokenIsAppContainer
            (&mut is_container as *mut u32).cast(),
            4,
            &mut size,
        )
    })?;
    if is_container == 0 {
        return Err(io::Error::other("引擎 Token 不是 AppContainer"));
    }
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            31,
            std::ptr::null_mut(),
            0,
            &mut size,
        );
    }
    if (size as usize) < std::mem::size_of::<Handle>() {
        return Err(io::Error::other("AppContainer SID 大小錯誤"));
    }
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            31, // TokenAppContainerSid
            data.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    })?;
    sid_string(data[0] as Handle)
}
fn check_admin_attributes(attributes: Option<u32>) -> io::Result<()> {
    if attributes.is_some_and(|a| a & 0x6 != 0 || a & 0x10 == 0) {
        return Err(io::Error::other("Administrators SID 未停用，拒絕啟動"));
    }
    Ok(())
}
fn verify_admin_disabled(token: Handle) -> io::Result<()> {
    check_admin_attributes(token_admin_attributes(token)?)
}
fn token_admin_attributes(token: Handle) -> io::Result<Option<u32>> {
    #[repr(C)]
    struct Groups {
        count: u32,
        first: SidAttributes,
    }
    let mut size = 0;
    unsafe {
        GetTokenInformation(token, 2, std::ptr::null_mut(), 0, &mut size);
    }
    let offset = std::mem::offset_of!(Groups, first);
    if (size as usize) < offset {
        return Err(io::Error::other("Token 群組大小錯誤"));
    }
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    check(unsafe { GetTokenInformation(token, 2, data.as_mut_ptr().cast(), size, &mut size) })?;
    let count = unsafe { *data.as_ptr().cast::<u32>() } as usize;
    if count > (size as usize - offset) / std::mem::size_of::<SidAttributes>() {
        return Err(io::Error::other("Token 群組數量錯誤"));
    }
    let admin = sid("S-1-5-32-544")?;
    let groups = unsafe {
        std::slice::from_raw_parts(
            data.as_ptr()
                .cast::<u8>()
                .add(offset)
                .cast::<SidAttributes>(),
            count,
        )
    };
    Ok(groups
        .iter()
        .find(|g| unsafe { EqualSid(g.sid, admin.0) } != 0)
        .map(|g| g.attributes))
}
pub(super) fn admin_attributes(process: Handle) -> io::Result<Option<u32>> {
    let mut token = std::ptr::null_mut();
    check(unsafe { OpenProcessToken(process, 8, &mut token) })?;
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    token_admin_attributes(token.as_raw_handle())
}
pub(super) struct Child {
    process: OwnedHandle,
    pub(super) stdout: Option<File>,
    pub(super) stderr: Option<File>,
}
impl Child {
    #[cfg(test)]
    pub(super) fn id(&self) -> u32 {
        unsafe { GetProcessId(self.process.as_raw_handle()) }
    }
    pub(super) fn kill(&mut self) -> io::Result<()> {
        check(unsafe { TerminateProcess(self.process.as_raw_handle(), 1) })
    }
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        use std::os::windows::process::ExitStatusExt;
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            0 => {
                let mut code = 0;
                check(unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) })?;
                Ok(Some(ExitStatus::from_raw(code)))
            }
            258 => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), u32::MAX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.try_wait()?
            .ok_or_else(|| io::Error::other("子程序仍在執行"))
    }
}

/// 測試用：以指定名稱建立／查詢 profile（模擬崩潰殘留），以及即時推導 capability SID。
#[cfg(test)]
pub(super) mod test_support {
    use super::*;
    pub(in super::super) fn create_profile(name: &str) -> io::Result<()> {
        let name = wide(name)?;
        let mut sid = std::ptr::null_mut();
        let hr = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                std::ptr::null(),
                0,
                &mut sid,
            )
        };
        if hr != 0 {
            return Err(io::Error::other(format!("HRESULT {hr:#010x}")));
        }
        unsafe { FreeSid(sid) };
        Ok(())
    }
    /// profile 是否仍存在：直接查詢 AppContainer 登錄對應（Mappings\<SID>），不建立或刪除任何項目。
    pub(in super::super) fn profile_exists(name: &str) -> bool {
        #[link(name = "userenv")]
        unsafe extern "system" {
            fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut Handle)
            -> i32;
        }
        let mut sid = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                DeriveAppContainerSidFromAppContainerName(wide(name).unwrap().as_ptr(), &mut sid)
            },
            0
        );
        let text = sid_string(sid).unwrap();
        unsafe { FreeSid(sid) };
        let path = wide(format!(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings\{text}"
        ))
        .unwrap();
        let mut key = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                0x80000001usize as Handle,
                path.as_ptr(),
                0,
                0x20019,
                &mut key,
            )
        };
        if status == 0 {
            unsafe { RegCloseKey(key) };
        }
        status == 0
    }
    pub(in super::super) fn delete_profile(name: &str) {
        unsafe { DeleteAppContainerProfile(wide(name).unwrap().as_ptr()) };
    }
    pub(in super::super) fn derived_runtime_capability() -> String {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleW(name: *const u16) -> Handle;
            fn GetProcAddress(module: Handle, name: *const u8) -> Handle;
        }
        type Derive = unsafe extern "system" fn(
            *const u16,
            *mut *mut Handle,
            *mut u32,
            *mut *mut Handle,
            *mut u32,
        ) -> i32;
        let function = unsafe {
            GetProcAddress(
                GetModuleHandleW(wide("kernelbase.dll").unwrap().as_ptr()),
                c"DeriveCapabilitySidsFromName".as_ptr().cast(),
            )
        };
        assert!(!function.is_null());
        let derive: Derive = unsafe { std::mem::transmute(function) };
        let (mut groups, mut group_count, mut caps, mut cap_count) =
            (std::ptr::null_mut(), 0, std::ptr::null_mut(), 0);
        assert_ne!(
            unsafe {
                derive(
                    wide("imbc-runtime").unwrap().as_ptr(),
                    &mut groups,
                    &mut group_count,
                    &mut caps,
                    &mut cap_count,
                )
            },
            0
        );
        assert!(cap_count >= 1);
        let text = sid_string(unsafe { *caps }).unwrap();
        unsafe {
            for i in 0..group_count as usize {
                LocalFree(*groups.add(i));
            }
            for i in 0..cap_count as usize {
                LocalFree(*caps.add(i));
            }
            LocalFree(groups.cast());
            LocalFree(caps.cast());
        }
        text
    }
    pub(in super::super) const RUNTIME: &str = RUNTIME_CAPABILITY;
    pub(in super::super) const PREFIX: &str = PROFILE_PREFIX;
}
