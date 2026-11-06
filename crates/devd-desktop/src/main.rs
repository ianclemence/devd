#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    devd_core::util::note_process_start();
    devd_desktop::run();
}
