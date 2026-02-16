mod ecosystem_config;
mod project_scanner;

use ecosystem_config::EcosystemsConfig;

fn main() {
    println!("🔍 Test de chargement de la configuration...");

    match EcosystemsConfig::load() {
        Ok(config) => {
            println!("✅ Configuration chargée avec succès");
            println!("📊 Nombre d'écosystèmes: {}", config.ecosystem.len());

            for eco in &config.ecosystem {
                println!("\n🌍 Écosystème: {}", eco.name);
                println!("   📁 Path: {}", eco.path);
                println!("   🎨 Icon: {}", eco.icon);
                println!("   🌐 Auto-open: {}", eco.auto_open_browser);
                println!("   📦 Services: {}", eco.services.len());

                if !eco.environments.is_empty() {
                    println!("   🔧 Environnements:");
                    for (name, env) in &eco.environments {
                        println!("      - {}: {}", name, env.browser_url);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("❌ Erreur de chargement: {}", e);
            eprintln!("Cause: {:?}", e);
            std::process::exit(1);
        }
    }

    println!("\n🔍 Test du scanner de projets...");
    let scanner = project_scanner::ProjectScanner::new("../");
    match scanner.scan() {
        Ok(projects) => {
            println!("✅ Scan réussi - {} projets trouvés", projects.len());
            for p in &projects {
                println!("   {} {} ({})", p.icon(), p.name, p.project_type());
            }
        }
        Err(e) => {
            eprintln!("❌ Erreur de scan: {}", e);
        }
    }
}
