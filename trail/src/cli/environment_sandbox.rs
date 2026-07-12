#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use landlock::{
        Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr, ABI,
    };

    #[derive(Debug)]
    struct SandboxInvocation {
        root: PathBuf,
        reads: Vec<PathBuf>,
        outputs: Vec<PathBuf>,
        caches: Vec<PathBuf>,
        home: PathBuf,
        temporary: PathBuf,
        program: PathBuf,
        args: Vec<OsString>,
    }

    pub(super) fn run(arguments: impl Iterator<Item = OsString>) -> Result<i32, String> {
        let invocation = parse(arguments)?;
        apply_landlock(&invocation)?;
        apply_network_and_privilege_seccomp()?;
        let error = Command::new(&invocation.program)
            .args(&invocation.args)
            .exec();
        Err(format!(
            "failed to execute sandboxed program `{}`: {error}",
            invocation.program.display()
        ))
    }

    fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<SandboxInvocation, String> {
        fn value(
            arguments: &mut impl Iterator<Item = OsString>,
            expected: &str,
        ) -> Result<PathBuf, String> {
            let flag = arguments
                .next()
                .ok_or_else(|| format!("missing internal sandbox flag `{expected}`"))?;
            if flag != OsStr::new(expected) {
                return Err(format!(
                    "expected internal sandbox flag `{expected}`, got `{}`",
                    flag.to_string_lossy()
                ));
            }
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("missing value for internal sandbox flag `{expected}`"))
        }

        let root = value(&mut arguments, "--root")?;
        let mut reads = Vec::new();
        let mut outputs = Vec::new();
        let mut caches = Vec::new();
        let home = loop {
            let flag = arguments
                .next()
                .ok_or_else(|| "missing internal sandbox output or home flag".to_string())?;
            let path = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("missing value for `{}`", flag.to_string_lossy()))?;
            if flag == OsStr::new("--read") {
                reads.push(path);
            } else if flag == OsStr::new("--output") {
                outputs.push(path);
            } else if flag == OsStr::new("--cache") {
                caches.push(path);
            } else if flag == OsStr::new("--home") {
                break path;
            } else {
                return Err(format!(
                    "expected internal sandbox flag `--read`, `--output`, `--cache`, or `--home`, got `{}`",
                    flag.to_string_lossy()
                ));
            }
        };
        let temporary = value(&mut arguments, "--tmp")?;
        let program = value(&mut arguments, "--program")?;
        let separator = arguments
            .next()
            .ok_or_else(|| "missing internal sandbox argument separator".to_string())?;
        if separator != OsStr::new("--") {
            return Err("invalid internal sandbox argument separator".to_string());
        }
        for (name, path) in [
            ("root", &root),
            ("home", &home),
            ("tmp", &temporary),
            ("program", &program),
        ] {
            if !path.is_absolute() || !path.exists() {
                return Err(format!(
                    "internal sandbox {name} path `{}` must be an existing absolute path",
                    path.display()
                ));
            }
        }
        for cache in &caches {
            if !cache.is_absolute() || !cache.is_dir() {
                return Err(format!(
                    "internal sandbox cache path `{}` must be an existing absolute directory",
                    cache.display()
                ));
            }
        }
        let root = fs::canonicalize(&root)
            .map_err(|error| format!("cannot canonicalize sandbox root: {error}"))?;
        let reads = reads
            .into_iter()
            .map(|read| {
                fs::canonicalize(&read).map_err(|error| {
                    format!(
                        "cannot canonicalize sandbox input `{}`: {error}",
                        read.display()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let outputs = outputs
            .into_iter()
            .map(|output| {
                fs::canonicalize(&output).map_err(|error| {
                    format!(
                        "cannot canonicalize sandbox output `{}`: {error}",
                        output.display()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let caches = caches
            .into_iter()
            .map(|cache| {
                fs::canonicalize(&cache).map_err(|error| {
                    format!(
                        "cannot canonicalize sandbox cache `{}`: {error}",
                        cache.display()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let home = fs::canonicalize(&home)
            .map_err(|error| format!("cannot canonicalize sandbox home: {error}"))?;
        let temporary = fs::canonicalize(&temporary)
            .map_err(|error| format!("cannot canonicalize sandbox temporary directory: {error}"))?;
        let program = fs::canonicalize(&program)
            .map_err(|error| format!("cannot canonicalize sandbox program: {error}"))?;
        for (kind, paths) in [("input", &reads), ("output", &outputs)] {
            for path in paths {
                if path.starts_with(&root) {
                    continue;
                }
                return Err(format!(
                    "internal sandbox {kind} `{}` must be an existing absolute path under its root",
                    path.display()
                ));
            }
        }
        // HOME and temporary storage may live beside an ephemeral mounted
        // candidate rather than beneath its read root. They remain explicit
        // allow-list entries and receive no access to their parent directory.
        Ok(SandboxInvocation {
            root,
            reads,
            outputs,
            caches,
            home,
            temporary,
            program,
            args: arguments.collect(),
        })
    }

    fn apply_landlock(invocation: &SandboxInvocation) -> Result<(), String> {
        let abi = ABI::V4;
        let all = AccessFs::from_all(abi);
        let read_without_execute = AccessFs::from_read(abi) & !AccessFs::Execute;
        let read_file = read_without_execute & AccessFs::from_file(abi);
        let read_execute_file = read_file | AccessFs::Execute;
        let writable = all & !AccessFs::Execute;
        let writable_file = writable & AccessFs::from_file(abi);
        let mut ruleset = Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(all)
            .map_err(|error| format!("Landlock cannot handle required filesystem rights: {error}"))?
            .create()
            .map_err(|error| format!("Landlock ruleset creation failed: {error}"))?
            .set_compatibility(CompatLevel::HardRequirement);

        for path in ["/usr", "/bin", "/lib", "/lib64", "/nix/store"] {
            if Path::new(path).exists() {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(
                        PathFd::new(path).map_err(|error| {
                            format!("cannot open sandbox system path `{path}`: {error}")
                        })?,
                        read_without_execute,
                    ))
                    .map_err(|error| {
                        format!("cannot allow sandbox system path `{path}`: {error}")
                    })?;
            }
        }
        for path in ["/etc/ld.so.cache", "/etc/ld.so.preload"] {
            if Path::new(path).is_file() {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(
                        PathFd::new(path).map_err(|error| {
                            format!("cannot open loader file `{path}`: {error}")
                        })?,
                        read_file,
                    ))
                    .map_err(|error| format!("cannot allow loader file `{path}`: {error}"))?;
            }
        }
        // Grant directory enumeration/metadata for path traversal without
        // granting ReadFile across the repository. Exact pinned files and
        // declared outputs are added separately below.
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                PathFd::new(&invocation.root).map_err(|error| {
                    format!(
                        "cannot open sandbox root `{}`: {error}",
                        invocation.root.display()
                    )
                })?,
                AccessFs::ReadDir,
            ))
            .map_err(|error| format!("cannot allow sandbox root reads: {error}"))?;
        for path in &invocation.reads {
            ruleset = ruleset
                .add_rule(PathBeneath::new(
                    PathFd::new(path).map_err(|error| {
                        format!("cannot open sandbox input `{}`: {error}", path.display())
                    })?,
                    if path.is_dir() {
                        read_without_execute
                    } else {
                        read_file
                    },
                ))
                .map_err(|error| {
                    format!("cannot allow sandbox input `{}`: {error}", path.display())
                })?;
        }
        for path in invocation.outputs.iter().chain(&invocation.caches) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(
                    PathFd::new(path).map_err(|error| {
                        format!(
                            "cannot open sandbox writable path `{}`: {error}",
                            path.display()
                        )
                    })?,
                    writable,
                ))
                .map_err(|error| {
                    format!(
                        "cannot allow sandbox writable path `{}`: {error}",
                        path.display()
                    )
                })?;
        }
        for path in [&invocation.home, &invocation.temporary] {
            ruleset = ruleset
                .add_rule(PathBeneath::new(
                    PathFd::new(path).map_err(|error| {
                        format!(
                            "cannot open sandbox writable path `{}`: {error}",
                            path.display()
                        )
                    })?,
                    writable,
                ))
                .map_err(|error| {
                    format!(
                        "cannot allow sandbox writable path `{}`: {error}",
                        path.display()
                    )
                })?;
        }
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                PathFd::new(&invocation.program).map_err(|error| {
                    format!(
                        "cannot open sandbox executable `{}`: {error}",
                        invocation.program.display()
                    )
                })?,
                read_execute_file,
            ))
            .map_err(|error| format!("cannot allow selected sandbox executable: {error}"))?;
        for path in ["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"] {
            if Path::new(path).exists() {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(
                        PathFd::new(path).map_err(|error| {
                            format!("cannot open sandbox device `{path}`: {error}")
                        })?,
                        writable_file,
                    ))
                    .map_err(|error| format!("cannot allow sandbox device `{path}`: {error}"))?;
            }
        }
        ruleset
            .restrict_self()
            .map_err(|error| format!("Landlock restriction was not fully enforced: {error}"))?;
        Ok(())
    }

    fn apply_network_and_privilege_seccomp() -> Result<(), String> {
        // A deny-list is appropriate here because Landlock supplies the
        // filesystem allow-list and exact executable policy. Seccomp closes
        // every socket-family entry point plus kernel/namespace escape hatches.
        let denied = [
            libc::SYS_socket,
            libc::SYS_socketpair,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_sendto,
            libc::SYS_recvfrom,
            libc::SYS_sendmsg,
            libc::SYS_recvmsg,
            libc::SYS_shutdown,
            libc::SYS_setsockopt,
            libc::SYS_getsockopt,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_setns,
            libc::SYS_unshare,
            libc::SYS_kexec_load,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_delete_module,
            libc::SYS_open_by_handle_at,
            libc::SYS_userfaultfd,
            libc::SYS_memfd_create,
            libc::SYS_io_uring_setup,
            libc::SYS_io_uring_enter,
            libc::SYS_io_uring_register,
        ];
        let mut filter = Vec::<libc::sock_filter>::with_capacity(20 + denied.len() * 2);
        filter.push(stmt(BPF_LD | BPF_W | BPF_ABS, 4));
        filter.push(jump(BPF_JMP | BPF_JEQ | BPF_K, audit_arch(), 1, 0));
        filter.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));
        filter.push(stmt(BPF_LD | BPF_W | BPF_ABS, 0));
        filter.push(jump(BPF_JMP | BPF_JGE | BPF_K, 0x4000_0000, 0, 1));
        filter.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));
        for syscall in denied {
            filter.push(jump(BPF_JMP | BPF_JEQ | BPF_K, syscall as u32, 0, 1));
            filter.push(stmt(
                BPF_RET | BPF_K,
                SECCOMP_RET_ERRNO | libc::EPERM as u32,
            ));
        }
        #[cfg(target_arch = "x86_64")]
        for syscall in [libc::SYS_fork, libc::SYS_vfork] {
            filter.push(jump(BPF_JMP | BPF_JEQ | BPF_K, syscall as u32, 0, 1));
            filter.push(stmt(
                BPF_RET | BPF_K,
                SECCOMP_RET_ERRNO | libc::EPERM as u32,
            ));
        }
        // Modern pthread implementations may probe clone3. Report it as
        // unavailable so they can fall back to clone, whose flags we can
        // inspect in classic BPF without dereferencing user memory.
        filter.push(jump(
            BPF_JMP | BPF_JEQ | BPF_K,
            libc::SYS_clone3 as u32,
            0,
            1,
        ));
        filter.push(stmt(
            BPF_RET | BPF_K,
            SECCOMP_RET_ERRNO | libc::ENOSYS as u32,
        ));
        // Permit threads but reject process creation. clone's first argument
        // is the flags word on every Linux architecture Trail supports.
        filter.push(jump(
            BPF_JMP | BPF_JEQ | BPF_K,
            libc::SYS_clone as u32,
            0,
            4,
        ));
        filter.push(stmt(BPF_LD | BPF_W | BPF_ABS, 16));
        filter.push(jump(
            BPF_JMP | BPF_JSET | BPF_K,
            libc::CLONE_THREAD as u32,
            1,
            0,
        ));
        filter.push(stmt(
            BPF_RET | BPF_K,
            SECCOMP_RET_ERRNO | libc::EPERM as u32,
        ));
        filter.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
        filter.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
        let mut program = libc::sock_fprog {
            len: u16::try_from(filter.len())
                .map_err(|_| "seccomp filter is too large".to_string())?,
            filter: filter.as_mut_ptr(),
        };
        // SAFETY: `prctl` receives the documented scalar values and a valid
        // sock_fprog whose backing vector remains alive for the syscall.
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(format!(
                    "cannot enable no_new_privs: {}",
                    io::Error::last_os_error()
                ));
            }
            if libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &mut program as *mut libc::sock_fprog,
            ) != 0
            {
                return Err(format!(
                    "cannot install network-denying seccomp filter: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }

    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10;
    const BPF_JGE: u16 = 0x30;
    const BPF_JSET: u16 = 0x40;
    const BPF_K: u16 = 0x00;
    const BPF_RET: u16 = 0x06;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

    fn stmt(code: u16, value: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k: value,
        }
    }

    fn jump(code: u16, value: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt,
            jf,
            k: value,
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn audit_arch() -> u32 {
        0xc000_003e
    }

    #[cfg(target_arch = "aarch64")]
    fn audit_arch() -> u32 {
        0xc000_00b7
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("Trail's Linux recipe sandbox supports x86_64 and aarch64 only");
}

#[cfg(target_os = "windows")]
mod windows {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr::{null, null_mut};
    use std::time::{SystemTime, UNIX_EPOCH};

    use winapi::shared::basetsd::{DWORD_PTR, SIZE_T};
    use winapi::shared::minwindef::{DWORD, FALSE};
    use winapi::shared::sddl::ConvertSidToStringSidW;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::jobapi2::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    };
    use winapi::um::processthreadsapi::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
        InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
        UpdateProcThreadAttribute, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    };
    use winapi::um::securitybaseapi::FreeSid;
    use winapi::um::synchapi::WaitForSingleObject;
    use winapi::um::userenv::{CreateAppContainerProfile, DeleteAppContainerProfile};
    use winapi::um::winbase::{
        LocalFree, CREATE_SUSPENDED, EXTENDED_STARTUPINFO_PRESENT, INFINITE, STARTUPINFOEXW,
        WAIT_FAILED, WAIT_OBJECT_0,
    };
    use winapi::um::winnt::{
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY, PSID, SECURITY_CAPABILITIES,
    };

    const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES: DWORD_PTR = 0x0002_0009;

    #[derive(Debug)]
    struct SandboxInvocation {
        root: PathBuf,
        reads: Vec<PathBuf>,
        outputs: Vec<PathBuf>,
        caches: Vec<PathBuf>,
        home: PathBuf,
        temporary: PathBuf,
        program: PathBuf,
        args: Vec<OsString>,
    }

    struct AppContainerProfile {
        name: Vec<u16>,
        sid: PSID,
    }

    impl Drop for AppContainerProfile {
        fn drop(&mut self) {
            // SAFETY: both values were returned by the AppContainer APIs and
            // remain owned by this guard until process completion.
            unsafe {
                let _ = DeleteAppContainerProfile(self.name.as_ptr());
                if !self.sid.is_null() {
                    let _ = FreeSid(self.sid);
                }
            }
        }
    }

    struct OwnedHandle(winapi::shared::ntdef::HANDLE);

    impl OwnedHandle {
        fn new(handle: winapi::shared::ntdef::HANDLE, what: &str) -> Result<Self, String> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                Err(last_error(what))
            } else {
                Ok(Self(handle))
            }
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: this guard uniquely owns a valid Win32 handle.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// A process created suspended must never escape if one of the Job Object
    /// setup steps fails. Closing a process handle does not terminate the
    /// process, so the ordinary handle guard is insufficient for this case.
    struct OwnedProcessHandle {
        handle: winapi::shared::ntdef::HANDLE,
        terminate_on_drop: bool,
    }

    impl OwnedProcessHandle {
        fn new(handle: winapi::shared::ntdef::HANDLE) -> Result<Self, String> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                Err(last_error("invalid Windows sandbox process handle"))
            } else {
                Ok(Self {
                    handle,
                    terminate_on_drop: true,
                })
            }
        }

        fn process_exited(&mut self) {
            self.terminate_on_drop = false;
        }
    }

    impl Drop for OwnedProcessHandle {
        fn drop(&mut self) {
            // SAFETY: this guard uniquely owns a valid process handle. A failed
            // setup must terminate the still-suspended child before closing it.
            unsafe {
                if self.terminate_on_drop {
                    let _ = TerminateProcess(self.handle, 126);
                    // Termination is asynchronous. Give the process a bounded
                    // opportunity to leave the AppContainer before its profile
                    // is deleted by the outer guard.
                    let _ = WaitForSingleObject(self.handle, 5_000);
                }
                let _ = CloseHandle(self.handle);
            }
        }
    }

    struct AttributeList {
        storage: Vec<usize>,
        list: LPPROC_THREAD_ATTRIBUTE_LIST,
    }

    impl AttributeList {
        fn one() -> Result<Self, String> {
            let mut bytes: SIZE_T = 0;
            // SAFETY: the documented sizing call accepts a null list.
            unsafe {
                let _ = InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
            }
            if bytes == 0 {
                return Err(last_error("cannot size Windows process attribute list"));
            }
            let words = bytes.div_ceil(size_of::<usize>());
            let mut storage = vec![0usize; words];
            let list = storage.as_mut_ptr().cast();
            // SAFETY: `storage` is aligned, writable, and at least the size
            // returned by the sizing call.
            if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) } == 0 {
                return Err(last_error(
                    "cannot initialize Windows process attribute list",
                ));
            }
            Ok(Self { storage, list })
        }
    }

    impl Drop for AttributeList {
        fn drop(&mut self) {
            // SAFETY: initialization succeeded and storage outlives this call.
            unsafe { DeleteProcThreadAttributeList(self.list) };
            let _ = self.storage.len();
        }
    }

    pub(super) fn run(arguments: impl Iterator<Item = OsString>) -> Result<i32, String> {
        let invocation = parse(arguments)?;
        let profile = create_profile()?;
        let sid = sid_string(profile.sid)?;
        grant_appcontainer_access(&invocation.root, &sid, "RX", false)?;
        for read in &invocation.reads {
            grant_appcontainer_access(read, &sid, "R", false)?;
            let mut ancestor = read.parent();
            while let Some(path) = ancestor {
                if !path.starts_with(&invocation.root) {
                    break;
                }
                grant_appcontainer_access(path, &sid, "RX", false)?;
                if path == invocation.root {
                    break;
                }
                ancestor = path.parent();
            }
        }
        for output in invocation.outputs.iter().chain(&invocation.caches) {
            grant_appcontainer_access(output, &sid, "M", true)?;
        }
        grant_appcontainer_access(&invocation.home, &sid, "M", true)?;
        grant_appcontainer_access(&invocation.temporary, &sid, "M", true)?;
        launch_in_appcontainer(&invocation, &profile)
    }

    fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<SandboxInvocation, String> {
        fn value(
            arguments: &mut impl Iterator<Item = OsString>,
            expected: &str,
        ) -> Result<PathBuf, String> {
            let flag = arguments
                .next()
                .ok_or_else(|| format!("missing internal sandbox flag `{expected}`"))?;
            if flag != OsStr::new(expected) {
                return Err(format!(
                    "expected internal sandbox flag `{expected}`, got `{}`",
                    flag.to_string_lossy()
                ));
            }
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("missing value for internal sandbox flag `{expected}`"))
        }

        let root = value(&mut arguments, "--root")?;
        let mut reads = Vec::new();
        let mut outputs = Vec::new();
        let mut caches = Vec::new();
        let home = loop {
            let flag = arguments
                .next()
                .ok_or_else(|| "missing internal sandbox output or home flag".to_string())?;
            let path = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("missing value for `{}`", flag.to_string_lossy()))?;
            if flag == OsStr::new("--read") {
                reads.push(path);
            } else if flag == OsStr::new("--output") {
                outputs.push(path);
            } else if flag == OsStr::new("--cache") {
                caches.push(path);
            } else if flag == OsStr::new("--home") {
                break path;
            } else {
                return Err(format!(
                    "expected internal sandbox flag `--read`, `--output`, `--cache`, or `--home`, got `{}`",
                    flag.to_string_lossy()
                ));
            }
        };
        let temporary = value(&mut arguments, "--tmp")?;
        let program = value(&mut arguments, "--program")?;
        let separator = arguments
            .next()
            .ok_or_else(|| "missing internal sandbox argument separator".to_string())?;
        if separator != OsStr::new("--") {
            return Err("invalid internal sandbox argument separator".to_string());
        }
        for (name, path) in [
            ("root", &root),
            ("home", &home),
            ("tmp", &temporary),
            ("program", &program),
        ] {
            if !path.is_absolute() || !path.exists() {
                return Err(format!(
                    "internal sandbox {name} path `{}` must be an existing absolute path",
                    path.display()
                ));
            }
        }
        for cache in &caches {
            if !cache.is_absolute() || !cache.is_dir() {
                return Err(format!(
                    "internal sandbox cache path `{}` must be an existing absolute directory",
                    cache.display()
                ));
            }
        }
        let root = fs::canonicalize(&root)
            .map_err(|error| format!("cannot canonicalize sandbox root: {error}"))?;
        let reads = reads
            .into_iter()
            .map(|read| {
                fs::canonicalize(&read).map_err(|error| {
                    format!(
                        "cannot canonicalize sandbox input `{}`: {error}",
                        read.display()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let outputs = outputs
            .into_iter()
            .map(|output| {
                fs::canonicalize(&output).map_err(|error| {
                    format!(
                        "cannot canonicalize sandbox output `{}`: {error}",
                        output.display()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let caches = caches
            .into_iter()
            .map(|cache| {
                fs::canonicalize(&cache).map_err(|error| {
                    format!(
                        "cannot canonicalize sandbox cache `{}`: {error}",
                        cache.display()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let home = fs::canonicalize(&home)
            .map_err(|error| format!("cannot canonicalize sandbox home: {error}"))?;
        let temporary = fs::canonicalize(&temporary)
            .map_err(|error| format!("cannot canonicalize sandbox temporary directory: {error}"))?;
        let program = fs::canonicalize(&program)
            .map_err(|error| format!("cannot canonicalize sandbox program: {error}"))?;
        for (kind, paths) in [("input", &reads), ("output", &outputs)] {
            for path in paths {
                if path.starts_with(&root) {
                    continue;
                }
                return Err(format!(
                    "internal sandbox {kind} `{}` must be an existing absolute path under its root",
                    path.display()
                ));
            }
        }
        // HOME and temporary storage may live beside an ephemeral mounted
        // candidate rather than beneath its read root. The AppContainer ACLs
        // grant only these directories, never their parent.
        Ok(SandboxInvocation {
            root,
            reads,
            outputs,
            caches,
            home,
            temporary,
            program,
            args: arguments.collect(),
        })
    }

    fn create_profile() -> Result<AppContainerProfile, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
            .as_nanos();
        let name = format!("Trail.Recipe.{}.{nonce:x}", std::process::id());
        let name = wide_null(OsStr::new(&name));
        let display = wide_null(OsStr::new("Trail restricted recipe"));
        let description = wide_null(OsStr::new("Ephemeral Trail command recipe sandbox"));
        let mut sid: PSID = null_mut();
        // SAFETY: all strings are NUL-terminated and `sid` is an out pointer.
        let result = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                display.as_ptr(),
                description.as_ptr(),
                null_mut(),
                0,
                &mut sid,
            )
        };
        if result < 0 || sid.is_null() {
            return Err(format!(
                "cannot create capability-free Windows AppContainer (HRESULT 0x{:08x})",
                result as u32
            ));
        }
        Ok(AppContainerProfile { name, sid })
    }

    fn sid_string(sid: PSID) -> Result<String, String> {
        let mut string_sid = null_mut();
        // SAFETY: `sid` is valid for the profile lifetime and the API allocates
        // a NUL-terminated string released with LocalFree.
        if unsafe { ConvertSidToStringSidW(sid, &mut string_sid) } == 0 {
            return Err(last_error("cannot format Windows AppContainer SID"));
        }
        let mut length = 0usize;
        // SAFETY: the conversion API returned a valid NUL-terminated string.
        unsafe {
            while *string_sid.add(length) != 0 {
                length += 1;
            }
        }
        // SAFETY: `length` was measured within the allocated string.
        let value = String::from_utf16(unsafe { std::slice::from_raw_parts(string_sid, length) })
            .map_err(|error| format!("AppContainer SID is not valid UTF-16: {error}"));
        // SAFETY: ownership of the allocation belongs to this function.
        unsafe {
            let _ = LocalFree(string_sid.cast());
        }
        value
    }

    fn grant_appcontainer_access(
        path: &Path,
        sid: &str,
        rights: &str,
        recursive: bool,
    ) -> Result<(), String> {
        let system_root = std::env::var_os("SystemRoot")
            .ok_or_else(|| "SystemRoot is unavailable to the Windows sandbox helper".to_string())?;
        let icacls = PathBuf::from(system_root).join("System32/icacls.exe");
        if !icacls.is_file() {
            return Err(format!(
                "Windows ACL utility `{}` is unavailable",
                icacls.display()
            ));
        }
        let principal = if recursive {
            format!("*{sid}:(OI)(CI){rights}")
        } else {
            format!("*{sid}:{rights}")
        };
        let mut command = Command::new(&icacls);
        command.arg(path).args(["/grant", &principal]);
        if recursive {
            command.arg("/T");
        }
        let status = command
            .arg("/Q")
            .status()
            .map_err(|error| format!("cannot launch `{}`: {error}", icacls.display()))?;
        if !status.success() {
            return Err(format!(
                "`{}` could not grant AppContainer {rights} access to `{}` ({status})",
                icacls.display(),
                path.display()
            ));
        }
        Ok(())
    }

    fn launch_in_appcontainer(
        invocation: &SandboxInvocation,
        profile: &AppContainerProfile,
    ) -> Result<i32, String> {
        let attributes = AttributeList::one()?;
        let mut capabilities = SECURITY_CAPABILITIES {
            AppContainerSid: profile.sid,
            Capabilities: null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        // SAFETY: the attribute list is initialized and both it and the
        // capabilities structure remain alive through CreateProcessW.
        if unsafe {
            UpdateProcThreadAttribute(
                attributes.list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                (&mut capabilities as *mut SECURITY_CAPABILITIES).cast(),
                size_of::<SECURITY_CAPABILITIES>(),
                null_mut(),
                null_mut(),
            )
        } == 0
        {
            return Err(last_error(
                "cannot attach AppContainer capabilities to Windows process",
            ));
        }

        let program = wide_null(invocation.program.as_os_str());
        let mut command_line = windows_command_line(&invocation.program, &invocation.args);
        let current_directory = std::env::current_dir()
            .and_then(std::fs::canonicalize)
            .map_err(|error| format!("cannot canonicalize sandbox working directory: {error}"))?;
        if !current_directory.starts_with(&invocation.root) {
            return Err(format!(
                "sandbox working directory `{}` escapes root `{}`",
                current_directory.display(),
                invocation.root.display()
            ));
        }
        let current_directory = wide_null(current_directory.as_os_str());
        // SAFETY: zero is the documented initialization for these structures;
        // required fields are assigned below.
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as DWORD;
        startup.lpAttributeList = attributes.list;
        let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
        // SAFETY: all pointers reference live, writable, NUL-terminated buffers
        // for the duration of CreateProcessW.
        if unsafe {
            CreateProcessW(
                program.as_ptr(),
                command_line.as_mut_ptr(),
                null_mut(),
                null_mut(),
                FALSE,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED,
                null_mut(),
                current_directory.as_ptr(),
                (&mut startup as *mut STARTUPINFOEXW).cast(),
                &mut process,
            )
        } == 0
        {
            return Err(last_error(&format!(
                "cannot launch `{}` in Windows AppContainer",
                invocation.program.display()
            )));
        }
        let mut process_handle = OwnedProcessHandle::new(process.hProcess)?;
        let thread_handle = OwnedHandle::new(process.hThread, "invalid thread handle")?;
        let job = OwnedHandle::new(
            // SAFETY: null security/name pointers request a private job object.
            unsafe { CreateJobObjectW(null_mut(), null()) },
            "cannot create Windows sandbox Job Object",
        )?;
        // SAFETY: zero initialization followed by the documented limit fields.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        if invocation.outputs.is_empty() {
            limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            limits.ProcessMemoryLimit = 512 * 1024 * 1024;
        }
        // SAFETY: the job and structure are valid for the call.
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
            )
        } == 0
        {
            return Err(last_error("cannot constrain Windows sandbox Job Object"));
        }
        // SAFETY: both handles are valid and the process is still suspended.
        if unsafe { AssignProcessToJobObject(job.0, process_handle.handle) } == 0 {
            return Err(last_error(
                "cannot assign restricted process to Windows sandbox Job Object",
            ));
        }
        // SAFETY: the primary thread is suspended exactly once by CreateProcessW.
        if unsafe { ResumeThread(thread_handle.0) } == u32::MAX {
            return Err(last_error("cannot resume Windows sandbox process"));
        }
        // SAFETY: process_handle remains live and an infinite wait is intended.
        let wait_result = unsafe { WaitForSingleObject(process_handle.handle, INFINITE) };
        if wait_result == WAIT_FAILED {
            return Err(last_error("cannot wait for Windows sandbox process"));
        }
        if wait_result != WAIT_OBJECT_0 {
            return Err(format!(
                "Windows sandbox process wait returned unexpected result 0x{wait_result:08x}"
            ));
        }
        process_handle.process_exited();
        let mut exit_code = 0u32;
        // SAFETY: the process has terminated and the output pointer is valid.
        if unsafe { GetExitCodeProcess(process_handle.handle, &mut exit_code) } == 0 {
            return Err(last_error("cannot read Windows sandbox process exit code"));
        }
        drop(attributes);
        Ok(exit_code as i32)
    }

    fn windows_command_line(program: &Path, arguments: &[OsString]) -> Vec<u16> {
        let mut command = quote_windows_argument(program.as_os_str());
        for argument in arguments {
            command.push(u16::from(b' '));
            command.extend(quote_windows_argument(argument));
        }
        command.push(0);
        command
    }

    fn quote_windows_argument(argument: &OsStr) -> Vec<u16> {
        let value = argument.encode_wide().collect::<Vec<_>>();
        if !value.is_empty()
            && !value
                .iter()
                .any(|unit| [u16::from(b' '), u16::from(b'\t'), u16::from(b'"')].contains(unit))
        {
            return value;
        }
        let mut quoted = vec![u16::from(b'"')];
        let mut backslashes = 0usize;
        for unit in value {
            if unit == u16::from(b'\\') {
                backslashes += 1;
            } else if unit == u16::from(b'"') {
                quoted.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes * 2 + 1));
                quoted.push(u16::from(b'"'));
                backslashes = 0;
            } else {
                quoted.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes));
                backslashes = 0;
                quoted.push(unit);
            }
        }
        quoted.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes * 2));
        quoted.push(u16::from(b'"'));
        quoted
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn last_error(context: &str) -> String {
        // SAFETY: GetLastError has no preconditions.
        let code = unsafe { GetLastError() };
        format!("{context}: {}", io::Error::from_raw_os_error(code as i32))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::windows::ffi::OsStringExt;

        fn quoted(value: &str) -> String {
            String::from_utf16(&quote_windows_argument(OsStr::new(value))).unwrap()
        }

        #[test]
        fn windows_arguments_follow_command_line_to_argv_w_quoting() {
            assert_eq!(quoted("plain"), "plain");
            assert_eq!(quoted(""), "\"\"");
            assert_eq!(quoted("two words"), "\"two words\"");
            assert_eq!(quoted("say \"hello\""), "\"say \\\"hello\\\"\"");
            assert_eq!(
                quoted("C:\\path with space\\"),
                "\"C:\\path with space\\\\\""
            );

            let unpaired_surrogate = OsString::from_wide(&[0xd800]);
            assert_eq!(
                quote_windows_argument(&unpaired_surrogate),
                vec![0xd800],
                "native Windows paths must never pass through lossy UTF-8 conversion"
            );
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn run(arguments: impl Iterator<Item = std::ffi::OsString>) -> Result<i32, String> {
    linux::run(arguments)
}

#[cfg(target_os = "windows")]
pub(crate) fn run(arguments: impl Iterator<Item = std::ffi::OsString>) -> Result<i32, String> {
    windows::run(arguments)
}
