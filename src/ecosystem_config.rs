use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EcosystemsConfig {
    pub ecosystem: Vec<Ecosystem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ecosystem {
    pub name: String,
    pub path: String,
    pub docker_compose: String,
    pub docker_compose_fallback: Option<String>,
    pub icon: String,
    pub color: String,
    #[serde(default)]
    pub auto_open_browser: bool,
    pub browser_profile: Option<String>,
    #[serde(default)]
    pub environments: HashMap<String, Environment>,
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Environment {
    pub browser_url: String,
    pub docker_compose: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Service {
    pub name: String,
    pub port: u16,
    pub icon: String,
    #[serde(rename = "type")]
    pub service_type: ServiceType,
    pub health_check: HealthCheckType,
    pub health_path: Option<String>,
    pub command: Option<Vec<String>>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType {
    Docker,
    App,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HealthCheckType {
    Tcp,
    Http,
}

impl EcosystemsConfig {
    /// Charge la configuration depuis le fichier ecosystems.toml
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Impossible de lire {}", config_path.display()))?;

        let config: EcosystemsConfig = toml::from_str(&content)
            .context("Erreur de parsing du fichier ecosystems.toml")?;

        Ok(config)
    }

    /// Chemin du fichier de configuration
    fn config_path() -> Result<PathBuf> {
        // Cherche d'abord dans le répertoire courant
        let local_path = PathBuf::from("ecosystems.toml");
        if local_path.exists() {
            return Ok(local_path);
        }

        // Puis dans le répertoire du binaire
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let exe_config = dir.join("ecosystems.toml");
                if exe_config.exists() {
                    return Ok(exe_config);
                }
            }
        }

        // Par défaut, retourne le chemin local (sera créé si nécessaire)
        Ok(local_path)
    }

    /// Retourne l'écosystème par nom
    pub fn get_ecosystem(&self, name: &str) -> Option<&Ecosystem> {
        self.ecosystem.iter().find(|e| e.name == name)
    }

    /// Liste les noms de tous les écosystèmes
    pub fn ecosystem_names(&self) -> Vec<String> {
        self.ecosystem.iter().map(|e| e.name.clone()).collect()
    }
}

impl Ecosystem {
    /// Retourne le chemin du fichier docker-compose à utiliser
    pub fn docker_compose_file(&self) -> String {
        let primary = format!("{}/{}", self.path, self.docker_compose);
        if std::path::Path::new(&primary).exists() {
            return self.docker_compose.clone();
        }

        if let Some(fallback) = &self.docker_compose_fallback {
            let fallback_path = format!("{}/{}", self.path, fallback);
            if std::path::Path::new(&fallback_path).exists() {
                return fallback.clone();
            }
        }

        // Par défaut, retourne le fichier principal même s'il n'existe pas
        self.docker_compose.clone()
    }

    /// Services Docker uniquement
    pub fn docker_services(&self) -> Vec<&Service> {
        self.services
            .iter()
            .filter(|s| s.service_type == ServiceType::Docker)
            .collect()
    }

    /// Services applicatifs uniquement
    pub fn app_services(&self) -> Vec<&Service> {
        self.services
            .iter()
            .filter(|s| s.service_type == ServiceType::App)
            .collect()
    }

    /// Tous les services
    pub fn all_services(&self) -> &[Service] {
        &self.services
    }

    /// Retourne l'URL du navigateur pour un environnement donné
    pub fn get_browser_url(&self, env: &str) -> Option<String> {
        self.environments.get(env).map(|e| e.browser_url.clone())
    }

    /// Retourne le fichier docker-compose pour un environnement donné
    pub fn get_docker_compose_for_env(&self, env: &str) -> String {
        self.environments
            .get(env)
            .and_then(|e| e.docker_compose.clone())
            .unwrap_or_else(|| self.docker_compose_file())
    }

    /// Vérifie si un environnement existe
    pub fn has_environment(&self, env: &str) -> bool {
        self.environments.contains_key(env)
    }
}

impl Service {
    /// Retourne le chemin de travail absolu pour le service
    pub fn working_directory(&self, ecosystem_path: &str) -> String {
        match &self.cwd {
            Some(cwd) if cwd == "." => ecosystem_path.to_string(),
            Some(cwd) => format!("{}/{}", ecosystem_path, cwd),
            None => ecosystem_path.to_string(),
        }
    }

    /// Vérifie si le service a un health check HTTP
    pub fn is_http_check(&self) -> bool {
        self.health_check == HealthCheckType::Http
    }

    /// Retourne le path HTTP pour le health check (ou "/" par défaut)
    pub fn http_health_path(&self) -> &str {
        self.health_path.as_deref().unwrap_or("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let config = EcosystemsConfig::load();
        assert!(config.is_ok(), "Config should load successfully");

        let config = config.unwrap();
        assert!(!config.ecosystem.is_empty(), "Should have at least one ecosystem");

        // Vérifie Hydra
        let hydra = config.get_ecosystem("Hydra");
        assert!(hydra.is_some(), "Hydra ecosystem should exist");

        let hydra = hydra.unwrap();
        assert_eq!(hydra.icon, "🐉");
        assert!(!hydra.services.is_empty(), "Hydra should have services");
    }
}
