# ttwm

A tabbed tiling window manager for X11, inspired by [Notion](https://notionwm.net/).

**This entire project was written by [Claude Code](https://claude.com/claude-code).**

![ttwm screenshot](docs/screenshot.png)

ttwm combines tiling layouts with tabbed window organization. Multiple windows share the same frame as tabs, and frames can be split to create complex layouts. Tab bars can be horizontal (top of frame) or vertical (left side) for different workflows.

## Features

- **Tabbed frames** - Stack windows as tabs within frames, like browser tabs
- **Horizontal or vertical tab bars** - Choose per-frame tab orientation
- **Flexible tiling** - Split frames horizontally or vertically with adjustable ratios
- **Startup layouts** - Define complex layouts and auto-launch apps in config

## Building

Requires Rust 1.70+, X11, and FreeType development libraries.

```bash
# Debian/Ubuntu
sudo apt install build-essential libx11-dev libxcb1-dev libfreetype6-dev

# Arch
sudo pacman -S base-devel libx11 libxcb freetype2
```

```bash
git clone https://github.com/adereth/ttwm.git
cd ttwm
cargo build --release
```

## Quick Start

```bash
mkdir -p ~/.config/ttwm
cp config.toml.example ~/.config/ttwm/config.toml
```

| Key | Action |
|-----|--------|
| `Mod4+s` | Split horizontally |
| `Mod4+v` | Split vertically |
| `Mod4+/` | Toggle vertical tabs |
| `Mod4+Arrow` | Focus frame in direction |
| `Mod4+Page_Down/Up` | Cycle tabs |
| `Mod4+q` | Close window |

## Documentation

- [User Guide](docs/USER_GUIDE.md) - Configuration and usage
- [Developer Guide](docs/DEVELOPER_GUIDE.md) - Architecture

## License

MIT
