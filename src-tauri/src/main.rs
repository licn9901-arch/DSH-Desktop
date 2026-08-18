//! Windows 桌面应用入口。

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    if let Some(exit_code) = dsh_desktop::internal_command::run_if_requested() {
        std::process::exit(exit_code);
    }
    dsh_desktop::run();
}
