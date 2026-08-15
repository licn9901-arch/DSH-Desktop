//! DSH Host 子进程创建、状态探测与清理。

use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::logger::{log_app, log_error};
use crate::runtime::RuntimePaths;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 持有唯一 Host 子进程，供启动线程、退出回调和监视线程共享。
pub struct HostSupervisor {
    child: Mutex<Option<Child>>,
}

impl HostSupervisor {
    /// 启动 DSH Web Host，并返回可分别消费的 stdout 与 stderr。
    pub fn spawn(paths: &RuntimePaths) -> Result<(Self, ChildStdout, ChildStderr), String> {
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

        Ok((
            Self {
                child: Mutex::new(Some(child)),
            },
            stdout,
            stderr,
        ))
    }

    /// 非阻塞读取 Host 退出码；外层 `None` 表示仍在运行，内层 `None` 表示无可用退出码。
    pub fn try_exit_code(&self) -> Option<Option<i32>> {
        let mut guard = self.child.lock().unwrap_or_else(|error| error.into_inner());
        match guard.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => Some(status.code()),
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
            .map(Child::id)
    }

    /// 终止 Host 进程树并回收子进程；重复调用不会产生额外副作用。
    pub fn shutdown(&self) {
        let child = self
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let Some(mut child) = child else {
            return;
        };

        let pid = child.id();
        let pid_text = pid.to_string();
        log_app(&format!("requesting host shutdown: pid={pid}"));
        let mut killer = Command::new("taskkill");
        killer.args(["/PID", &pid_text, "/T"]);
        hide_console_window(&mut killer);
        let _ = killer.status();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(status)) => {
                    log_app(&format!("host exited: pid={pid}, code={:?}", status.code()));
                    return;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(error) => {
                    log_error(&format!("failed while waiting for host pid={pid}: {error}"));
                    break;
                }
            }
        }

        log_error(&format!("forcing host process tree shutdown: pid={pid}"));
        let mut force_killer = Command::new("taskkill");
        force_killer.args(["/PID", &pid_text, "/T", "/F"]);
        hide_console_window(&mut force_killer);
        let _ = force_killer.status();
        match child.wait() {
            Ok(status) => log_app(&format!(
                "host process tree reaped: pid={pid}, code={:?}",
                status.code()
            )),
            Err(error) => log_error(&format!("failed to reap host pid={pid}: {error}")),
        }
    }
}

/// 防止 Windows GUI 应用启动控制台子进程时弹出黑色窗口。
#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}
