//! DSH Host 子进程创建、状态探测与清理。

use std::io;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::logger::{log_app, log_error};
use crate::runtime::RuntimePaths;

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
}

/// 抽象 Windows 进程树终止动作，测试可记录请求而不操作真实进程。
trait ProcessTreeTerminator {
    /// 请求进程树正常结束。
    fn request(&self, pid: u32);
    /// 强制结束仍未退出的完整进程树。
    fn force(&self, pid: u32);
}

struct WindowsProcessTreeTerminator;

impl ProcessTreeTerminator for WindowsProcessTreeTerminator {
    fn request(&self, pid: u32) {
        run_taskkill(pid, false);
    }

    fn force(&self, pid: u32) {
        run_taskkill(pid, true);
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
            "spawning: {} --expose-internals {} web --host 127.0.0.1 --port 0 (cwd: {})",
            paths.node.display(),
            paths.cli_entry.display(),
            paths.working_directory.display()
        ));

        let mut command = Command::new(&paths.node);
        command
            .arg("--expose-internals")
            .arg(&paths.cli_entry)
            .args(["web", "--host", "127.0.0.1", "--port", "0"])
            .current_dir(&paths.working_directory)
            .env("DSH_HOME", &paths.dsh_home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console_window(&mut command);

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
        self.shutdown_with(
            &WindowsProcessTreeTerminator,
            Duration::from_secs(5),
            Duration::from_millis(100),
        );
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
        log_app(&format!("requesting host shutdown: pid={pid}"));
        terminator.request(pid);

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
        terminator.force(pid);
        match child.wait_for_exit() {
            Ok(exit_code) => log_app(&format!(
                "host process tree reaped: pid={pid}, code={exit_code:?}"
            )),
            Err(error) => log_error(&format!("failed to reap host pid={pid}: {error}")),
        }
    }
}

/// 调用 Windows `taskkill` 处理指定 PID 的完整进程树。
fn run_taskkill(pid: u32, force: bool) {
    let pid_text = pid.to_string();
    let mut killer = Command::new("taskkill");
    killer.args(["/PID", &pid_text, "/T"]);
    if force {
        killer.arg("/F");
    }
    hide_console_window(&mut killer);
    let _ = killer.status();
}

/// 防止 Windows GUI 应用启动控制台子进程时弹出黑色窗口。
#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{HostSupervisor, ManagedChild, ProcessTreeTerminator};
    use crate::runtime::RuntimePaths;

    struct FakeChild {
        pid: u32,
        polls: VecDeque<Option<Option<i32>>>,
        wait_code: Option<i32>,
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
    }

    #[derive(Default)]
    struct RecordingTerminator {
        requested: Arc<Mutex<Vec<u32>>>,
        forced: Arc<Mutex<Vec<u32>>>,
    }

    impl ProcessTreeTerminator for RecordingTerminator {
        fn request(&self, pid: u32) {
            self.requested.lock().unwrap().push(pid);
        }

        fn force(&self, pid: u32) {
            self.forced.lock().unwrap().push(pid);
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
        });
        assert_eq!(supervisor.try_exit_code(), Some(Some(7)));
    }

    #[test]
    fn missing_node_executable_reports_start_failure() {
        let paths = RuntimePaths {
            node: std::env::temp_dir().join("dsh-desktop-node-does-not-exist.exe"),
            cli_entry: std::env::temp_dir().join("fake-host.js"),
            host_root: std::env::temp_dir(),
            plugins_root: std::env::temp_dir(),
            dsh_home: std::env::temp_dir(),
            web_profile: std::env::temp_dir(),
            managed_plugins_root: std::env::temp_dir(),
            working_directory: std::env::temp_dir(),
            readiness_timeout: Duration::from_secs(1),
        };
        let error = HostSupervisor::spawn(&paths).err().unwrap();
        assert!(error.contains("failed to start Node"));
    }

    #[test]
    fn graceful_exit_and_repeated_shutdown_are_idempotent() {
        let supervisor = supervisor(FakeChild {
            pid: 43,
            polls: VecDeque::from([Some(Some(0))]),
            wait_code: Some(0),
        });
        let terminator = RecordingTerminator::default();
        supervisor.shutdown_with(&terminator, Duration::from_secs(1), Duration::ZERO);
        supervisor.shutdown_with(&terminator, Duration::ZERO, Duration::ZERO);
        assert_eq!(*terminator.requested.lock().unwrap(), vec![43]);
        assert!(terminator.forced.lock().unwrap().is_empty());
    }

    #[test]
    fn timeout_forces_only_the_recorded_process_tree() {
        let supervisor = supervisor(FakeChild {
            pid: 44,
            polls: VecDeque::new(),
            wait_code: Some(1),
        });
        let terminator = RecordingTerminator::default();
        supervisor.shutdown_with(&terminator, Duration::ZERO, Duration::ZERO);
        assert_eq!(*terminator.requested.lock().unwrap(), vec![44]);
        assert_eq!(*terminator.forced.lock().unwrap(), vec![44]);
    }
}
