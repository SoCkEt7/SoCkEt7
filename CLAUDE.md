# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

<!-- rtk-instructions v2 -->
# RTK (Rust Token Killer) - Token-Optimized Commands

## Golden Rule

**Always prefix commands with `rtk`**. If RTK has a dedicated filter, it uses it. If not, it passes through unchanged. This means RTK is always safe to use.

**Important**: Even in command chains with `&&`, use `rtk`:
```bash
# ❌ Wrong
git add . && git commit -m "msg" && git push

# ✅ Correct
rtk git add . && rtk git commit -m "msg" && rtk git push
```

## RTK Commands by Workflow

### Build & Compile (80-90% savings)
```bash
rtk cargo build         # Cargo build output
rtk cargo check         # Cargo check output
rtk cargo clippy        # Clippy warnings grouped by file (80%)
rtk tsc                 # TypeScript errors grouped by file/code (83%)
rtk lint                # ESLint/Biome violations grouped (84%)
rtk prettier --check    # Files needing format only (70%)
rtk next build          # Next.js build with route metrics (87%)
```

### Test (90-99% savings)
```bash
rtk cargo test          # Cargo test failures only (90%)
rtk vitest run          # Vitest failures only (99.5%)
rtk playwright test     # Playwright failures only (94%)
rtk test <cmd>          # Generic test wrapper - failures only
```

### Git (59-80% savings)
```bash
rtk git status          # Compact status
rtk git log             # Compact log (works with all git flags)
rtk git diff            # Compact diff (80%)
rtk git show            # Compact show (80%)
rtk git add             # Ultra-compact confirmations (59%)
rtk git commit          # Ultra-compact confirmations (59%)
rtk git push            # Ultra-compact confirmations
rtk git pull            # Ultra-compact confirmations
rtk git branch          # Compact branch list
rtk git fetch           # Compact fetch
rtk git stash           # Compact stash
rtk git worktree        # Compact worktree
```

Note: Git passthrough works for ALL subcommands, even those not explicitly listed.

### GitHub (26-87% savings)
```bash
rtk gh pr view <num>    # Compact PR view (87%)
rtk gh pr checks        # Compact PR checks (79%)
rtk gh run list         # Compact workflow runs (82%)
rtk gh issue list       # Compact issue list (80%)
rtk gh api              # Compact API responses (26%)
```

### JavaScript/TypeScript Tooling (70-90% savings)
```bash
rtk pnpm list           # Compact dependency tree (70%)
rtk pnpm outdated       # Compact outdated packages (80%)
rtk pnpm install        # Compact install output (90%)
rtk npm run <script>    # Compact npm script output
rtk npx <cmd>           # Compact npx command output
rtk prisma              # Prisma without ASCII art (88%)
```

### Files & Search (60-75% savings)
```bash
rtk ls <path>           # Tree format, compact (65%)
rtk read <file>         # Code reading with filtering (60%)
rtk grep <pattern>      # Search grouped by file (75%)
rtk find <pattern>      # Find grouped by directory (70%)
```

### Analysis & Debug (70-90% savings)
```bash
rtk err <cmd>           # Filter errors only from any command
rtk log <file>          # Deduplicated logs with counts
rtk json <file>         # JSON structure without values
rtk deps                # Dependency overview
rtk env                 # Environment variables compact
rtk summary <cmd>       # Smart summary of command output
rtk diff                # Ultra-compact diffs
```

### Infrastructure (85% savings)
```bash
rtk docker ps           # Compact container list
rtk docker images       # Compact image list
rtk docker logs <c>     # Deduplicated logs
rtk kubectl get         # Compact resource list
rtk kubectl logs        # Deduplicated pod logs
```

### Network (65-70% savings)
```bash
rtk curl <url>          # Compact HTTP responses (70%)
rtk wget <url>          # Compact download output (65%)
```

### Meta Commands
```bash
rtk gain                # View token savings statistics
rtk gain --history      # View command history with savings
rtk discover            # Analyze Claude Code sessions for missed RTK usage
rtk proxy <cmd>         # Run command without filtering (for debugging)
rtk init                # Add RTK instructions to CLAUDE.md
rtk init --global       # Add RTK to ~/.claude/CLAUDE.md
```

## Token Savings Overview

| Category | Commands | Typical Savings |
|----------|----------|-----------------|
| Tests | vitest, playwright, cargo test | 90-99% |
| Build | next, tsc, lint, prettier | 70-87% |
| Git | status, log, diff, add, commit | 59-80% |
| GitHub | gh pr, gh run, gh issue | 26-87% |
| Package Managers | pnpm, npm, npx | 70-90% |
| Files | ls, read, grep, find | 60-75% |
| Infrastructure | docker, kubectl | 85% |
| Network | curl, wget | 65-70% |

Overall average: **60-90% token reduction** on common development operations.
<!-- /rtk-instructions -->

---

# Project Overview

**Hydra Nexus** is a control center for the Hydra ecosystem, providing three interfaces to monitor and manage Docker services and applications:

1. **TUI** (Terminal UI) - `nexus-tui` using ratatui
2. **Native GUI** - `nexus` using egui/eframe
3. **Web GUI** - Tauri-based web application (in development)

## Architecture

### Core Components

- **`src/main.rs`** - TUI implementation with ratatui
- **`src/gui.rs`** - Native GUI with egui/eframe
- **`src-tauri/`** - Tauri web application backend
  - **`src-tauri/src/ecosystem.rs`** - Shared service management logic (health checks, Docker operations)
  - **`src-tauri/src/lib.rs`** - Tauri commands for web GUI

### Monitored Services

All services run on custom ports (31xxx range):

**Docker Services:**
- Redis (31379) - TCP health check
- Qdrant (31333) - HTTP /healthz
- Neo4j (31474) - TCP health check
- LiteLLM (31300) - HTTP /health
- Prometheus (31990) - HTTP /-/healthy
- Grafana (31900) - HTTP /api/health

**Application Services:**
- Backend (31100) - HTTP /api/health
- Frontend (31173) - HTTP /

### Service Health Checks

Two types of health checks implemented:
- **TCP checks** - Connects to port (500ms timeout)
- **HTTP checks** - Raw TCP + HTTP/1.1 request (500ms timeout)

Health checks run every 5 seconds in a background task.

### Log Aggregation

All three interfaces stream and aggregate logs from:
- Docker containers (`docker-compose logs -f`)
- Backend process (pnpm dev in hydra directory)
- Frontend process (pnpm dev in hydra/frontend)

Logs are classified by level (Info/Warn/Error/Debug) and errors are grouped by message for easier debugging.

## Development Commands

### Build

```bash
# Build TUI binary
rtk cargo build --bin nexus-tui

# Build native GUI binary
rtk cargo build --bin nexus

# Build optimized release
rtk cargo build --release

# Build Tauri app
cd src-tauri && rtk cargo build
```

### Run

```bash
# Run TUI
./run-nexus.sh dev          # or: ./target/debug/nexus-tui dev
./run-nexus.sh status
./run-nexus.sh stop

# Run native GUI
./run-gui.sh                # or: ./target/debug/nexus

# Run Tauri (web GUI) - development
rtk pnpm dev

# Run Tauri (web GUI) - production
rtk pnpm tauri build
```

### Lint & Check

```bash
rtk cargo clippy            # Rust linting
rtk cargo check             # Fast type checking
rtk cargo test              # Run tests
```

## Key Implementation Details

### Async Runtime

All interfaces use Tokio for async operations:
- TUI: spawns runtime in `main()`
- Native GUI: spawns runtime in background thread from `NexusApp::new()`
- Tauri: runtime managed by Tauri framework

### Log Streaming

Logs are streamed via `mpsc` channels:
1. Background tasks spawn processes with piped stdout/stderr
2. `BufReader` wraps streams and reads line-by-line
3. Lines sent via channel to UI thread
4. UI processes logs (classification, deduplication, stats)

### Stats Tracking

Real-time metrics calculated from logs:
- Total requests (counting HTTP methods in logs)
- Request throughput (requests/second)
- Error rate (errors/total requests)
- Uptime
- CPU/Memory usage (TUI: parsed from `top`, GUI: simulated)

### Error Grouping

Errors with identical messages are grouped:
- First 500 chars used as key
- Count incremented on duplicates
- Last seen timestamp updated
- HashMap maintains message → index mapping

## Project Layout

```
hydra-nexus/
├── src/
│   ├── main.rs              # TUI (nexus-tui binary)
│   └── gui.rs               # Native GUI (nexus binary)
├── src-tauri/               # Tauri web app
│   ├── src/
│   │   ├── main.rs          # Tauri entry point
│   │   ├── lib.rs           # Tauri commands
│   │   └── ecosystem.rs     # Shared service logic
│   ├── Cargo.toml
│   └── tauri.conf.json      # Tauri configuration
├── Cargo.toml               # Root workspace config
├── run-nexus.sh             # TUI launcher
└── run-gui.sh               # GUI launcher
```

## Hydra Ecosystem Context

Nexus controls the main Hydra ecosystem located at:
- **Backend**: `../hydra/` (pnpm dev)
- **Frontend**: `../hydra/frontend/` (pnpm dev)
- **Docker**: `../hydra/docker-compose.yml` or `docker-compose.full.yml`

The Hydra project directory structure:
```
../hydra/
├── backend/
├── frontend/               # Next.js/React frontend
├── docker-compose.yml
└── docker-compose.full.yml
```

## Technology Stack

**Rust:**
- tokio - Async runtime
- ratatui + crossterm - TUI framework
- egui + eframe - Native GUI framework
- tauri 2.10 - Web app framework
- anyhow - Error handling
- serde/serde_json - Serialization
- chrono - Timestamps
- regex - Log parsing

**Build:**
- cargo - Rust build system
- pnpm - Node package manager (for Tauri frontend)
- docker-compose - Container orchestration

## Notes

- All three interfaces (TUI/GUI/Tauri) share similar architecture but duplicate code
- Consider extracting common logic into a shared crate for better maintainability
- The Tauri app is configured to serve from `../hydra/frontend/dist` in production
- Process cleanup targets specific patterns to avoid killing unrelated processes
