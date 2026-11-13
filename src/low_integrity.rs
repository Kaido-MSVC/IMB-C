//! 僅用於受控引擎：唯讀輸入、管線輸出、移除特權、Low IL、原子 Job 納管。
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
#[link(name = "advapi32")]
unsafe extern "system" {
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
fn set_user_defaults(token: Handle) -> io::Result<()> {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
    }
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
    let mut text = std::ptr::null_mut();
    check(unsafe { ConvertSidToStringSidW(user, &mut text) })?;
    let text_owner = Local(text.cast());
    let user_text = unsafe {
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    drop(text_owner);
    set_default_dacl(token, &format!("D:(A;;GA;;;{user_text})(A;;GA;;;SY)"))?;
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
pub(super) struct Sandbox {
    token: OwnedHandle,
}
impl Sandbox {
    pub(super) fn new() -> io::Result<Self> {
        let mut original = std::ptr::null_mut();
        check(unsafe { OpenProcessToken(GetCurrentProcess(), 0x008b, &mut original) })?; // QUERY | DUPLICATE | ASSIGN_PRIMARY | ADJUST_DEFAULT
        Self::from_token(unsafe { OwnedHandle::from_raw_handle(original) }, false)
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
        set_user_defaults(token.as_raw_handle())?;
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
        Ok(Self { token })
    }
    /// 僅對本次新建且仍被原始控制代碼鎖定的空目錄呼叫。
    pub(super) fn grant_directory(&self, dir: &File) -> io::Result<()> {
        // 不增加 DACL 成員；只降低本次目錄的完整性標籤。
        let sddl = wide("D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)S:(ML;OICI;NW;;;LW)")?;
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
        let executable = command.get_program();
        if !std::path::Path::new(executable).is_absolute() {
            return Err(io::Error::other("隔離引擎需要絕對路徑"));
        }
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
        let mut attrs = Attributes::new(2)?;
        attrs.set(0x20002, &mut handles)?; // HANDLE_LIST
        attrs.set(0x2000d, &mut jobs)?; // JOB_LIST
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
                0x08080404,
                env.as_mut_ptr().cast(),
                cwd.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                &mut startup,
                &mut info,
            )
        })?; // NO_WINDOW | EXTENDED_STARTUPINFO | UNICODE_ENV | SUSPENDED
        drop((input, output, error));
        drop(inheritance_lock);
        let process = unsafe { OwnedHandle::from_raw_handle(info.process) };
        let thread = unsafe { OwnedHandle::from_raw_handle(info.thread) };
        let mut child = Child {
            process,
            stdout: Some(out_read),
            stderr: Some(err_read),
        };
        let result = integrity(child.process.as_raw_handle())
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
