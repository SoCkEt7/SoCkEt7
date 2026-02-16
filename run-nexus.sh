#!/bin/bash
# Hydra Nexus - Centre de contrôle natif

case "${1:-dev}" in
    dev)
        echo "🐉 Lancement du centre de contrôle Hydra..."
        ./target/debug/nexus dev
        ;;
    status)
        ./target/debug/nexus status
        ;;
    stop)
        ./target/debug/nexus stop
        ;;
    *)
        echo "Usage: $0 {dev|status|stop}"
        exit 1
        ;;
esac
