// Windows 上不要开控制台窗口。
//
// **缺了这一行，双击安装好的程序会先弹出一个黑框再出界面**，关掉那个黑框
// 会把程序一起杀掉。Tauri 的模板自带这一行，本项目一直没有 —— 因为
// macOS 与 Linux 上它不起作用，开发机上永远看不出来。
//
// `not(debug_assertions)`：调试构建仍然要那个控制台，`println!` 得有地方去。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    colm_desktop_gui_lib::run()
}
