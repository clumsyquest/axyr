# Axyr
Le « DevTools / F12 du microcontrôleur » : une couche open-source qui capte l'état interne réel d'un MCU en temps réel et le rend lisible — pour le développeur et pour les agents IA (via MCP).

## Structure
- `firmware/`  — code embarqué (C, Zephyr) qui tourne sur la puce
- `engine/`    — moteur hôte + serveur MCP (Rust)
- `dashboard/` — interface web (TypeScript)
- `docs/`      — documentation et notes de conception

## Statut
🚧 Pré-v1 — première cible : l'« explicateur de crash » pour STM32 (Cortex-M) sur Zephyr.
