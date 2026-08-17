//! 应用与 Host 之间共享的生命周期类型。

use std::sync::{mpsc, Arc, Mutex};

/// 桌面托管 Host 的运行状态，用于限制重启与退出操作的并发关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRuntimeState {
    /// Host 正在进行首次启动。
    Starting,
    /// Host 已通过就绪检查并可供 WebView 使用。
    Ready,
    /// 桌面协调线程正在串行替换 Host 进程。
    Restarting,
    /// 应用正在退出，不再接受重启。
    Stopping,
    /// 最近一次启动或重启失败，可由用户再次尝试重启。
    Failed,
}

/// 管理 Host 运行状态转换，避免托盘重复点击触发并行重启。
#[derive(Debug)]
pub struct HostStateMachine {
    state: HostRuntimeState,
}

impl Default for HostStateMachine {
    /// 创建处于首次启动阶段的状态机。
    fn default() -> Self {
        Self::new()
    }
}

impl HostStateMachine {
    /// 创建 Host 状态机，初始状态为 `Starting`。
    pub fn new() -> Self {
        Self {
            state: HostRuntimeState::Starting,
        }
    }

    /// 返回当前 Host 运行状态。
    pub fn state(&self) -> HostRuntimeState {
        self.state
    }

    /// 标记 Host 已完成就绪检查。
    pub fn mark_ready(&mut self) {
        self.state = HostRuntimeState::Ready;
    }

    /// 尝试进入重启状态；仅 Ready 或 Failed 状态允许发起重启。
    pub fn begin_restart(&mut self) -> Result<(), String> {
        match self.state {
            HostRuntimeState::Ready | HostRuntimeState::Failed => {
                self.state = HostRuntimeState::Restarting;
                Ok(())
            }
            state => Err(format!("host cannot restart while in state {state:?}")),
        }
    }

    /// 标记应用进入退出流程，后续重启请求都会被拒绝。
    pub fn mark_stopping(&mut self) {
        self.state = HostRuntimeState::Stopping;
    }

    /// 标记最近一次 Host 启动或重启失败。
    pub fn mark_failed(&mut self) {
        self.state = HostRuntimeState::Failed;
    }
}

/// Host 协调线程可接收的桌面内部命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostCommand {
    /// 串行停止并重新启动当前 DSH Host。
    Restart,
}

/// 桌面内部 Host 控制器，只暴露受状态机保护的重启与状态更新能力。
pub struct HostController {
    sender: mpsc::Sender<HostCommand>,
    state: Arc<Mutex<HostStateMachine>>,
}

impl HostController {
    /// 创建控制器和唯一命令接收端，接收端应由 Host 协调线程独占。
    pub(crate) fn new() -> (Self, mpsc::Receiver<HostCommand>) {
        let (sender, receiver) = mpsc::channel();
        (
            Self {
                sender,
                state: Arc::new(Mutex::new(HostStateMachine::new())),
            },
            receiver,
        )
    }

    /// 请求串行重启；忙碌、启动中或退出中会返回错误且不发送命令。
    pub fn restart(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.begin_restart()?;
        if self.sender.send(HostCommand::Restart).is_err() {
            state.mark_failed();
            return Err("host controller is not available".to_owned());
        }
        Ok(())
    }

    /// 标记当前 Host 已就绪。
    pub(crate) fn mark_ready(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .mark_ready();
    }

    /// 标记 Host 启动或重启失败。
    pub(crate) fn mark_failed(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .mark_failed();
    }

    /// 标记应用正在退出，拒绝后续重启请求。
    pub(crate) fn mark_stopping(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .mark_stopping();
    }
}

/// Host 监督线程向应用主流程发送的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// Host 核心服务已输出可加载的回环地址。
    CoreReady(String),
    /// Host 全部 Loader 插件已经完成。
    PluginsReady(String),
    /// Host 进程已经结束。
    Exited(Option<i32>),
    /// Host 输出了互相冲突的就绪地址。
    ProtocolError(String),
}

/// 应用触发清理流程的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    /// 用户或系统要求正常退出应用。
    ApplicationExit,
    /// Host 在就绪前启动失败。
    StartupFailure,
    /// Host 在应用运行期间异常退出。
    HostExited,
}

/// 生命周期状态机要求桌面层执行的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleAction {
    /// 当前事件不需要额外动作。
    Ignore,
    /// 导航到已验证的 Host 地址。
    Navigate(String),
    /// 全部插件已经完成，可以提交托管插件事务。
    PluginsReady,
    /// 插件阶段失败或超时，但核心页面仍可继续使用。
    PluginDegraded { message: String },
    /// 以明确原因和可诊断消息结束应用。
    Fail {
        reason: ShutdownReason,
        message: String,
    },
}

/// 将启动与运行期 Host 事件归一为确定性动作，避免线程时序改变结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStartupPhase {
    /// 正在等待核心 Web 服务。
    StartingCore,
    /// 核心页面可交互，插件仍可能在后台加载。
    CoreReady,
    /// 全部 Loader 插件已经完成。
    PluginsReady,
}

/// 两级就绪状态机，确保插件失败不会错误地终止可用核心页面。
#[derive(Debug)]
pub struct LifecycleStateMachine {
    phase: HostStartupPhase,
    terminated: bool,
}

impl Default for LifecycleStateMachine {
    fn default() -> Self {
        Self {
            phase: HostStartupPhase::StartingCore,
            terminated: false,
        }
    }
}

impl LifecycleStateMachine {
    /// 创建等待 Host 就绪的状态机。
    pub fn new() -> Self {
        Self::default()
    }

    /// 返回当前启动阶段，供协调线程选择核心或插件超时策略。
    pub fn phase(&self) -> HostStartupPhase {
        self.phase
    }

    /// 处理一个 Host 事件；显式退出期间的迟到事件一律忽略。
    pub fn on_event(&mut self, event: HostEvent, shutting_down: bool) -> LifecycleAction {
        if shutting_down || self.terminated {
            return LifecycleAction::Ignore;
        }

        match event {
            HostEvent::CoreReady(url) if self.phase == HostStartupPhase::StartingCore => {
                self.phase = HostStartupPhase::CoreReady;
                LifecycleAction::Navigate(url)
            }
            HostEvent::CoreReady(_) => LifecycleAction::Ignore,
            HostEvent::PluginsReady(_) if self.phase == HostStartupPhase::StartingCore => {
                LifecycleAction::Ignore
            }
            HostEvent::PluginsReady(_) if self.phase == HostStartupPhase::CoreReady => {
                self.phase = HostStartupPhase::PluginsReady;
                LifecycleAction::PluginsReady
            }
            HostEvent::PluginsReady(_) => LifecycleAction::Ignore,
            HostEvent::Exited(exit_code) => {
                self.terminated = true;
                let reason = if self.phase != HostStartupPhase::StartingCore {
                    ShutdownReason::HostExited
                } else {
                    ShutdownReason::StartupFailure
                };
                LifecycleAction::Fail {
                    reason,
                    message: format!(
                        "DeepSeek Harness exited {} (exit code: {exit_code:?}).",
                        if self.phase != HostStartupPhase::StartingCore {
                            "unexpectedly"
                        } else {
                            "before becoming ready"
                        }
                    ),
                }
            }
            HostEvent::ProtocolError(message) => {
                self.terminated = true;
                LifecycleAction::Fail {
                    reason: if self.phase != HostStartupPhase::StartingCore {
                        ShutdownReason::HostExited
                    } else {
                        ShutdownReason::StartupFailure
                    },
                    message: format!("DeepSeek Harness readiness protocol error: {message}"),
                }
            }
        }
    }

    /// 处理就绪等待超时；就绪后或退出期间的超时不会改变状态。
    pub fn on_timeout(&mut self, shutting_down: bool, seconds: u64) -> LifecycleAction {
        if shutting_down || self.phase != HostStartupPhase::StartingCore || self.terminated {
            return LifecycleAction::Ignore;
        }
        self.terminated = true;
        LifecycleAction::Fail {
            reason: ShutdownReason::StartupFailure,
            message: format!("DeepSeek Harness did not report a URL within {seconds} seconds."),
        }
    }

    /// 处理插件完成超时；只降级插件，不终止已经可用的核心页面。
    pub fn on_plugins_timeout(&mut self, shutting_down: bool, seconds: u64) -> LifecycleAction {
        if shutting_down || self.phase != HostStartupPhase::CoreReady || self.terminated {
            return LifecycleAction::Ignore;
        }
        LifecycleAction::PluginDegraded {
            message: format!("DeepSeek Harness plugins did not finish within {seconds} seconds."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HostEvent, HostRuntimeState, HostStateMachine, LifecycleAction, LifecycleStateMachine,
        ShutdownReason,
    };

    #[test]
    fn core_and_plugins_have_distinct_state_transitions() {
        let mut machine = LifecycleStateMachine::new();
        assert_eq!(machine.phase(), super::HostStartupPhase::StartingCore);
        assert!(matches!(
            machine.on_event(HostEvent::CoreReady("http://localhost:1".to_owned()), false),
            LifecycleAction::Navigate(_)
        ));
        assert_eq!(machine.phase(), super::HostStartupPhase::CoreReady);
        assert_eq!(
            machine.on_event(
                HostEvent::PluginsReady("http://localhost:1".to_owned()),
                false
            ),
            LifecycleAction::PluginsReady
        );
        assert_eq!(machine.phase(), super::HostStartupPhase::PluginsReady);
    }

    #[test]
    fn plugin_timeout_keeps_the_core_alive() {
        let mut machine = LifecycleStateMachine::new();
        machine.on_event(HostEvent::CoreReady("http://localhost:1".to_owned()), false);
        assert!(matches!(
            machine.on_plugins_timeout(false, 30),
            LifecycleAction::PluginDegraded { .. }
        ));
        assert_eq!(machine.phase(), super::HostStartupPhase::CoreReady);
    }

    #[test]
    fn timeout_is_a_startup_failure_without_waiting() {
        let mut machine = LifecycleStateMachine::new();
        assert!(matches!(
            machine.on_timeout(false, 9),
            LifecycleAction::Fail {
                reason: ShutdownReason::StartupFailure,
                ..
            }
        ));
    }

    #[test]
    fn startup_and_runtime_exits_have_distinct_reasons() {
        let mut startup = LifecycleStateMachine::new();
        assert!(matches!(
            startup.on_event(HostEvent::Exited(Some(2)), false),
            LifecycleAction::Fail {
                reason: ShutdownReason::StartupFailure,
                ..
            }
        ));

        let mut running = LifecycleStateMachine::new();
        assert!(matches!(
            running.on_event(HostEvent::CoreReady("http://localhost:1".to_owned()), false),
            LifecycleAction::Navigate(_)
        ));
        assert!(matches!(
            running.on_event(HostEvent::Exited(Some(7)), false),
            LifecycleAction::Fail {
                reason: ShutdownReason::HostExited,
                ..
            }
        ));
    }

    #[test]
    fn duplicate_exit_and_shutdown_races_are_ignored() {
        let mut machine = LifecycleStateMachine::new();
        assert!(matches!(
            machine.on_event(HostEvent::Exited(None), false),
            LifecycleAction::Fail { .. }
        ));
        assert_eq!(
            machine.on_event(HostEvent::Exited(None), false),
            LifecycleAction::Ignore
        );

        let mut shutting_down = LifecycleStateMachine::new();
        assert_eq!(
            shutting_down.on_event(HostEvent::Exited(Some(1)), true),
            LifecycleAction::Ignore
        );
        assert_eq!(shutting_down.on_timeout(true, 1), LifecycleAction::Ignore);
    }

    #[test]
    fn runtime_state_serializes_restart_and_allows_retry_after_failure() {
        let mut machine = HostStateMachine::new();
        assert_eq!(machine.state(), HostRuntimeState::Starting);
        machine.mark_ready();
        assert_eq!(machine.state(), HostRuntimeState::Ready);
        assert!(machine.begin_restart().is_ok());
        assert_eq!(machine.state(), HostRuntimeState::Restarting);
        assert!(machine.begin_restart().is_err());

        machine.mark_failed();
        assert_eq!(machine.state(), HostRuntimeState::Failed);
        assert!(machine.begin_restart().is_ok());
        machine.mark_stopping();
        assert_eq!(machine.state(), HostRuntimeState::Stopping);
        assert!(machine.begin_restart().is_err());
    }

    #[test]
    fn controller_sends_only_one_restart_while_busy() {
        let (controller, receiver) = super::HostController::new();
        controller.mark_ready();

        assert!(controller.restart().is_ok());
        assert_eq!(receiver.try_recv(), Ok(super::HostCommand::Restart));
        assert!(controller.restart().is_err());

        controller.mark_failed();
        assert!(controller.restart().is_ok());
        assert_eq!(receiver.try_recv(), Ok(super::HostCommand::Restart));
    }
}
