// Keeps a second console window from appearing next to the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sdc_lib::run();
}
