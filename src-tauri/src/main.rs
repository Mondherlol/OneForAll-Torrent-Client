// Pas de console noire en release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    oneforall_lib::run()
}
