//! 应用与 Host 之间共享的生命周期类型。

/// Host 监督线程向应用主流程发送的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// Host 已输出可加载的回环地址。
    Ready(String),
    /// Host 进程已经结束。
    Exited,
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
