#!/bin/bash

echo "🐉 Lancement du GUI Hydra Nexus v2 (Multi-Ecosystem)..."

if [ ! -f "target/debug/nexus-v2" ]; then
    echo "❌ Binaire non trouvé. Compilez d'abord avec: cargo build --bin nexus-v2"
    exit 1
fi

# Kill l'ancienne instance si elle existe
pkill -f "target/debug/nexus-v2" 2>/dev/null || true
sleep 0.5

# Lance le GUI
DISPLAY=${DISPLAY:-:0} ./target/debug/nexus-v2

echo "✅ GUI fermé"
