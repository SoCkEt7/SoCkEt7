#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ecosystem_config;
mod project_scanner;
mod logger;

use anyhow::Result;
use chrono::Local;
use eframe::egui;
use egui::{Color32, RichText, ScrollArea};
use ecosystem_config::{EcosystemsConfig, Ecosystem, Service};
use project_scanner::{ProjectScanner, DiscoveredProject};
use logger::Logger;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use egui::IconData;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
struct LogLine {
    ecosystem: String,
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
}

struct NexusApp {
    // Configuration
    config: EcosystemsConfig,
    discovered_projects: Vec<DiscoveredProject>,

    // État actuel
    selected_ecosystem: Option<String>,
    selected_environment: String,  // "dev" ou "prod"
    selected_tab: usize,

    // Services status
    services_status: Arc<Mutex<HashMap<String, Vec<ServiceStatus>>>>,

    // Logs
    logs: Arc<Mutex<VecDeque<LogLine>>>,
    max_logs: usize,

    // Stats
    stats: Arc<Mutex<HashMap<String, AppStats>>>,

    // Communication
    tx: Option<mpsc::Sender<LogLine>>,
    rx: Option<mpsc::Receiver<LogLine>>,
    status_rx: Option<mpsc::Receiver<(String, Vec<ServiceStatus>)>>,
    stats_tx: mpsc::Sender<ProjectStats>,
    stats_rx: mpsc::Receiver<ProjectStats>,

    // Runtime
    start_time: Instant,

    // Actions pending
    pending_launch: Option<String>,
    pending_monitor: Option<String>,

    // État du lancement
    launching: bool,

    // Tokei stats
    show_stats: Option<ProjectStats>,
    loading_stats: bool,

    // Logo texture
    logo_texture: Option<egui::TextureHandle>,
}

#[derive(Clone)]
struct ServiceStatus {
    name: String,
    up: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct TokeiLanguage {
    blanks: u64,
    code: u64,
    comments: u64,
}

#[derive(Debug, Clone)]
struct ProjectStats {
    project_name: String,
    languages: Vec<(String, TokeiLanguage)>,
    total_lines: u64,
    total_code: u64,
    total_files: usize,
}

impl Default for NexusApp {
    fn default() -> Self {
        let logger = Logger::new("System", "NexusApp");
        logger.info("Initialisation de NexusApp...");

        let config = EcosystemsConfig::load().unwrap_or_else(|e| {
            let logger = Logger::new("System", "Config");
            logger.error(&format!("Erreur chargement config: {}", e));
            EcosystemsConfig { ecosystem: vec![] }
        });

        let logger = Logger::new("System", "Config");
        logger.info(&format!("Config chargée: {} écosystèmes", config.ecosystem.len()));

        let (stats_tx, stats_rx) = mpsc::channel(10);

        Self {
            config,
            discovered_projects: vec![],
            selected_ecosystem: None,
            selected_environment: "dev".to_string(),  // Par défaut en dev
            selected_tab: 0,
            services_status: Arc::new(Mutex::new(HashMap::new())),
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            max_logs: 1000,
            stats: Arc::new(Mutex::new(HashMap::new())),
            tx: None,
            rx: None,
            status_rx: None,
            start_time: Instant::now(),
            pending_launch: None,
            pending_monitor: None,
            launching: false,
            show_stats: None,
            loading_stats: false,
            stats_tx,
            stats_rx,
            logo_texture: None,
        }
    }
}

impl NexusApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let logger = Logger::new("System", "NexusApp");
        logger.info("Création de NexusApp...");

        let mut app = Self::default();

        // Charger le logo
        app.logo_texture = Self::load_logo(&cc.egui_ctx);

        // Scanner les projets dans /home/antonin/app/projects/Hydra-ecosystem
        let parent_dir = std::path::PathBuf::from("/home/antonin/app/projects/Hydra-ecosystem");

        let logger = Logger::new("System", "ProjectScanner");
        logger.info(&format!("Scan du répertoire: {}", parent_dir.display()));

        let scanner = ProjectScanner::new(parent_dir);
        let mut projects = scanner.scan().unwrap_or_else(|e| {
            let logger = Logger::new("System", "ProjectScanner");
            logger.error(&format!("Erreur scan projets: {}", e));
            vec![]
        });

        let logger = Logger::new("System", "ProjectScanner");
        logger.info(&format!("Projets découverts: {}", projects.len()));

        // Marquer les projets configurés
        let configured_names = app.config.ecosystem_names();
        let logger = Logger::new("System", "Config");
        logger.info(&format!("Noms configurés: {:?}", configured_names));

        ProjectScanner::mark_configured(&mut projects, &configured_names);

        // Log des projets et leur status configuré
        for project in &projects {
            let logger = Logger::new("System", "ProjectScanner");
            logger.info(&format!("Projet '{}' - configuré: {}", project.name, project.is_configured));
        }

        app.discovered_projects = projects;

        // Sélectionner automatiquement hydra si disponible
        if app.config.get_ecosystem("hydra").is_some() {
            app.selected_ecosystem = Some("hydra".to_string());
        }

        let (tx, rx) = mpsc::channel(100);
        let (status_tx, status_rx) = mpsc::channel(10);

        app.tx = Some(tx);
        app.rx = Some(rx);
        app.status_rx = Some(status_rx);

        let config = app.config.clone();

        // Démarre le runtime Tokio en background
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Boucle de monitoring
                loop {
                    for ecosystem in &config.ecosystem {
                        let mut statuses = Vec::new();

                        for service in ecosystem.all_services() {
                            let up = check_service_health(service).await;
                            statuses.push(ServiceStatus {
                                name: service.name.clone(),
                                up,
                            });
                        }

                        let _ = status_tx.send((ecosystem.name.clone(), statuses)).await;
                    }

                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
        });

        app
    }

    fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let logo_bytes = include_bytes!("../assets/hydra-logo.png");

        match image::load_from_memory(logo_bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();

                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    rgba.as_raw(),
                );

                Some(ctx.load_texture(
                    "hydra-logo",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ))
            }
            Err(e) => {
                let logger = Logger::new("System", "Logo");
                logger.error(&format!("Impossible de charger le logo: {}", e));
                None
            }
        }
    }

    fn load_tokei_stats(&mut self, project_path: &std::path::PathBuf) {
        let path = project_path.clone();
        let project_name = path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        self.loading_stats = true;

        let stats_tx = self.stats_tx.clone();

        std::thread::spawn(move || {
            use std::sync::mpsc;
            use std::time::Duration;

            let (tx_result, rx_result) = mpsc::channel();
            let path_clone = path.clone();

            // Thread pour exécuter tokei
            std::thread::spawn(move || {
                let output = std::process::Command::new("tokei")
                    .arg(path_clone.to_str().unwrap())
                    .arg("--output")
                    .arg("json")
                    .output();
                let _ = tx_result.send(output);
            });

            // Attendre max 10 secondes
            let output_result = rx_result.recv_timeout(Duration::from_secs(10));

            match output_result {
                Ok(Ok(output)) if output.status.success() => {
                    let json_str = String::from_utf8_lossy(&output.stdout);

                    if let Ok(data) = serde_json::from_str::<HashMap<String, TokeiLanguage>>(&json_str) {
                        let mut languages: Vec<(String, TokeiLanguage)> = data
                            .into_iter()
                            .filter(|(k, _)| k != "Total")
                            .collect();

                        languages.sort_by(|a, b| b.1.code.cmp(&a.1.code));

                        let total_lines: u64 = languages.iter().map(|(_, l)| l.code + l.comments + l.blanks).sum();
                        let total_code: u64 = languages.iter().map(|(_, l)| l.code).sum();

                        let stats = ProjectStats {
                            project_name: project_name.clone(),
                            languages,
                            total_lines,
                            total_code,
                            total_files: 0,
                        };

                        let logger = Logger::new("System", "Tokei");
                        logger.info(&format!("Stats loaded for {}: {} lines", project_name, total_lines));
                        let _ = stats_tx.send(stats);
                    } else {
                        let logger = Logger::new("System", "Tokei");
                        logger.error("Failed to parse tokei JSON");
                    }
                }
                Ok(Err(e)) => {
                    let logger = Logger::new("System", "Tokei");
                    logger.error(&format!("Failed to run tokei: {}", e));
                }
                Err(_) => {
                    let logger = Logger::new("System", "Tokei");
                    logger.warn("Tokei timeout (>10s), opération annulée");
                }
                _ => {
                    let logger = Logger::new("System", "Tokei");
                    logger.error("Tokei command failed");
                }
            }
        });
    }

    fn start_ecosystem(&mut self, ecosystem_name: &str) {
        eprintln!("🔍 start_ecosystem() appelé pour: {}", ecosystem_name);
        if let Some(ecosystem) = self.config.get_ecosystem(ecosystem_name) {
            eprintln!("✅ Écosystème trouvé: {:?}", ecosystem.name);
            let ecosystem_clone = ecosystem.clone();
            let tx = self.tx.as_ref().unwrap().clone();
            let env = self.selected_environment.clone();
            eprintln!("🧵 Spawning thread pour lancement...");

            std::thread::spawn(move || {
                eprintln!("🚀 THREAD DÉMARRÉ pour {}", ecosystem_clone.name);
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    // Démarre Docker
                    let logger = Logger::new(&ecosystem_clone.name, "Docker");
                    logger.info(&format!("Démarrage Docker ({})...", env));
                    let _ = start_docker(&ecosystem_clone, &env).await;

                    // Attendre que Docker soit prêt (3 secondes)
                    tokio::time::sleep(Duration::from_secs(3)).await;

                    // Cloner les services pour éviter les lifetime issues
                    let app_services: Vec<Service> = ecosystem_clone.app_services()
                        .into_iter()
                        .cloned()
                        .collect();

                    // Démarre les services applicatifs
                    let logger = Logger::new(&ecosystem_clone.name, "Services");
                    logger.info("Démarrage des services applicatifs...");
                    for service in app_services {
                        if let Some(cmd) = &service.command {
                            let cwd = service.working_directory(&ecosystem_clone.path);
                            let tx_clone = tx.clone();
                            let ecosystem_name = ecosystem_clone.name.clone();
                            let service_name = service.name.clone();
                            let cmd_vec = cmd.clone();

                            let logger = Logger::new(&ecosystem_name, &service_name);
                            let cmd_display = cmd_vec.join(" ");
                            logger.info(&format!("Démarrage: {} (cwd: {})", cmd_display, cwd));

                            tokio::spawn(async move {
                                // Construire la commande complète pour bash
                                let full_cmd = format!("{} {}", cmd_vec[0], cmd_vec[1..].join(" "));

                                match spawn_process(
                                    "/bin/bash",
                                    &["-c".to_string(), full_cmd],
                                    &cwd,
                                    &ecosystem_name,
                                    &service_name,
                                    tx_clone
                                ).await {
                                    Ok(mut child) => {
                                        let logger = Logger::new(&ecosystem_name, &service_name);
                                        logger.info(&format!("{} démarré (PID: {:?})", service_name, child.id()));
                                        let _ = child.wait().await;
                                    }
                                    Err(e) => {
                                        let logger = Logger::new(&ecosystem_name, &service_name);
                                        logger.error(&format!("Erreur lancement {}: {}", service_name, e));
                                    }
                                }
                            });
                        }
                    }

                    // Surveiller le démarrage et ouvrir le navigateur
                    if ecosystem_clone.auto_open_browser {
                        let env_clone = env.clone();
                        tokio::spawn(async move {
                            wait_for_ready_and_open_browser(&ecosystem_clone, &env_clone).await;
                        });
                    }
                });
            });
        }
    }
}

impl eframe::App for NexusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Recevoir les logs
        if let Some(rx) = &mut self.rx {
            while let Ok(log) = rx.try_recv() {
                let mut logs = self.logs.lock().unwrap();
                if logs.len() >= self.max_logs {
                    logs.pop_front();
                }
                logs.push_back(log);
            }
        }

        // Recevoir les statuts
        if let Some(status_rx) = &mut self.status_rx {
            while let Ok((ecosystem_name, statuses)) = status_rx.try_recv() {
                let mut services_status = self.services_status.lock().unwrap();
                services_status.insert(ecosystem_name, statuses);
            }
        }

        // Recevoir les stats tokei
        while let Ok(stats) = self.stats_rx.try_recv() {
            self.loading_stats = false;
            self.show_stats = Some(stats);
        }

        // Header
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Logo animé avec pulsation
                if let Some(logo) = &self.logo_texture {
                    let time = self.start_time.elapsed().as_secs_f32();
                    let pulse = (time * 2.0).sin() * 0.1 + 1.0; // Oscille entre 0.9 et 1.1
                    let logo_size = 32.0 * pulse;

                    ui.add(egui::Image::new(egui::ImageSource::Texture(egui::load::SizedTexture::new(
                        logo.id(),
                        egui::vec2(logo_size, logo_size),
                    ))));
                }

                ui.heading(RichText::new("HYDRA NEXUS").color(Color32::from_rgb(0, 217, 255)).strong());
                ui.label(RichText::new("Multi-Ecosystem Manager").color(Color32::GRAY));

                ui.separator();

                ui.selectable_value(&mut self.selected_tab, 0, "🌍 Projects");
                ui.selectable_value(&mut self.selected_tab, 1, "📊 Services");
                ui.selectable_value(&mut self.selected_tab, 2, "📜 Logs");

                ui.separator();

                // Sélecteur d'environnement
                ui.label("Environment:");
                let mut env_changed = false;
                env_changed |= ui.selectable_value(&mut self.selected_environment, "dev".to_string(), "🔧 Dev").clicked();
                env_changed |= ui.selectable_value(&mut self.selected_environment, "prod".to_string(), "🚀 Prod").clicked();

                if env_changed {
                    let logger = Logger::new("System", "GUI");
                    logger.info(&format!("Environnement changé: {}", self.selected_environment));
                }
            });
        });

        // Contenu principal
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.selected_tab {
                0 => self.render_projects(ui),
                1 => self.render_services(ui),
                2 => self.render_logs(ui),
                _ => {}
            }
        });

        // Footer avec copyright et technologies - AGRANDI
        egui::TopBottomPanel::bottom("footer")
            .min_height(40.0)
            .show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);

                // Technologies (gauche) - TEXTE PLUS GRAND
                ui.label(RichText::new("Built with").color(Color32::LIGHT_GRAY));
                ui.label(RichText::new("🦀").size(18.0));
                ui.hyperlink_to(
                    RichText::new("Rust").color(Color32::from_rgb(222, 165, 132)).strong(),
                    "https://www.rust-lang.org/"
                );
                ui.label(RichText::new("•").color(Color32::GRAY));
                ui.label(RichText::new("🎨").size(18.0));
                ui.hyperlink_to(
                    RichText::new("egui").color(Color32::from_rgb(139, 233, 253)).strong(),
                    "https://www.egui.rs/"
                );
                ui.label(RichText::new("•").color(Color32::GRAY));
                ui.label(RichText::new("🐳").size(18.0));
                ui.hyperlink_to(
                    RichText::new("Docker").color(Color32::from_rgb(33, 150, 243)).strong(),
                    "https://www.docker.com/"
                );

                // Copyright (droite) - TEXTE PLUS GRAND
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new("antonin.niv@gmail.com").color(Color32::LIGHT_GRAY).italics());

                    ui.separator();

                    ui.hyperlink_to(
                        RichText::new("Antonin Nvh").color(Color32::WHITE).strong(),
                        "https://olive.click"
                    );

                    ui.label(RichText::new("by").color(Color32::GRAY));

                    ui.label(RichText::new("❤️").size(18.0).color(Color32::from_rgb(236, 72, 153)));

                    ui.label(RichText::new("Made with").color(Color32::LIGHT_GRAY));
                });
            });
            ui.add_space(8.0);
        });

        // Traiter les actions pending APRÈS le rendu
        if let Some(ecosystem_name) = self.pending_launch.take() {
            eprintln!("🔥 PENDING_LAUNCH DÉTECTÉ: {}", ecosystem_name);
            if !self.launching {
                eprintln!("🚀 LANCEMENT EN COURS...");
                let logger = Logger::new(&ecosystem_name, "Launcher");
                logger.info(&format!("Lancement de {}...", ecosystem_name));
                self.launching = true;
                self.selected_ecosystem = Some(ecosystem_name.clone());
                eprintln!("📞 Appel start_ecosystem()...");
                self.start_ecosystem(&ecosystem_name);
                eprintln!("✅ start_ecosystem() terminé");
                self.selected_tab = 1; // Switch to Services tab

                // Réinitialiser après 2 secondes
                let launching_ref = std::sync::Arc::new(std::sync::Mutex::new(false));
                let launching_clone = launching_ref.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    *launching_clone.lock().unwrap() = false;
                });
            } else {
                let logger = Logger::new("System", "Launcher");
                logger.warn("Lancement déjà en cours, ignoré");
            }
        }

        if let Some(ecosystem_name) = self.pending_monitor.take() {
            let logger = Logger::new(&ecosystem_name, "Monitor");
            logger.info(&format!("Monitoring de {}...", ecosystem_name));
            self.selected_ecosystem = Some(ecosystem_name);
            self.selected_tab = 1;
        }

        // Popup Stats Tokei
        let mut close_stats = false;
        if let Some(stats) = &self.show_stats {
            let stats = stats.clone();
            egui::Window::new(format!("📊 {} - Code Statistics", stats.project_name))
                .collapsible(false)
                .resizable(true)
                .default_width(500.0)
                .show(ctx, |ui| {
                    ui.heading(format!("Total: {} lines ({} code)", stats.total_lines, stats.total_code));
                    ui.add_space(10.0);

                    // Tableau des langages
                    egui::Grid::new("tokei_grid")
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label(RichText::new("Language").strong());
                            ui.label(RichText::new("Files").strong());
                            ui.label(RichText::new("Code").strong());
                            ui.label(RichText::new("Comments").strong());
                            ui.label(RichText::new("Blanks").strong());
                            ui.end_row();

                            for (lang, data) in &stats.languages {
                                ui.label(lang);
                                ui.label("-");
                                ui.label(format!("{}", data.code));
                                ui.label(format!("{}", data.comments));
                                ui.label(format!("{}", data.blanks));
                                ui.end_row();
                            }
                        });

                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        close_stats = true;
                    }
                });
        }

        if close_stats {
            self.show_stats = None;
        }

        if self.loading_stats {
            egui::Window::new("Loading...")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.spinner();
                    ui.label("Analyzing code with tokei...");
                });
        }

        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl NexusApp {
    fn render_projects(&mut self, ui: &mut egui::Ui) {
        ui.heading("Discovered Projects");
        ui.add_space(10.0);

        // Clone les projets pour éviter le borrow checker
        let projects = self.discovered_projects.clone();
        let selected_ecosystem = self.selected_ecosystem.clone();

        ScrollArea::vertical().show(ui, |ui| {
            for project in &projects {
                let is_selected = selected_ecosystem.as_ref() == Some(&project.name);

                let bg_color = if project.is_configured {
                    if is_selected {
                        Color32::from_rgb(0, 100, 150)
                    } else {
                        Color32::from_rgb(40, 40, 45)
                    }
                } else {
                    Color32::from_rgb(30, 30, 30)
                };

                let text_color = if project.is_configured {
                    Color32::WHITE
                } else {
                    Color32::DARK_GRAY
                };

                let project_name = project.name.clone();
                let is_configured = project.is_configured;

                egui::Frame::default()
                    .fill(bg_color)
                    .inner_margin(10.0)
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(project.icon()).size(24.0));
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&project.name).size(18.0).color(text_color).strong());
                                ui.label(RichText::new(project.project_type()).size(12.0).color(Color32::GRAY));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // Bouton Stats (toujours disponible)
                                if ui.button("📊 Stats").clicked() {
                                    let logger = Logger::new("System", "UI");
                                    logger.info(&format!("Stats button clicked for {}", project_name));
                                    self.load_tokei_stats(&project.path);
                                }

                                if is_configured {
                                    if ui.button("▶ Launch").clicked() {
                                        let logger = Logger::new("System", "UI");
                                        logger.info(&format!("Launch button clicked for {}", project_name));
                                        self.pending_launch = Some(project_name.clone());
                                    }

                                    if ui.button("👁 Monitor").clicked() {
                                        self.pending_monitor = Some(project_name.clone());
                                    }
                                } else {
                                    ui.label(RichText::new("Not configured").color(Color32::DARK_GRAY).italics());
                                }
                            });
                        });
                    });

                ui.add_space(5.0);
            }
        });
    }

    fn render_services(&mut self, ui: &mut egui::Ui) {
        if let Some(eco_name) = &self.selected_ecosystem {
            ui.heading(format!("Services - {}", eco_name));

            let statuses = self.services_status.lock().unwrap();
            if let Some(services) = statuses.get(eco_name) {
                ui.add_space(10.0);

                for svc in services {
                    ui.horizontal(|ui| {
                        let color = if svc.up { Color32::GREEN } else { Color32::RED };
                        let icon = if svc.up { "●" } else { "○" };

                        ui.label(RichText::new(icon).size(16.0).color(color));
                        ui.label(&svc.name);
                    });
                }
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No ecosystem selected").color(Color32::GRAY));
            });
        }
    }

    fn render_logs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Logs Stream");
        ui.add_space(10.0);

        ScrollArea::vertical()
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
                        ui.label(RichText::new(&log.ecosystem).color(Color32::from_rgb(0, 217, 255)).strong());
                        ui.label(RichText::new(&log.service).color(Color32::LIGHT_BLUE));
                        ui.label(RichText::new(&log.message).color(color));
                    });
                }
            });
    }
}

async fn check_service_health(service: &Service) -> bool {
    if service.is_http_check() {
        http_health_check(service.port, service.http_health_path()).await
    } else {
        tcp_health_check(service.port).await
    }
}

async fn tcp_health_check(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

async fn http_health_check(port: u16, path: &str) -> bool {
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

async fn start_docker(ecosystem: &Ecosystem, env: &str) -> Result<()> {
    let compose_file = ecosystem.get_docker_compose_for_env(env);

    let logger = Logger::new(&ecosystem.name, "Docker");
    logger.info(&format!("Docker Compose: {}", compose_file));

    Command::new("docker-compose")
        .args(["-f", &compose_file, "up", "-d"])
        .current_dir(&ecosystem.path)
        .output()
        .await?;

    Ok(())
}

async fn wait_for_ready_and_open_browser(ecosystem: &Ecosystem, env: &str) {
    let logger = Logger::new(&ecosystem.name, "Browser");
    logger.info(&format!("Attente du démarrage complet de {} ({})...", ecosystem.name, env));

    let max_attempts = 60; // 60 * 5 = 5 minutes max
    let mut attempt = 0;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        attempt += 1;

        // Vérifie tous les services
        let mut all_up = true;
        for service in ecosystem.all_services() {
            if !check_service_health(service).await {
                all_up = false;
                break;
            }
        }

        if all_up {
            let logger = Logger::new(&ecosystem.name, "Browser");
            logger.info(&format!("{} est prêt ! Ouverture du navigateur...", ecosystem.name));

            // Ouvrir Chrome avec le profil
            if let Err(e) = open_browser(ecosystem, env).await {
                let logger = Logger::new(&ecosystem.name, "Browser");
                logger.error(&format!("Erreur ouverture navigateur: {}", e));
            }
            break;
        }

        if attempt >= max_attempts {
            let logger = Logger::new(&ecosystem.name, "Browser");
            logger.warn(&format!("Timeout: {} n'est pas complètement démarré après 5 minutes", ecosystem.name));
            break;
        }

        let logger = Logger::new(&ecosystem.name, "Browser");
        logger.debug(&format!("Tentative {}/{} - En attente...", attempt, max_attempts));
    }
}

async fn open_browser(ecosystem: &Ecosystem, env: &str) -> Result<()> {
    let url = ecosystem.get_browser_url(env)
        .ok_or_else(|| anyhow::anyhow!("Aucune URL configurée pour l'environnement {}", env))?;

    let mut cmd = Command::new("google-chrome");

    if let Some(profile) = &ecosystem.browser_profile {
        cmd.arg(format!("--profile-directory={}", profile));
    }

    cmd.arg(&url);

    let output = cmd.spawn();

    match output {
        Ok(_) => {
            let logger = Logger::new(&ecosystem.name, "Browser");
            logger.info(&format!("Chrome ouvert sur {}", url));
            Ok(())
        }
        Err(e) => {
            let logger = Logger::new(&ecosystem.name, "Browser");
            logger.error(&format!("Erreur lancement Chrome: {}", e));
            // Fallback: essayer xdg-open
            Command::new("xdg-open")
                .arg(url)
                .spawn()
                .map(|_| ())
                .map_err(|e| anyhow::anyhow!("Impossible d'ouvrir le navigateur: {}", e))
        }
    }
}

async fn spawn_process(
    cmd: &str,
    args: &[String],
    cwd: &str,
    ecosystem: &str,
    service: &str,
    tx: mpsc::Sender<LogLine>,
) -> Result<Child> {
    let mut child = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let ecosystem_name = ecosystem.to_string();
    let service_name = service.to_string();
    let tx_out = tx.clone();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_out.send(LogLine {
                ecosystem: ecosystem_name.clone(),
                service: service_name.clone(),
                message: line,
                level: LogLevel::Info,
                timestamp: Local::now().format("%H:%M:%S").to_string(),
            }).await;
        }
    });

    let ecosystem_name = ecosystem.to_string();
    let service_name = service.to_string();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let level = if line.contains("ERROR") || line.contains("Error") {
                LogLevel::Error
            } else if line.contains("WARN") {
                LogLevel::Warn
            } else {
                LogLevel::Debug
            };

            let _ = tx.send(LogLine {
                ecosystem: ecosystem_name.clone(),
                service: service_name.clone(),
                message: line,
                level,
                timestamp: Local::now().format("%H:%M:%S").to_string(),
            }).await;
        }
    });

    Ok(child)
}

fn load_icon() -> Option<IconData> {
    let icon_bytes = include_bytes!("../src-tauri/icons/128x128.png");

    match image::load_from_memory(icon_bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();

            Some(IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            })
        }
        Err(e) => {
            let logger = Logger::new("System", "Icon");
            logger.warn(&format!("Impossible de charger l'icône: {}", e));
            None
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let main_logger = Logger::new("System", "Nexus");
    main_logger.info("Démarrage du GUI Hydra Nexus...");

    // Charger l'icône
    let icon = load_icon();

    let mut viewport = egui::ViewportBuilder::default()
        .with_min_inner_size([900.0, 600.0])
        .with_max_inner_size([1920.0, 1080.0])
        .with_title("HYDRA NEXUS - Multi-Ecosystem Manager")
        .with_resizable(true);

    if let Some(icon_data) = icon {
        viewport = viewport.with_icon(icon_data);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let window_logger = Logger::new("System", "Window");
    window_logger.info("Création de la fenêtre...");

    let result = eframe::run_native(
        "HYDRA Nexus",
        options,
        Box::new(|cc| {
            let init_logger = Logger::new("System", "Window");
            init_logger.info("Context créé, initialisation de l'app...");
            Ok(Box::new(NexusApp::new(cc)))
        }),
    );

    match &result {
        Ok(_) => {
            let exit_logger = Logger::new("System", "GUI");
            exit_logger.info("GUI terminé normalement");
        }
        Err(e) => {
            let exit_logger = Logger::new("System", "GUI");
            exit_logger.error(&format!("Erreur GUI: {}", e));
        }
    }

    result
}
