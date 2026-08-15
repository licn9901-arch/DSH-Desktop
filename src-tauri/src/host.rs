//! DSH Host 子进程创建、状态探测与清理。

use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::logger::log_app;
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

    /// 非阻塞检查 Host 是否已经退出；无法读取状态时按退出处理。
    pub fn has_exited(&self) -> bool {
        let mut guard = self.child.lock().unwrap_or_else(|error| error.into_inner());
        match guard.as_mut() {
            Some(child) => child
                .try_wait()
                .map(|status| status.is_some())
                .unwrap_or(true),
            None => true,
        }
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

        let pid = child.id().to_string();
        let mut killer = Command::new("taskkill");
        killer.args(["/PID", &pid, "/T"]);
        hide_console_window(&mut killer);
        let _ = killer.status();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// 防止 Windows GUI 应用启动控制台子进程时弹出黑色窗口。
#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}
