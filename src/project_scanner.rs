use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DiscoveredProject {
    pub name: String,
    pub path: PathBuf,
    pub has_docker_compose: bool,
    pub has_package_json: bool,
    pub has_cargo_toml: bool,
    pub is_configured: bool,
}

pub struct ProjectScanner {
    scan_dir: PathBuf,
}

impl ProjectScanner {
    pub fn new(scan_dir: impl Into<PathBuf>) -> Self {
        Self {
            scan_dir: scan_dir.into(),
        }
    }

    /// Scanne le répertoire parent pour détecter tous les projets
    pub fn scan(&self) -> Result<Vec<DiscoveredProject>> {
        let mut projects = Vec::new();

        if !self.scan_dir.exists() || !self.scan_dir.is_dir() {
            return Ok(projects);
        }

        for entry in std::fs::read_dir(&self.scan_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Ignore les fichiers et répertoires cachés
            if let Some(name) = path.file_name() {
                let name_str = name.to_string_lossy();
                if name_str.starts_with('.') {
                    continue;
                }

                // Ignore certains répertoires connus
                if matches!(
                    name_str.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".git"
                ) {
                    continue;
                }
            }

            // Ne garde que les répertoires
            if !path.is_dir() {
                continue;
            }

            // Détecte les indicateurs de projet
            let has_docker_compose = self.has_docker_compose(&path);
            let has_package_json = path.join("package.json").exists();
            let has_cargo_toml = path.join("Cargo.toml").exists();

            // Si c'est un projet potentiel (au moins un indicateur)
            if has_docker_compose || has_package_json || has_cargo_toml {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                projects.push(DiscoveredProject {
                    name,
                    path,
                    has_docker_compose,
                    has_package_json,
                    has_cargo_toml,
                    is_configured: false, // Sera mis à jour après
                });
            }
        }

        // Trie par nom
        projects.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(projects)
    }

    /// Vérifie si un répertoire contient un fichier docker-compose
    fn has_docker_compose(&self, dir: &PathBuf) -> bool {
        let patterns = [
            "docker-compose.yml",
            "docker-compose.yaml",
            "docker-compose.full.yml",
            "docker-compose.prod.yml",
        ];

        patterns
            .iter()
            .any(|pattern| dir.join(pattern).exists())
    }

    /// Marque les projets configurés dans ecosystems.toml
    pub fn mark_configured(
        projects: &mut [DiscoveredProject],
        configured_names: &[String],
    ) {
        for project in projects.iter_mut() {
            // Matching case-insensitive pour éviter les problèmes de casse
            project.is_configured = configured_names.iter().any(|name| {
                name.eq_ignore_ascii_case(&project.name)
            });
        }
    }
}

impl DiscoveredProject {
    /// Retourne une description lisible du type de projet
    pub fn project_type(&self) -> String {
        let mut types = Vec::new();

        if self.has_docker_compose {
            types.push("Docker");
        }
        if self.has_package_json {
            types.push("Node.js");
        }
        if self.has_cargo_toml {
            types.push("Rust");
        }

        if types.is_empty() {
            "Unknown".to_string()
        } else {
            types.join(" + ")
        }
    }

    /// Icône basée sur le type de projet
    pub fn icon(&self) -> &'static str {
        if self.has_docker_compose && self.has_package_json {
            "🐳"
        } else if self.has_cargo_toml {
            "🦀"
        } else if self.has_package_json {
            "📦"
        } else if self.has_docker_compose {
            "🐋"
        } else {
            "📁"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_projects() {
        let scanner = ProjectScanner::new("../");
        let projects = scanner.scan();

        assert!(projects.is_ok(), "Scan should succeed");

        let projects = projects.unwrap();
        println!("Discovered {} projects:", projects.len());
        for p in &projects {
            println!("  - {} [{}] ({})", p.name, p.icon(), p.project_type());
        }

        assert!(!projects.is_empty(), "Should discover at least one project");
    }
}
