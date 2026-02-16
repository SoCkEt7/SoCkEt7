use anyhow::{Result, Context};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use serde::{Serialize, Deserialize};

pub struct Svc {
    pub name: &'static str,
    pub port: u16,
    pub icon: &'static str,
    pub http: Option<&'static str>,
}

pub static DOCKER_SVCS: &[Svc] = &[
    Svc { name: "Redis",      port: 31379, icon: "🔴", http: None },
    Svc { name: "Qdrant",     port: 31333, icon: "🟣", http: Some("/healthz") },
    Svc { name: "Neo4j",      port: 31474, icon: "🟢", http: None },
    Svc { name: "LiteLLM",    port: 31300, icon: "🤖", http: Some("/health") },
    Svc { name: "Prometheus",  port: 31990, icon: "📊", http: Some("/-/healthy") },
    Svc { name: "Grafana",    port: 31900, icon: "📈", http: Some("/api/health") },
];

pub static APP_SVCS: &[Svc] = &[
    Svc { name: "Backend",  port: 31100, icon: "⚙️ ", http: Some("/api/health") },
    Svc { name: "Frontend", port: 31173, icon: "🎨", http: Some("/") },
];

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceStatus {
    pub name: String,
    pub up: bool,
    pub icon: String,
}

pub async fn svc_check(svc: &Svc) -> bool {
    match svc.http {
        Some(p) => http_ok(svc.port, p).await,
        None => tcp_ok(svc.port).await,
    }
}

pub async fn tcp_ok(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

pub async fn http_ok(port: u16, path: &str) -> bool {
    let mut stream = match tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    ).await {
        Ok(Ok(s)) => s,
        _ => return false,
    };

    let req = format!(
        "GET {} HTTP/1.1
Host: 127.0.0.1:{}
Connection: close

",
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

pub async fn docker_up(dir: &str) -> Result<()> {
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

pub async fn docker_down(dir: &str) {
    for f in ["docker-compose.full.yml", "docker-compose.yml"] {
        if std::path::Path::new(&format!("{}/{}", dir, f)).exists() {
            let _ = Command::new("docker-compose")
                .args(["-f", f, "down"])
                .current_dir(dir)
                .output().await;
        }
    }
}

pub async fn cleanup_procs() {
    for pat in ["ts-node.*index-hexa", "tsx.*index-hexa", "node.*dist.*index", "vite.*31173"] {
        let _ = Command::new("pkill").args(["-f", pat]).output().await;
    }
}
