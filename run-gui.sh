#!/bin/bash
# Lancer le GUI Nexus

echo "🐉 Lancement du GUI Hydra Nexus..."

# Vérifier que nous sommes dans le bon répertoire
if [ ! -f "target/debug/nexus" ]; then
    echo "❌ Binaire non trouvé. Compilez d'abord avec: cargo build --bin nexus"
    exit 1
fi

# Tuer les anciennes instances
pkill -f "target/debug/nexus" 2>/dev/null || true
sleep 1

# Lancer le GUI
DISPLAY=${DISPLAY:-:0} ./target/debug/nexus

echo "✅ GUI fermé"
