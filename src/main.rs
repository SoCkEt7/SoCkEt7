// HYDRA Unified Stack Launcher & Dashboard
use anyhow::{Context, Result};
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use arboard::Clipboard;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Gauge, Sparkline, ListState, Clear},
    Frame, Terminal,
};
use std::{
    collections::{VecDeque, HashMap},
    io,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

// ANSI constants for legacy non-TUI output
const C: &str = "\x1b[36m";
const B: &str = "\x1b[1m";
const D: &str = "\x1b[2m";
const X: &str = "\x1b[0m";

// Service definitions
struct Svc {
    name: &'static str,
    port: u16,
    icon: &'static str,
    http: Option<&'static str>,
}

static DOCKER_SVCS: &[Svc] = &[
    Svc { name: "Redis",      port: 31379, icon: "🔴", http: None },
    Svc { name: "Qdrant",     port: 31333, icon: "🟣", http: Some("/healthz") },
    Svc { name: "Neo4j",      port: 31474, icon: "🟢", http: None },
    Svc { name: "LiteLLM",    port: 31300, icon: "🤖", http: Some("/health") },
    Svc { name: "Prometheus",  port: 31990, icon: "📊", http: Some("/-/healthy") },
    Svc { name: "Grafana",    port: 31900, icon: "📈", http: Some("/api/health") },
];

static APP_SVCS: &[Svc] = &[
    Svc { name: "Backend",  port: 31100, icon: "⚙️ ", http: Some("/api/health") },
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
    last_error: Option<String>,
    req_per_sec: f64,
    error_rate: f64,
    history: VecDeque<u64>,
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

struct App {
    logs: VecDeque<LogLine>,
    max_logs: usize,
    docker_status: Vec<(String, bool, f64)>,
    app_status: Vec<(String, bool, f64)>,
    stats: AppStats,
    grouped_errors: Vec<GroupedError>,
    error_map: HashMap<String, usize>,
    selected_tab: usize,
    should_quit: bool,
    last_tick: Instant,
    tick_requests: u64,
    log_state: ListState,
    error_state: ListState,
    show_details: bool,
}

impl App {
    fn new() -> Self {
        let mut log_state = ListState::default();
        log_state.select(Some(0));
        let mut error_state = ListState::default();
        error_state.select(Some(0));

        Self {
            logs: VecDeque::with_capacity(1000),
            max_logs: 1000,
            docker_status: vec![],
            app_status: vec![],
            stats: AppStats {
                total_requests: 0,
                errors_count: 0,
                uptime: Duration::from_secs(0),
                last_error: None,
                req_per_sec: 0.0,
                error_rate: 0.0,
                history: VecDeque::from(vec![0; 60]),
                cpu_usage: 0.0,
                mem_usage: 0.0,
            },
            grouped_errors: vec![],
            error_map: HashMap::new(),
            selected_tab: 0,
            should_quit: false,
            last_tick: Instant::now(),
            tick_requests: 0,
            log_state,
            error_state,
            show_details: false,
        }
    }

    fn add_log(&mut self, service: &str, line: &str) {
        // Advanced JS Error Detection
        let is_error = line.contains("ERROR") || 
                      line.contains("Error:") || 
                      line.contains("fail") || 
                      line.contains("TypeError") ||
                      line.contains("ReferenceError") ||
                      line.contains("at ") && line.contains(":") || // Stack trace hint
                      line.contains("uncaught");

        let level = if is_error {
            self.stats.errors_count += 1;
            // Collapse newlines for the grouped view but keep them for details if needed
            let msg = line.replace('\n', " ").chars().take(500).collect::<String>();
            self.stats.last_error = Some(format!("{}: {}", service, msg));
            
            if let Some(&idx) = self.error_map.get(&msg) {
                self.grouped_errors[idx].count += 1;
                self.grouped_errors[idx].last_seen = Local::now().format("%H:%M:%S").to_string();
            } else {
                let idx = self.grouped_errors.len();
                self.grouped_errors.push(GroupedError {
                    message: msg.clone(),
                    count: 1,
                    service: service.to_string(),
                    last_seen: Local::now().format("%H:%M:%S").to_string(),
                });
                self.error_map.insert(msg, idx);
            }
            LogLevel::Error
        } else if line.contains("WARN") || line.contains("Warning") {
            LogLevel::Warn
        } else if line.contains("DEBUG") {
            LogLevel::Debug
        } else {
            LogLevel::Info
        };

        if line.contains("GET ") || line.contains("POST ") || line.contains("HTTP") || line.contains("PATCH") || line.contains("DELETE") {
            self.stats.total_requests += 1;
            self.tick_requests += 1;
        }

        let log = LogLine {
            service: service.to_string(),
            message: line.to_string(),
            level,
            timestamp: Local::now().format("%H:%M:%S").to_string(),
        };

        if self.logs.len() >= self.max_logs {
            self.logs.pop_front();
        }
        self.logs.push_back(log);
    }

    fn update_stats(&mut self) {
        let elapsed = self.last_tick.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.stats.req_per_sec = self.tick_requests as f64 / elapsed;
            self.stats.history.pop_front();
            self.stats.history.push_back(self.tick_requests);
            
            if self.stats.total_requests > 0 {
                self.stats.error_rate = (self.stats.errors_count as f64 / self.stats.total_requests as f64) * 100.0;
            }

            // System Metrics (Simple)
            if let Ok(output) = std::process::Command::new("top").args(["-bn1"]).output() {
                let s = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = s.lines().find(|l| l.contains("%Cpu(s)")) {
                    if let Some(idle) = line.split(',').find(|p| p.contains("id")) {
                        if let Some(val) = idle.trim().split_whitespace().next() {
                            if let Ok(idle_val) = val.parse::<f64>() {
                                self.stats.cpu_usage = 100.0 - idle_val;
                            }
                        }
                    }
                }
            }

            if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
                let total = mem.lines().find(|l| l.starts_with("MemTotal")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
                let free = mem.lines().find(|l| l.starts_with("MemAvailable")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                self.stats.mem_usage = ((total - free) / total) * 100.0;
            }

            self.tick_requests = 0;
            self.last_tick = Instant::now();
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "dev".into());
    let dir = std::env::current_dir()?.to_string_lossy().to_string();

    match mode.as_str() {
        "dev" | "full" => run_tui(&dir).await?,
        "stop" => { stop_all(&dir).await?; }
        "status" => status().await?,
        _ => usage(),
    }
    Ok(())
}

fn usage() {
    eprintln!("\n  {B}🐉 HYDRA Unified Stack Launcher{X}\n");
    eprintln!("  {B}Usage:{X} hydra <command>\n");
    eprintln!("  {C}dev{X}      Dashboard TUI + Services {D}(défaut){X}");
    eprintln!("  {C}stop{X}     Arrêter tous les services");
    eprintln!("  {C}status{X}   État des services\n");
}

async fn run_tui(dir: &str) -> Result<()> {
    // Cleanup first
    stop_all(dir).await?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // App state
    let mut app = App::new();
    let start_time = Instant::now();

    // Log channel
    let (tx, mut rx) = mpsc::channel::<(String, String)>(100);

    // Start services
    let logs_dir = format!("{}/.hydra-logs", dir);
    std::fs::create_dir_all(&logs_dir)?;

    docker_up(dir).await?;
    
    let mut be = spawn_proc_tui("pnpm", &["dev"], dir, "Backend", tx.clone()).await?;
    let mut fe = spawn_proc_tui("pnpm", &["dev"], &format!("{}/frontend", dir), "Frontend", tx.clone()).await?;

    // Log tailing for docker
    let tx_docker = tx.clone();
    tokio::spawn(async move {
        let mut child = Command::new("docker-compose")
            .args(["logs", "-f", "--tail", "10"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_docker.send(("Docker".to_string(), line)).await;
        }
    });

    // Health check loop
    let (status_tx, mut status_rx) = mpsc::channel::<Vec<St>>(10);
    tokio::spawn(async move {
        loop {
            let mut all_svcs = vec![];
            for svc in DOCKER_SVCS.iter().chain(APP_SVCS.iter()) {
                let up = svc_check(svc).await;
                all_svcs.push(St {
                    name: svc.name.into(),
                    up,
                    ms: 0.0, // simplified for TUI
                });
            }
            let _ = status_tx.send(all_svcs).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // Main loop
    loop {
        app.stats.uptime = start_time.elapsed();
        app.update_stats();

        terminal.draw(|f| ui(f, &app))?;

                if event::poll(Duration::from_millis(50))? {
                    match event::read()? {
                        Event::Key(key) => {
                            match key.code {
                                KeyCode::Char('q') => break,
                                KeyCode::Tab => app.selected_tab = (app.selected_tab + 1) % 3,
                                KeyCode::Char('1') => app.selected_tab = 0,
                                KeyCode::Char('2') => app.selected_tab = 1,
                                KeyCode::Char('3') => app.selected_tab = 2,
                                KeyCode::Down => {
                                    if app.selected_tab == 0 {
                                        let i = match app.log_state.selected() {
                                            Some(i) => if i + 1 < app.logs.len() { i + 1 } else { i },
                                            None => 0,
                                        };
                                        app.log_state.select(Some(i));
                                    } else if app.selected_tab == 2 {
                                        let i = match app.error_state.selected() {
                                            Some(i) => if i + 1 < app.grouped_errors.len() { i + 1 } else { i },
                                            None => 0,
                                        };
                                        app.error_state.select(Some(i));
                                    }
                                }
                                KeyCode::Up => {
                                    if app.selected_tab == 0 {
                                        let i = match app.log_state.selected() {
                                            Some(i) => if i > 0 { i - 1 } else { 0 },
                                            None => 0,
                                        };
                                        app.log_state.select(Some(i));
                                    } else if app.selected_tab == 2 {
                                        let i = match app.error_state.selected() {
                                            Some(i) => if i > 0 { i - 1 } else { 0 },
                                            None => 0,
                                        };
                                        app.error_state.select(Some(i));
                                    }
                                }
                                KeyCode::Enter => {
                                    if app.selected_tab == 2 {
                                        app.show_details = !app.show_details;
                                    }
                                }
                                KeyCode::Char('y') => {
                                    let text_to_copy = if app.selected_tab == 0 {
                                        app.log_state.selected().and_then(|i| app.logs.get(app.logs.len() - 1 - i)).map(|l| l.message.clone())
                                    } else if app.selected_tab == 2 {
                                        app.error_state.selected().and_then(|i| app.grouped_errors.get(i)).map(|e| e.message.clone())
                                    } else {
                                        None
                                    };
        
                                    if let Some(text) = text_to_copy {
                                        if let Ok(mut clipboard) = Clipboard::new() {
                                            let _ = clipboard.set_text(text);
                                        }
                                    }
                                }
                                KeyCode::Char('o') => {
                                    let _ = Command::new("xdg-open").arg("http://localhost:31173").spawn();
                                }
                                KeyCode::Esc => {
                                    app.show_details = false;
                                }
                                KeyCode::Char('c') => {
                                    app.logs.clear();
                                    app.grouped_errors.clear();
                                    app.error_map.clear();
                                    app.stats.errors_count = 0;
                                    app.stats.total_requests = 0;
                                }
                                _ => {}
                            }
                        }
                        Event::Mouse(mouse_event) => {
                            if mouse_event.kind == event::MouseEventKind::Down(event::MouseButton::Left) {
                                let x = mouse_event.column;
                                let y = mouse_event.row;
                                
                                // Tab detection (Header is at y=1 roughly)
                                if y >= 0 && y <= 2 {
                                    let width = terminal.size()?.width;
                                    let tab_width = width / 3;
                                    if x < tab_width { app.selected_tab = 0; }
                                    else if x < tab_width * 2 { app.selected_tab = 1; }
                                    else { app.selected_tab = 2; }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // Receive logs
        while let Ok((svc, line)) = rx.try_recv() {
            app.add_log(&svc, &line);
        }

        // Receive status
        if let Ok(st) = status_rx.try_recv() {
            app.docker_status = st.iter().filter(|s| DOCKER_SVCS.iter().any(|d| d.name == s.name))
                .map(|s| (s.name.clone(), s.up, s.ms)).collect();
            app.app_status = st.iter().filter(|s| APP_SVCS.iter().any(|a| a.name == s.name))
                .map(|s| (s.name.clone(), s.up, s.ms)).collect();
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Kill processes
    let _ = be.kill().await;
    let _ = fe.kill().await;
    docker_down(dir).await;

    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    let size = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main
            Constraint::Length(4), // Alert Bar
            Constraint::Length(3), // Footer
        ])
        .split(size);

    // Header
    let titles = vec![" [1] Dashboard ", " [2] Analytics ", " [3] Errors "];
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" 🐉 HYDRA Unified Stack "))
        .select(app.selected_tab)
        .style(Style::default().fg(Color::Cyan))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_widget(tabs, chunks[0]);

    match app.selected_tab {
        0 => {
            // Main Area: Logs & Status
            let main_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(20), // Sidebar: Status
                    Constraint::Percentage(80), // Main: Logs
                ])
                .split(chunks[1]);

            // Status Sidebar
            let status_block = Block::default().title(" Status ").borders(Borders::ALL);
            let mut status_items = vec![];
            
            status_items.push(ListItem::new(Span::styled("DOCKER", Style::default().add_modifier(Modifier::BOLD).fg(Color::Magenta))));
            for (name, up, _) in &app.docker_status {
                let color = if *up { Color::Green } else { Color::Red };
                status_items.push(ListItem::new(Span::styled(format!(" {} {:<10} UP", if *up { "●" } else { "○" }, name), Style::default().fg(color))));
            }
            
            status_items.push(ListItem::new(Span::raw("")));
            status_items.push(ListItem::new(Span::styled("APPS", Style::default().add_modifier(Modifier::BOLD).fg(Color::Blue))));
            for (name, up, _) in &app.app_status {
                let color = if *up { Color::Green } else { Color::Red };
                status_items.push(ListItem::new(Span::styled(format!(" {} {:<10} UP", if *up { "●" } else { "○" }, name), Style::default().fg(color))));
            }

            let status_list = List::new(status_items).block(status_block);
            f.render_widget(status_list, main_chunks[0]);

            // Logs Main
            let log_title = format!(" Streaming Logs ({}) ", app.logs.len());
            let logs_block = Block::default()
                .title(log_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if app.selected_tab == 0 { Color::Yellow } else { Color::White }));
            
            let logs: Vec<ListItem> = app.logs.iter()
                .rev()
                .map(|l| {
                    let color = match l.level {
                        LogLevel::Error => Color::Red,
                        LogLevel::Warn => Color::Yellow,
                        LogLevel::Debug => Color::DarkGray,
                        LogLevel::Info => Color::White,
                    };
                    let content = Line::from(vec![
                        Span::styled(format!("[{}] ", l.timestamp), Style::default().fg(Color::DarkGray)),
                        Span::styled(format!("{:<10} ", l.service), Style::default().fg(Color::Cyan)),
                        Span::styled(&l.message, Style::default().fg(color)),
                    ]);
                    ListItem::new(content)
                })
                .collect();
            
            let mut state = app.log_state.clone();
            f.render_stateful_widget(
                List::new(logs)
                    .block(logs_block)
                    .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
                    .highlight_symbol(">> "),
                main_chunks[1],
                &mut state
            );
        }
        1 => {
            // Analytics Tab - Bento Grid Style
            let analytics_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(10), // Stats Cards
                    Constraint::Min(5),    // Charts
                ])
                .split(chunks[1]);
                
            let stats_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                ])
                .split(analytics_chunks[0]);

            // Card 1: Traffic
            let traffic_text = vec![
                Line::from(vec![Span::styled("Throughput: ", Style::default().fg(Color::Gray)), Span::styled(format!("{:.1} req/s", app.stats.req_per_sec), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))]),
                Line::from(vec![Span::styled("Total Req:  ", Style::default().fg(Color::Gray)), Span::styled(app.stats.total_requests.to_string(), Style::default().fg(Color::White))]),
                Line::from(vec![Span::styled("Uptime:     ", Style::default().fg(Color::Gray)), Span::styled(format!("{}s", app.stats.uptime.as_secs()), Style::default().fg(Color::Cyan))]),
            ];
            f.render_widget(Paragraph::new(traffic_text).block(Block::default().title(" 📡 TRAFFIC ").borders(Borders::ALL)), stats_chunks[0]);

            // Card 2: Reliability
            let rel_text = vec![
                Line::from(vec![Span::styled("Error Rate: ", Style::default().fg(Color::Gray)), Span::styled(format!("{:.2}%", app.stats.error_rate), Style::default().fg(if app.stats.error_rate > 5.0 { Color::Red } else { Color::Green }).add_modifier(Modifier::BOLD))]),
                Line::from(vec![Span::styled("Total Err:  ", Style::default().fg(Color::Gray)), Span::styled(app.stats.errors_count.to_string(), Style::default().fg(Color::Red))]),
            ];
            f.render_widget(Paragraph::new(rel_text).block(Block::default().title(" 🛡️ RELIABILITY ").borders(Borders::ALL)), stats_chunks[1]);

            // Card 3: System
            let sys_text = vec![
                Line::from(vec![Span::styled("CPU Usage:  ", Style::default().fg(Color::Gray)), Span::styled(format!("{:.1}%", app.stats.cpu_usage), Style::default().fg(if app.stats.cpu_usage > 80.0 { Color::Red } else { Color::Green }))]),
                Line::from(vec![Span::styled("MEM Usage:  ", Style::default().fg(Color::Gray)), Span::styled(format!("{:.1}%", app.stats.mem_usage), Style::default().fg(if app.stats.mem_usage > 80.0 { Color::Red } else { Color::Green }))]),
            ];
            f.render_widget(Paragraph::new(sys_text).block(Block::default().title(" 💻 SYSTEM ").borders(Borders::ALL)), stats_chunks[2]);
            
            let chart_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(40), // Sparkline
                    Constraint::Percentage(30), // CPU Gauge
                    Constraint::Percentage(30), // MEM Gauge
                ])
                .split(analytics_chunks[1]);

            let sparkline = Sparkline::default()
                .block(Block::default().title(" Request History (Last 60s) ").borders(Borders::ALL))
                .data(&app.stats.history.as_slices().0)
                .style(Style::default().fg(Color::Yellow));
            f.render_widget(sparkline, chart_chunks[0]);
            
            let cpu_gauge = Gauge::default()
                .block(Block::default().title(" CPU Load ").borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Green).bg(Color::Black))
                .percent(app.stats.cpu_usage.min(100.0) as u16);
            f.render_widget(cpu_gauge, chart_chunks[1]);

            let mem_gauge = Gauge::default()
                .block(Block::default().title(" MEM Load ").borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
                .percent(app.stats.mem_usage.min(100.0) as u16);
            f.render_widget(mem_gauge, chart_chunks[2]);
        }
        2 => {
            // Grouped Errors Tab
            let error_list: Vec<ListItem> = app.grouped_errors.iter()
                .map(|e| {
                    let content = Line::from(vec![
                        Span::styled(format!("[x{}] ", e.count), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{:<10} ", e.service), Style::default().fg(Color::Cyan)),
                        Span::styled(&e.message, Style::default().fg(Color::White)),
                    ]);
                    ListItem::new(content)
                })
                .collect();

            let error_block = Block::default()
                .title(format!(" Grouped JS Errors ({}) ", app.grouped_errors.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let mut state = app.error_state.clone();
            f.render_stateful_widget(
                List::new(error_list)
                    .block(error_block)
                    .highlight_style(Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD))
                    .highlight_symbol("-> "),
                chunks[1],
                &mut state
            );

            if app.show_details {
                if let Some(selected) = app.error_state.selected() {
                    if let Some(err) = app.grouped_errors.get(selected) {
                        let area = centered_rect(80, 60, size);
                        f.render_widget(Clear, area); // Clear the area before rendering the popup
                        let details = Paragraph::new(format!(
                            "SERVICE: {}\nCOUNT: {}\nLAST SEEN: {}\n\nFULL MESSAGE:\n{}",
                            err.service, err.count, err.last_seen, err.message
                        ))
                        .block(Block::default().title(" Error Details ").borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)))
                        .wrap(ratatui::widgets::Wrap { trim: true });
                        f.render_widget(details, area);
                    }
                }
            }
        }
        _ => {}
    }

    // Alert Bar
    let last_error = app.stats.last_error.as_deref().unwrap_or("No errors detected");
    let error_style = if app.stats.errors_count > 0 { Style::default().fg(Color::Red) } else { Style::default().fg(Color::Green) };
    let info_bar = Paragraph::new(format!(" LAST ALERT: {}", last_error))
        .style(error_style)
        .block(Block::default().borders(Borders::ALL).title(" Alerts & Monitoring "));
    f.render_widget(info_bar, chunks[2]);

    // Footer
    let footer_text = Line::from(vec![
        Span::styled(" [Q]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)), Span::raw(" Quit | "),
        Span::styled(" [1-3]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::raw(" Tabs | "),
        Span::styled(" [↑↓]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)), Span::raw(" Nav | "),
        Span::styled(" [O]", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)), Span::raw(" Open UI | "),
        Span::styled(" [Y]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)), Span::raw(" Copy | "),
        Span::styled(" [ENTER]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)), Span::raw(" Details | "),
        Span::styled(" [C]", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)), Span::raw(" Clear "),
    ]);
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[3]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

// Re-using and adapting existing helper functions
async fn spawn_proc_tui(cmd: &str, args: &[&str], cwd: &str, label: &str, tx: mpsc::Sender<(String, String)>) -> Result<Child> {
    let mut child = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to start {}", label))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let label_s = label.to_string();
    let tx_out = tx.clone();
    let label_err = label.to_string();
    let tx_err = tx.clone();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_out.send((label_s.clone(), line)).await;
        }
    });

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_err.send((label_err.clone(), line)).await;
        }
    });

    Ok(child)
}

// Original helper functions adapted to be non-blocking or used in TUI
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

async fn docker_down(dir: &str) {
    for f in ["docker-compose.full.yml", "docker-compose.yml"] {
        if std::path::Path::new(&format!("{}/{}", dir, f)).exists() {
            let _ = Command::new("docker-compose")
                .args(["-f", f, "down"])
                .current_dir(dir)
                .output().await;
        }
    }
}

async fn stop_all(dir: &str) -> Result<()> {
    cleanup_procs().await;
    docker_down(dir).await;
    Ok(())
}

async fn cleanup_procs() {
    for pat in ["ts-node.*index-hexa", "tsx.*index-hexa", "node.*dist.*index", "vite.*31173"] {
        let _ = Command::new("pkill").args(["-f", pat]).output().await;
    }
}

#[derive(Clone)]
struct St {
    name: String,
    up: bool,
    ms: f64,
}

async fn status() -> Result<()> {
    for svc in DOCKER_SVCS.iter().chain(APP_SVCS.iter()) {
        let up = svc_check(svc).await;
        println!("{} {:<12} :{} {}", svc.icon, svc.name, svc.port, if up { "✅" } else { "❌" });
    }
    Ok(())
}
