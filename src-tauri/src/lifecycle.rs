//! 应用与 Host 之间共享的生命周期类型。

/// Host 监督线程向应用主流程发送的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// Host 已输出可加载的回环地址。
    Ready(String),
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
    /// 以明确原因和可诊断消息结束应用。
    Fail {
        reason: ShutdownReason,
        message: String,
    },
}

/// 将启动与运行期 Host 事件归一为确定性动作，避免线程时序改变结果。
#[derive(Debug, Default)]
pub struct LifecycleStateMachine {
    ready: bool,
    terminated: bool,
}

impl LifecycleStateMachine {
    /// 创建等待 Host 就绪的状态机。
    pub fn new() -> Self {
        Self::default()
    }

    /// 处理一个 Host 事件；显式退出期间的迟到事件一律忽略。
    pub fn on_event(&mut self, event: HostEvent, shutting_down: bool) -> LifecycleAction {
        if shutting_down || self.terminated {
            return LifecycleAction::Ignore;
        }

        match event {
            HostEvent::Ready(url) if !self.ready => {
                self.ready = true;
                LifecycleAction::Navigate(url)
            }
            HostEvent::Ready(_) => LifecycleAction::Ignore,
            HostEvent::Exited(exit_code) => {
                self.terminated = true;
                let reason = if self.ready {
                    ShutdownReason::HostExited
                } else {
                    ShutdownReason::StartupFailure
                };
                LifecycleAction::Fail {
                    reason,
                    message: format!(
                        "DeepSeek Harness exited {} (exit code: {exit_code:?}).",
                        if self.ready {
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
                    reason: if self.ready {
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
        if shutting_down || self.ready || self.terminated {
            return LifecycleAction::Ignore;
        }
        self.terminated = true;
        LifecycleAction::Fail {
            reason: ShutdownReason::StartupFailure,
            message: format!("DeepSeek Harness did not report a URL within {seconds} seconds."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HostEvent, LifecycleAction, LifecycleStateMachine, ShutdownReason};

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
            running.on_event(HostEvent::Ready("http://localhost:1".to_owned()), false),
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
}
