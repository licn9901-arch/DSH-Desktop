//! DSH Host 子进程创建、状态探测与清理。

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::logger::{log_app, log_error};
use crate::runtime::RuntimePaths;

const HINDSIGHT_CREDENTIAL_REF: &str = "DSH_DESKTOP_HINDSIGHT_API_TOKEN";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 持有唯一 Host 子进程，供启动线程、退出回调和监视线程共享。
pub struct HostSupervisor {
    child: Mutex<Option<Box<dyn ManagedChild>>>,
}

impl Default for HostSupervisor {
    /// 创建空 supervisor，Host 由启动协调流程显式挂载。
    fn default() -> Self {
        Self::new()
    }
}

/// 抽象可管理子进程，使 supervisor 单元测试不依赖真实 Node 或 DSH。
pub trait ManagedChild: Send {
    /// 返回操作系统进程 ID。
    fn id(&self) -> u32;
    /// 非阻塞读取退出码，`None` 表示进程仍在运行。
    fn try_exit_code(&mut self) -> io::Result<Option<Option<i32>>>;
    /// 等待进程结束并返回可用退出码。
    fn wait_for_exit(&mut self) -> io::Result<Option<i32>>;
    /// 进程树终止命令不可用时，使用已持有句柄强制结束根进程。
    fn force_exit(&mut self) -> io::Result<()>;
}

impl ManagedChild for Child {
    fn id(&self) -> u32 {
        Child::id(self)
    }

    fn try_exit_code(&mut self) -> io::Result<Option<Option<i32>>> {
        self.try_wait()
            .map(|status| status.map(|value| value.code()))
    }

    fn wait_for_exit(&mut self) -> io::Result<Option<i32>> {
        self.wait().map(|status| status.code())
    }

    fn force_exit(&mut self) -> io::Result<()> {
        self.kill()
    }
}

/// 抽象 Windows 进程树终止动作，测试可记录请求而不操作真实进程。
trait ProcessTreeTerminator {
    /// 请求进程树正常结束。
    fn request(&self, pid: u32) -> Result<(), String>;
    /// 强制结束仍未退出的完整进程树。
    fn force(&self, pid: u32) -> Result<(), String>;
}

#[cfg(windows)]
struct WindowsProcessTreeTerminator;

#[cfg(windows)]
impl ProcessTreeTerminator for WindowsProcessTreeTerminator {
    fn request(&self, pid: u32) -> Result<(), String> {
        run_taskkill(pid, false)
    }

    fn force(&self, pid: u32) -> Result<(), String> {
        run_taskkill(pid, true)
    }
}

#[cfg(unix)]
struct UnixProcessTreeTerminator;

#[cfg(unix)]
impl ProcessTreeTerminator for UnixProcessTreeTerminator {
    fn request(&self, pid: u32) -> Result<(), String> {
        signal_process_group(pid, libc::SIGTERM)
    }

    fn force(&self, pid: u32) -> Result<(), String> {
        signal_process_group(pid, libc::SIGKILL)
    }
}

impl HostSupervisor {
    /// 创建尚未持有进程的 supervisor，允许插件失败后复用同一应用状态重启核心 Host。
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }

    /// 启动 DSH Web Host，并返回可分别消费的 stdout 与 stderr。
    pub fn spawn(paths: &RuntimePaths) -> Result<(Self, ChildStdout, ChildStderr), String> {
        let supervisor = Self::new();
        let (stdout, stderr) = supervisor.start(paths)?;
        Ok((supervisor, stdout, stderr))
    }

    /// 在当前 supervisor 中启动唯一 Host；已持有进程时拒绝重复启动。
    pub fn start(&self, paths: &RuntimePaths) -> Result<(ChildStdout, ChildStderr), String> {
        let mut guard = self.child.lock().unwrap_or_else(|error| error.into_inner());
        if guard.is_some() {
            return Err("host is already running".to_owned());
        }
        log_app(&format!(
            "spawning: {} --expose-internals {} web --patch {} --host 127.0.0.1 --port 0 (cwd: {})",
            paths.node.display(),
            paths.cli_entry.display(),
            paths.desktop_policy_patch.display(),
            paths.working_directory.display()
        ));

        let inherited_path = env::var_os("PATH");
        let host_path = build_host_path(paths, inherited_path.as_deref())?;
        let mut command = Command::new(&paths.node);
        command
            .arg("--expose-internals")
            .arg(&paths.cli_entry)
            .arg("web")
            .arg("--patch")
            .arg(&paths.desktop_policy_patch)
            .args(["--host", "127.0.0.1", "--port", "0"])
            .current_dir(&paths.working_directory)
            .env("DSH_HOME", &paths.dsh_home)
            .env("DSH_DESKTOP_NODE_EXECUTABLE", &paths.node)
            .env("DSH_DESKTOP_CLI_ENTRY", &paths.cli_entry)
            .env("DSH_DESKTOP_HOST_ROOT", &paths.host_root)
            .env("DSH_DESKTOP_WEB_PROFILE", &paths.web_profile)
            .env("DSH_DESKTOP_USER_HOME", &paths.user_home)
            .env("PATH", host_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 环境变量由部署平台显式提供时优先；否则桥接桌面凭据文件中的专用引用。
        if env::var_os("HINDSIGHT_API_TOKEN").is_none() {
            if let Some(token) = hindsight_token_from_credentials(&paths.dsh_home)? {
                command.env("HINDSIGHT_API_TOKEN", token);
            }
        }
        hide_console_window(&mut command);
        configure_process_group(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start Node ({}): {error}", paths.node.display()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "host stdout is not available".to_owned())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "host stderr is not available".to_owned())?;
        log_app(&format!("host started: pid={}", child.id()));
        *guard = Some(Box::new(child));
        Ok((stdout, stderr))
    }

    /// 非阻塞读取 Host 退出码；外层 `None` 表示仍在运行，内层 `None` 表示无可用退出码。
    pub fn try_exit_code(&self) -> Option<Option<i32>> {
        let mut guard = self.child.lock().unwrap_or_else(|error| error.into_inner());
        match guard.as_mut() {
            Some(child) => match child.try_exit_code() {
                Ok(Some(exit_code)) => Some(exit_code),
                Ok(None) => None,
                Err(error) => {
                    log_error(&format!("failed to read host exit status: {error}"));
                    Some(None)
                }
            },
            None => Some(None),
        }
    }

    /// 返回当前 Host PID；进程已被清理时返回 `None`。
    pub fn pid(&self) -> Option<u32> {
        self.child
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|child| child.id())
    }

    /// 终止 Host 进程树并回收子进程；重复调用不会产生额外副作用。
    pub fn shutdown(&self) {
        #[cfg(windows)]
        let terminator = WindowsProcessTreeTerminator;
        #[cfg(unix)]
        let terminator = UnixProcessTreeTerminator;
        self.shutdown_with(
            &terminator,
            Duration::from_secs(5),
            Duration::from_millis(100),
        );
    }

    /// 启动恢复时立即强制结束当前记录的进程树，避免等待已失效 Host 的优雅退出窗口。
    pub fn shutdown_for_recovery(&self) {
        #[cfg(windows)]
        let terminator = WindowsProcessTreeTerminator;
        #[cfg(unix)]
        let terminator = UnixProcessTreeTerminator;
        self.shutdown_for_recovery_with(&terminator);
    }

    /// 注入进程树终止器执行恢复清理，供单元测试确认只处理记录 PID。
    fn shutdown_for_recovery_with(&self, terminator: &dyn ProcessTreeTerminator) {
        self.shutdown_with(terminator, Duration::ZERO, Duration::ZERO);
    }

    /// 使用指定终止器和时钟参数执行退出，供无固定等待的单元测试注入 fake。
    fn shutdown_with(
        &self,
        terminator: &dyn ProcessTreeTerminator,
        grace_period: Duration,
        poll_interval: Duration,
    ) {
        let child = self
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let Some(mut child) = child else {
            return;
        };
        let pid = child.id();
        match child.try_exit_code() {
            Ok(Some(exit_code)) => {
                log_app(&format!(
                    "host already exited before shutdown: pid={pid}, code={exit_code:?}"
                ));
                return;
            }
            Ok(None) => {}
            Err(error) => {
                log_error(&format!(
                    "failed to inspect host before shutdown pid={pid}: {error}"
                ));
            }
        }
        log_app(&format!("requesting host shutdown: pid={pid}"));
        if let Err(error) = terminator.request(pid) {
            log_error(&format!(
                "host graceful shutdown request failed: pid={pid}, {error}"
            ));
        }

        let deadline = Instant::now() + grace_period;
        while Instant::now() < deadline {
            match child.try_exit_code() {
                Ok(Some(exit_code)) => {
                    log_app(&format!("host exited: pid={pid}, code={exit_code:?}"));
                    return;
                }
                Ok(None) => std::thread::sleep(poll_interval),
                Err(error) => {
                    log_error(&format!("failed while waiting for host pid={pid}: {error}"));
                    break;
                }
            }
        }

        log_error(&format!("forcing host process tree shutdown: pid={pid}"));
        if let Err(error) = terminator.force(pid) {
            log_error(&format!(
                "host process tree shutdown failed; forcing root process handle: pid={pid}, {error}"
            ));
            if let Err(kill_error) = child.force_exit() {
                log_error(&format!(
                    "failed to force host root process handle: pid={pid}, {kill_error}"
                ));
                return;
            }
        }
        match child.wait_for_exit() {
            Ok(exit_code) => log_app(&format!(
                "host process tree reaped: pid={pid}, code={exit_code:?}"
            )),
            Err(error) => log_error(&format!("failed to reap host pid={pid}: {error}")),
        }
    }
}

/// 从 DSH 凭据文档读取 Hindsight 专用引用；返回值仅注入子进程，禁止进入日志。
fn hindsight_token_from_credentials(dsh_home: &std::path::Path) -> Result<Option<String>, String> {
    let path = dsh_home.join(".credentials.yaml");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read DSH credentials {}: {error}",
                path.display()
            ))
        }
    };
    let document: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("invalid DSH credentials {}: {error}", path.display()))?;
    let mapping = document
        .as_mapping()
        .ok_or_else(|| format!("DSH credentials {} must be a YAML mapping", path.display()))?;
    let key = serde_yaml::Value::String(HINDSIGHT_CREDENTIAL_REF.to_owned());
    match mapping.get(&key) {
        None => Ok(None),
        Some(serde_yaml::Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(format!(
            "DSH credential {HINDSIGHT_CREDENTIAL_REF} in {} must be a non-empty string",
            path.display()
        )),
    }
}

/// 生成 Host 私有 PATH，确保内置 Node 与 pnpm 在用户和系统工具之前解析。
fn build_host_path(paths: &RuntimePaths, inherited: Option<&OsStr>) -> Result<OsString, String> {
    let node_directory = paths.node.parent().ok_or_else(|| {
        format!(
            "bundled Node has no parent directory: {}",
            paths.node.display()
        )
    })?;
    let mut directories = vec![
        node_directory.to_path_buf(),
        paths.tool_bin_directory.clone(),
    ];
    if let Some(value) = inherited {
        directories.extend(env::split_paths(value));
    }
    env::join_paths(directories)
        .map_err(|error| format!("failed to construct private Host PATH: {error}"))
}

/// 调用 Windows `taskkill` 处理指定 PID 的完整进程树，并保留失败诊断。
#[cfg(windows)]
fn run_taskkill(pid: u32, force: bool) -> Result<(), String> {
    let pid_text = pid.to_string();
    let mut killer = Command::new("taskkill");
    killer.args(["/PID", &pid_text, "/T"]);
    if force {
        killer.arg("/F");
    }
    hide_console_window(&mut killer);
    let output = killer
        .output()
        .map_err(|error| format!("failed to start taskkill: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(format!(
        "taskkill exited with {}: {}{}{}",
        output.status,
        stdout,
        if stdout.is_empty() || stderr.is_empty() {
            ""
        } else {
            "; "
        },
        stderr
    ))
}

/// 向应用创建的 Unix 进程组发送信号，避免 Host 的子进程在桌面退出后残留。
#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) -> Result<(), String> {
    let process_group = i32::try_from(pid)
        .map_err(|_| format!("host pid is outside the Unix process id range: {pid}"))?;
    // SAFETY: `process_group` 来自当前应用已启动并持有的子进程 PID；负值只选择该 PID 对应的进程组。
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "failed to signal host process group {pid} with signal {signal}: {error}"
        ))
    }
}

/// 防止 Windows GUI 应用启动控制台子进程时弹出黑色窗口。
#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(windows)]
fn configure_process_group(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[cfg(unix)]
    use super::signal_process_group;
    use super::{
        build_host_path, hindsight_token_from_credentials, HostSupervisor, ManagedChild,
        ProcessTreeTerminator,
    };
    use crate::runtime::RuntimePaths;

    struct FakeChild {
        pid: u32,
        polls: VecDeque<Option<Option<i32>>>,
        wait_code: Option<i32>,
        forced: Option<Arc<AtomicBool>>,
    }

    impl ManagedChild for FakeChild {
        fn id(&self) -> u32 {
            self.pid
        }

        fn try_exit_code(&mut self) -> io::Result<Option<Option<i32>>> {
            Ok(self.polls.pop_front().unwrap_or(None))
        }

        fn wait_for_exit(&mut self) -> io::Result<Option<i32>> {
            Ok(self.wait_code)
        }

        fn force_exit(&mut self) -> io::Result<()> {
            if let Some(forced) = &self.forced {
                forced.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingTerminator {
        requested: Arc<Mutex<Vec<u32>>>,
        forced: Arc<Mutex<Vec<u32>>>,
    }

    impl ProcessTreeTerminator for RecordingTerminator {
        fn request(&self, pid: u32) -> Result<(), String> {
            self.requested.lock().unwrap().push(pid);
            Ok(())
        }

        fn force(&self, pid: u32) -> Result<(), String> {
            self.forced.lock().unwrap().push(pid);
            Ok(())
        }
    }

    struct FailingTerminator;

    impl ProcessTreeTerminator for FailingTerminator {
        fn request(&self, _pid: u32) -> Result<(), String> {
            Err("request denied".to_owned())
        }

        fn force(&self, _pid: u32) -> Result<(), String> {
            Err("force denied".to_owned())
        }
    }

    fn supervisor(child: FakeChild) -> HostSupervisor {
        HostSupervisor {
            child: Mutex::new(Some(Box::new(child))),
        }
    }

    #[test]
    fn reports_real_abnormal_exit_code() {
        let supervisor = supervisor(FakeChild {
            pid: 42,
            polls: VecDeque::from([Some(Some(7))]),
            wait_code: Some(7),
            forced: None,
        });
        assert_eq!(supervisor.try_exit_code(), Some(Some(7)));
    }

    #[test]
    fn missing_node_executable_reports_start_failure() {
        let paths = RuntimePaths {
            node: std::env::temp_dir().join("dsh-desktop-node-does-not-exist.exe"),
            cli_entry: std::env::temp_dir().join("fake-host.js"),
            host_root: std::env::temp_dir(),
            tool_bin_directory: std::env::temp_dir().join("node_modules/.bin"),
            desktop_policy_patch: std::env::temp_dir().join("dsh-market.patch.yml"),
            plugins_root: std::env::temp_dir(),
            user_home: std::env::temp_dir(),
            dsh_home: std::env::temp_dir(),
            web_profile: std::env::temp_dir(),
            managed_plugins_root: std::env::temp_dir(),
            working_directory: std::env::temp_dir(),
            core_ready_timeout: Duration::from_secs(1),
            plugin_ready_timeout: Duration::from_secs(1),
            immutable_plugins: false,
            activation: None,
        };
        let error = HostSupervisor::spawn(&paths).err().unwrap();
        assert!(error.contains("failed to start Node"));
    }

    #[test]
    fn graceful_exit_and_repeated_shutdown_are_idempotent() {
        let supervisor = supervisor(FakeChild {
            pid: 43,
            polls: VecDeque::from([None, Some(Some(0))]),
            wait_code: Some(0),
            forced: None,
        });
        let terminator = RecordingTerminator::default();
        supervisor.shutdown_with(&terminator, Duration::from_secs(1), Duration::ZERO);
        supervisor.shutdown_with(&terminator, Duration::ZERO, Duration::ZERO);
        assert_eq!(*terminator.requested.lock().unwrap(), vec![43]);
        assert!(terminator.forced.lock().unwrap().is_empty());
    }

    #[test]
    fn recovery_does_not_signal_a_host_that_already_exited() {
        let supervisor = supervisor(FakeChild {
            pid: 46,
            polls: VecDeque::from([Some(Some(97))]),
            wait_code: Some(97),
            forced: None,
        });
        let terminator = RecordingTerminator::default();

        supervisor.shutdown_for_recovery_with(&terminator);

        assert!(terminator.requested.lock().unwrap().is_empty());
        assert!(terminator.forced.lock().unwrap().is_empty());
    }

    #[test]
    fn recovery_shutdown_forces_only_the_recorded_process_tree() {
        let supervisor = supervisor(FakeChild {
            pid: 44,
            polls: VecDeque::new(),
            wait_code: Some(1),
            forced: None,
        });
        let terminator = RecordingTerminator::default();
        supervisor.shutdown_for_recovery_with(&terminator);
        assert_eq!(*terminator.requested.lock().unwrap(), vec![44]);
        assert_eq!(*terminator.forced.lock().unwrap(), vec![44]);
    }

    #[test]
    fn taskkill_failure_falls_back_to_owned_child_handle() {
        let forced = Arc::new(AtomicBool::new(false));
        let supervisor = supervisor(FakeChild {
            pid: 45,
            polls: VecDeque::new(),
            wait_code: Some(1),
            forced: Some(forced.clone()),
        });

        supervisor.shutdown_with(&FailingTerminator, Duration::ZERO, Duration::ZERO);

        assert!(forced.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[test]
    fn unix_process_group_rejects_pid_outside_signed_range() {
        let error = signal_process_group(u32::MAX, libc::SIGTERM).unwrap_err();
        assert!(error.contains("outside the Unix process id range"));
    }

    #[test]
    fn host_path_prefers_bundled_node_and_pnpm_before_inherited_path() {
        let app_root = if cfg!(windows) {
            std::path::PathBuf::from(r"C:\app")
        } else {
            std::path::PathBuf::from("/app")
        };
        let inherited_directories = if cfg!(windows) {
            vec![
                std::path::PathBuf::from(r"C:\Windows\System32"),
                std::path::PathBuf::from(r"C:\tools"),
            ]
        } else {
            vec![
                std::path::PathBuf::from("/usr/bin"),
                std::path::PathBuf::from("/opt/tools/bin"),
            ]
        };
        let paths = RuntimePaths {
            node: app_root.join(if cfg!(windows) {
                "node/node.exe"
            } else {
                "node/node"
            }),
            cli_entry: app_root.join("host/node_modules/@deepseek-ai/dsh/lib/bin.js"),
            host_root: app_root.join("host"),
            tool_bin_directory: app_root.join("host/node_modules/.bin"),
            desktop_policy_patch: app_root.join("policy/dsh-market.patch.yml"),
            plugins_root: std::env::temp_dir(),
            user_home: std::env::temp_dir(),
            dsh_home: std::env::temp_dir(),
            web_profile: std::env::temp_dir(),
            managed_plugins_root: std::env::temp_dir(),
            working_directory: std::env::temp_dir(),
            core_ready_timeout: Duration::from_secs(1),
            plugin_ready_timeout: Duration::from_secs(1),
            immutable_plugins: false,
            activation: None,
        };
        let inherited = std::env::join_paths(&inherited_directories).unwrap();

        let actual = build_host_path(&paths, Some(&inherited)).unwrap();
        let directories = std::env::split_paths(&actual).collect::<Vec<_>>();
        assert_eq!(
            directories,
            [
                app_root.join("node"),
                app_root.join("host/node_modules/.bin"),
            ]
            .into_iter()
            .chain(inherited_directories)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn hindsight_credential_bridge_reads_only_the_dedicated_reference() {
        let root = std::env::temp_dir().join(format!(
            "dsh-desktop-host-credentials-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(".credentials.yaml"),
            "OTHER_TOKEN: keep-private\nDSH_DESKTOP_HINDSIGHT_API_TOKEN: hindsight-private\n",
        )
        .unwrap();

        assert_eq!(
            hindsight_token_from_credentials(&root).unwrap(),
            Some("hindsight-private".to_owned())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hindsight_credential_bridge_rejects_non_string_values() {
        let root = std::env::temp_dir().join(format!(
            "dsh-desktop-host-invalid-credentials-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(".credentials.yaml"),
            "DSH_DESKTOP_HINDSIGHT_API_TOKEN: 123\n",
        )
        .unwrap();

        assert!(hindsight_token_from_credentials(&root)
            .unwrap_err()
            .contains("non-empty string"));
        fs::remove_dir_all(root).unwrap();
    }
}
