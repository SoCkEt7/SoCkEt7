mod ecosystem;

use ecosystem::{DOCKER_SVCS, APP_SVCS, svc_check, ServiceStatus, docker_up, docker_down, cleanup_procs};
use serde::Serialize;
use std::time::Duration;
use std::collections::VecDeque;

#[derive(Serialize, Clone)]
pub struct SystemMetrics {
    cpu_usage: f64,
    mem_usage: f64,
    uptime: u64,
}

#[tauri::command]
async fn get_service_status() -> Vec<ServiceStatus> {
    let mut statuses = Vec::new();
    for svc in DOCKER_SVCS.iter().chain(APP_SVCS.iter()) {
        let up = svc_check(svc).await;
        statuses.push(ServiceStatus {
            name: svc.name.to_string(),
            up,
            icon: svc.icon.to_string(),
        });
    }
    statuses
}

#[tauri::command]
async fn start_ecosystem(app_handle: tauri::AppHandle) -> Result<(), String> {
    // Hardcoded path to the hydra directory in the ecosystem
    let dir = "/home/antonin/app/projects/Hydra-ecosystem/hydra";
    
    cleanup_procs().await;
    docker_up(dir).await.map_err(|e| e.to_string())?;
    
    // Start backend and frontend in background
    let dir_clone = dir.to_string();
    tokio::spawn(async move {
        let _ = tokio::process::Command::new("pnpm")
            .args(["dev"])
            .current_dir(&dir_clone)
            .spawn();
    });

    let dir_fe = format!("{}/frontend", dir);
    tokio::spawn(async move {
        let _ = tokio::process::Command::new("pnpm")
            .args(["dev"])
            .current_dir(&dir_fe)
            .spawn();
    });

    Ok(())
}

#[tauri::command]
async fn stop_ecosystem() -> Result<(), String> {
    let dir = "/home/antonin/app/projects/Hydra-ecosystem/hydra";
    cleanup_procs().await;
    docker_down(dir).await;
    Ok(())
}

#[tauri::command]
async fn get_system_metrics() -> SystemMetrics {
    let mut cpu_usage = 0.0;
    if let Ok(output) = std::process::Command::new("top").args(["-bn1"]).output() {
        let s = String::from_utf8_lossy(&output.stdout);
        if let Some(line) = s.lines().find(|l| l.contains("%Cpu(s)")) {
            if let Some(idle) = line.split(',').find(|p| p.contains("id")) {
                if let Some(val) = idle.trim().split_whitespace().next() {
                    if let Ok(idle_val) = val.parse::<f64>() {
                        cpu_usage = 100.0 - idle_val;
                    }
                }
            }
        }
    }

    let mut mem_usage = 0.0;
    if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
        let total = mem.lines().find(|l| l.starts_with("MemTotal")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
        let free = mem.lines().find(|l| l.starts_with("MemAvailable")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        mem_usage = ((total - free) / total) * 100.0;
    }

    SystemMetrics {
        cpu_usage,
        mem_usage,
        uptime: 0, // Simplified
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            get_service_status,
            start_ecosystem,
            stop_ecosystem,
            get_system_metrics
        ])
        .setup(|app| {
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
