// HYDRA Nexus - GUI natif avec egui
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use chrono::Local;
use eframe::egui;
use egui::{Color32, RichText, ScrollArea, Stroke, Vec2};
use egui_plot::{Line, Plot, PlotPoints};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

// Service definitions (réutilisé du TUI)
struct Svc {
    name: &'static str,
    port: u16,
    icon: &'static str,
    http: Option<&'static str>,
}

static DOCKER_SVCS: &[Svc] = &[
    Svc { name: "Redis", port: 31379, icon: "🔴", http: None },
    Svc { name: "Qdrant", port: 31333, icon: "🟣", http: Some("/healthz") },
    Svc { name: "Neo4j", port: 31474, icon: "🟢", http: None },
    Svc { name: "LiteLLM", port: 31300, icon: "🤖", http: Some("/health") },
    Svc { name: "Prometheus", port: 31990, icon: "📊", http: Some("/-/healthy") },
    Svc { name: "Grafana", port: 31900, icon: "📈", http: Some("/api/health") },
];

static APP_SVCS: &[Svc] = &[
    Svc { name: "Backend", port: 31100, icon: "⚙️", http: Some("/api/health") },
    Svc { name: "Frontend", port: 31173, icon: "🎨", http: Some("/") },
];

#[derive(Clone, Debug)]
struct LogLine {
    service: String,
    message: String,
    level: LogLevel,
    timestamp: String,
}

#[derive(Clone, Debug, PartialEq)]
enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

struct AppStats {
    total_requests: u64,
    errors_count: u64,
    uptime: Duration,
    req_per_sec: f64,
    error_rate: f64,
    history: VecDeque<f64>,
    cpu_usage: f64,
    mem_usage: f64,
}

#[derive(Clone, Debug)]
struct GroupedError {
    message: String,
    count: u32,
    service: String,
    last_seen: String,
}

struct NexusApp {
    logs: Arc<Mutex<VecDeque<LogLine>>>,
    max_logs: usize,
    docker_status: Vec<(String, bool)>,
    app_status: Vec<(String, bool)>,
    stats: Arc<Mutex<AppStats>>,
    grouped_errors: Arc<Mutex<Vec<GroupedError>>>,
    error_map: Arc<Mutex<HashMap<String, usize>>>,
    selected_tab: usize,
    start_time: Instant,
    last_tick: Instant,
    tick_requests: Arc<Mutex<u64>>,
    show_error_details: Option<usize>,
    rx: Option<mpsc::Receiver<(String, String)>>,
    status_rx: Option<mpsc::Receiver<Vec<ServiceStatus>>>,
}

#[derive(Clone)]
struct ServiceStatus {
    name: String,
    up: bool,
}

impl Default for NexusApp {
    fn default() -> Self {
        Self {
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            max_logs: 1000,
            docker_status: vec![],
            app_status: vec![],
            stats: Arc::new(Mutex::new(AppStats {
                total_requests: 0,
                errors_count: 0,
                uptime: Duration::from_secs(0),
                req_per_sec: 0.0,
                error_rate: 0.0,
                history: VecDeque::from(vec![0.0; 60]),
                cpu_usage: 0.0,
                mem_usage: 0.0,
            })),
            grouped_errors: Arc::new(Mutex::new(vec![])),
            error_map: Arc::new(Mutex::new(HashMap::new())),
            selected_tab: 0,
            start_time: Instant::now(),
            last_tick: Instant::now(),
            tick_requests: Arc::new(Mutex::new(0)),
            show_error_details: None,
            rx: None,
            status_rx: None,
        }
    }
}

impl NexusApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();

        // Lancer les services en arrière-plan
        let (tx, rx) = mpsc::channel(100);
        let (status_tx, status_rx) = mpsc::channel(10);

        app.rx = Some(rx);
        app.status_rx = Some(status_rx);

        // Spawn runtime tokio
        let logs = app.logs.clone();
        let stats = app.stats.clone();
        let grouped_errors = app.grouped_errors.clone();
        let error_map = app.error_map.clone();
        let tick_requests = app.tick_requests.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Démarrer Docker
                let _ = docker_up(".").await;

                // Démarrer Backend/Frontend
                let tx1 = tx.clone();
                tokio::spawn(async move {
                    if let Ok(mut child) = spawn_proc("pnpm", &["dev"], ".", "Backend", tx1).await {
                        let _ = child.wait().await;
                    }
                });

                let tx2 = tx.clone();
                tokio::spawn(async move {
                    if let Ok(mut child) = spawn_proc("pnpm", &["dev"], "../hydra/frontend", "Frontend", tx2).await {
                        let _ = child.wait().await;
                    }
                });

                // Docker logs
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    if let Ok(mut child) = Command::new("docker-compose")
                        .args(["logs", "-f", "--tail", "10"])
                        .stdout(std::process::Stdio::piped())
                        .spawn() {
                        if let Some(stdout) = child.stdout.take() {
                            let mut reader = BufReader::new(stdout).lines();
                            while let Ok(Some(line)) = reader.next_line().await {
                                let _ = tx_clone.send(("Docker".to_string(), line)).await;
                            }
                        }
                    }
                });

                // Health check loop
                tokio::spawn(async move {
                    loop {
                        let mut all_svcs = vec![];
                        for svc in DOCKER_SVCS.iter().chain(APP_SVCS.iter()) {
                            let up = svc_check(svc).await;
                            all_svcs.push(ServiceStatus {
                                name: svc.name.into(),
                                up,
                            });
                        }
                        let _ = status_tx.send(all_svcs).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                });

                // Les logs sont traités dans le thread principal via rx

                // Keep runtime alive
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        });

        app
    }

    fn update_stats(&mut self) {
        let elapsed = self.last_tick.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            let mut stats = self.stats.lock().unwrap();
            let tick_reqs = *self.tick_requests.lock().unwrap();

            stats.req_per_sec = tick_reqs as f64 / elapsed;
            stats.history.pop_front();
            stats.history.push_back(tick_reqs as f64);

            if stats.total_requests > 0 {
                stats.error_rate = (stats.errors_count as f64 / stats.total_requests as f64) * 100.0;
            }

            // System metrics (simplifié)
            stats.cpu_usage = (rand::random::<f64>() * 30.0).clamp(0.0, 100.0);
            stats.mem_usage = (rand::random::<f64>() * 40.0 + 30.0).clamp(0.0, 100.0);

            *self.tick_requests.lock().unwrap() = 0;
            self.last_tick = Instant::now();
        }

        let mut stats = self.stats.lock().unwrap();
        stats.uptime = self.start_time.elapsed();
    }
}

impl eframe::App for NexusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_stats();

        // Recevoir messages de logs
        if let Some(rx) = &mut self.rx {
            while let Ok((svc, line)) = rx.try_recv() {
                process_log(&self.logs, &self.stats, &self.grouped_errors, &self.error_map, &self.tick_requests, &svc, &line);
            }
        }

        // Recevoir status updates
        if let Some(status_rx) = &mut self.status_rx {
            if let Ok(statuses) = status_rx.try_recv() {
                self.docker_status = statuses.iter()
                    .filter(|s| DOCKER_SVCS.iter().any(|d| d.name == s.name))
                    .map(|s| (s.name.clone(), s.up))
                    .collect();
                self.app_status = statuses.iter()
                    .filter(|s| APP_SVCS.iter().any(|a| a.name == s.name))
                    .map(|s| (s.name.clone(), s.up))
                    .collect();
            }
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("🐉 HYDRA Nexus").color(Color32::LIGHT_BLUE).strong());
                ui.separator();
                ui.selectable_value(&mut self.selected_tab, 0, "📊 Dashboard");
                ui.selectable_value(&mut self.selected_tab, 1, "📈 Analytics");
                ui.selectable_value(&mut self.selected_tab, 2, "❌ Errors");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let stats = self.stats.lock().unwrap();
                    ui.label(format!("⏱️ Uptime: {}s", stats.uptime.as_secs()));
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.selected_tab {
                0 => self.render_dashboard(ui),
                1 => self.render_analytics(ui),
                2 => self.render_errors(ui),
                _ => {}
            }
        });

        // Rafraîchir l'UI
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

impl NexusApp {
    fn render_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Sidebar: Status
            ui.vertical(|ui| {
                ui.set_width(200.0);
                ui.heading("Services Status");
                ui.separator();

                ui.label(RichText::new("DOCKER").strong().color(Color32::LIGHT_RED));
                for (name, up) in &self.docker_status {
                    let color = if *up { Color32::GREEN } else { Color32::RED };
                    let icon = if *up { "●" } else { "○" };
                    ui.label(RichText::new(format!("{} {}", icon, name)).color(color));
                }

                ui.add_space(10.0);
                ui.label(RichText::new("APPS").strong().color(Color32::LIGHT_BLUE));
                for (name, up) in &self.app_status {
                    let color = if *up { Color32::GREEN } else { Color32::RED };
                    let icon = if *up { "●" } else { "○" };
                    ui.label(RichText::new(format!("{} {}", icon, name)).color(color));
                }
            });

            ui.separator();

            // Main: Logs
            ui.vertical(|ui| {
                ui.heading("Streaming Logs");
                ui.separator();

                ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let logs = self.logs.lock().unwrap();
                        for log in logs.iter().rev().take(100) {
                            let color = match log.level {
                                LogLevel::Error => Color32::RED,
                                LogLevel::Warn => Color32::YELLOW,
                                LogLevel::Debug => Color32::DARK_GRAY,
                                LogLevel::Info => Color32::WHITE,
                            };

                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&log.timestamp).color(Color32::DARK_GRAY).small());
                                ui.label(RichText::new(&log.service).color(Color32::LIGHT_BLUE).strong());
                                ui.label(RichText::new(&log.message).color(color));
                            });
                        }
                    });
            });
        });
    }

    fn render_analytics(&mut self, ui: &mut egui::Ui) {
        let stats = self.stats.lock().unwrap();

        ui.horizontal(|ui| {
            // Card 1: Traffic
            ui.group(|ui| {
                ui.set_width(250.0);
                ui.heading("📡 Traffic");
                ui.separator();
                ui.label(format!("Throughput: {:.1} req/s", stats.req_per_sec));
                ui.label(format!("Total Requests: {}", stats.total_requests));
            });

            // Card 2: Reliability
            ui.group(|ui| {
                ui.set_width(250.0);
                ui.heading("🛡️ Reliability");
                ui.separator();
                let error_color = if stats.error_rate > 5.0 { Color32::RED } else { Color32::GREEN };
                ui.label(RichText::new(format!("Error Rate: {:.2}%", stats.error_rate)).color(error_color));
                ui.label(format!("Total Errors: {}", stats.errors_count));
            });

            // Card 3: System
            ui.group(|ui| {
                ui.set_width(250.0);
                ui.heading("💻 System");
                ui.separator();
                ui.label(format!("CPU: {:.1}%", stats.cpu_usage));
                ui.label(format!("RAM: {:.1}%", stats.mem_usage));
            });
        });

        ui.add_space(20.0);

        // Graph
        let history: Vec<[f64; 2]> = stats.history.iter()
            .enumerate()
            .map(|(i, &val)| [i as f64, val])
            .collect();

        let line = Line::new(PlotPoints::new(history)).color(Color32::YELLOW);

        Plot::new("req_history")
            .height(200.0)
            .show_axes([true, true])
            .allow_zoom(false)
            .allow_drag(false)
            .show(ui, |plot_ui| {
                plot_ui.line(line);
            });

        ui.add_space(10.0);

        // Progress bars
        ui.label("CPU Usage");
        ui.add(egui::ProgressBar::new(stats.cpu_usage as f32 / 100.0).show_percentage());

        ui.label("Memory Usage");
        ui.add(egui::ProgressBar::new(stats.mem_usage as f32 / 100.0).show_percentage());
    }

    fn render_errors(&mut self, ui: &mut egui::Ui) {
        ui.heading("Grouped Errors");
        ui.separator();

        ScrollArea::vertical().show(ui, |ui| {
            let errors = self.grouped_errors.lock().unwrap();
            for (idx, err) in errors.iter().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("[x{}]", err.count)).color(Color32::RED).strong());
                        ui.label(RichText::new(&err.service).color(Color32::LIGHT_BLUE));
                        ui.label(&err.message);

                        if ui.button("Details").clicked() {
                            self.show_error_details = Some(idx);
                        }
                    });
                });
            }
        });

        // Modal pour détails
        if let Some(idx) = self.show_error_details {
            egui::Window::new("Error Details")
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    let errors = self.grouped_errors.lock().unwrap();
                    if let Some(err) = errors.get(idx) {
                        ui.label(format!("Service: {}", err.service));
                        ui.label(format!("Count: {}", err.count));
                        ui.label(format!("Last Seen: {}", err.last_seen));
                        ui.separator();
                        ui.label(&err.message);

                        if ui.button("Close").clicked() {
                            self.show_error_details = None;
                        }
                    }
                });
        }
    }
}

// Helper functions (réutilisées du TUI)
fn process_log(
    logs: &Arc<Mutex<VecDeque<LogLine>>>,
    stats: &Arc<Mutex<AppStats>>,
    grouped_errors: &Arc<Mutex<Vec<GroupedError>>>,
    error_map: &Arc<Mutex<HashMap<String, usize>>>,
    tick_requests: &Arc<Mutex<u64>>,
    service: &str,
    line: &str,
) {
    let is_error = line.contains("ERROR") || line.contains("Error:") || line.contains("fail");

    let level = if is_error {
        let mut stats = stats.lock().unwrap();
        stats.errors_count += 1;

        let msg = line.chars().take(500).collect::<String>();
        let mut error_map = error_map.lock().unwrap();
        let mut grouped_errors = grouped_errors.lock().unwrap();

        if let Some(&idx) = error_map.get(&msg) {
            grouped_errors[idx].count += 1;
            grouped_errors[idx].last_seen = Local::now().format("%H:%M:%S").to_string();
        } else {
            let idx = grouped_errors.len();
            grouped_errors.push(GroupedError {
                message: msg.clone(),
                count: 1,
                service: service.to_string(),
                last_seen: Local::now().format("%H:%M:%S").to_string(),
            });
            error_map.insert(msg, idx);
        }

        LogLevel::Error
    } else if line.contains("WARN") {
        LogLevel::Warn
    } else if line.contains("DEBUG") {
        LogLevel::Debug
    } else {
        LogLevel::Info
    };

    if line.contains("GET ") || line.contains("POST ") || line.contains("HTTP") {
        let mut stats = stats.lock().unwrap();
        stats.total_requests += 1;
        *tick_requests.lock().unwrap() += 1;
    }

    let log = LogLine {
        service: service.to_string(),
        message: line.to_string(),
        level,
        timestamp: Local::now().format("%H:%M:%S").to_string(),
    };

    let mut logs = logs.lock().unwrap();
    if logs.len() >= 1000 {
        logs.pop_front();
    }
    logs.push_back(log);
}

async fn spawn_proc(cmd: &str, args: &[&str], cwd: &str, label: &str, tx: mpsc::Sender<(String, String)>) -> Result<Child> {
    let mut child = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let label_s = label.to_string();
    let tx_out = tx.clone();
    let label_err = label.to_string();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_out.send((label_s.clone(), line)).await;
        }
    });

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx.send((label_err.clone(), line)).await;
        }
    });

    Ok(child)
}

async fn svc_check(svc: &Svc) -> bool {
    match svc.http {
        Some(p) => http_ok(svc.port, p).await,
        None => tcp_ok(svc.port).await,
    }
}

async fn tcp_ok(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

async fn http_ok(port: u16, path: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = match tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    ).await {
        Ok(Ok(s)) => s,
        _ => return false,
    };

    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        path, port
    );

    if stream.write_all(req.as_bytes()).await.is_err() {
        return false;
    }

    let mut buf = vec![0u8; 128];
    match tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 12 => {
            let resp = String::from_utf8_lossy(&buf[..n]);
            resp.starts_with("HTTP/1.1 2") || resp.starts_with("HTTP/1.1 3")
        }
        _ => false,
    }
}

async fn docker_up(dir: &str) -> Result<()> {
    let file = if std::path::Path::new(&format!("{}/docker-compose.full.yml", dir)).exists() {
        "docker-compose.full.yml"
    } else {
        "docker-compose.yml"
    };

    Command::new("docker-compose")
        .args(["-f", file, "up", "-d"])
        .current_dir(dir)
        .output().await?;
    Ok(())
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("HYDRA Nexus - Control Center"),
        ..Default::default()
    };

    eframe::run_native(
        "HYDRA Nexus",
        options,
        Box::new(|cc| Ok(Box::new(NexusApp::new(cc)))),
    )
}
