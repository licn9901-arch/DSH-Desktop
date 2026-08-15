//! Windows 桌面应用入口。

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    dsh_desktop::run();
}
