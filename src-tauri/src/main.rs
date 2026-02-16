// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    // If we have arguments (like "dev", "stop", "status"), we might want to run the CLI version.
    // However, Tauri 2.0 has its own CLI plugin system.
    // For simplicity, we check if the first arg is one of our commands.
    
    if args.len() > 1 {
        let cmd = &args[1];
        if cmd == "dev" || cmd == "stop" || cmd == "status" {
            // We can't easily call the TUI from here without including all its code.
            // For now, let's just use the Tauri run. 
            // In a real scenario, we'd refactor the TUI logic into a library.
        }
    }

    app_lib::run();
}
