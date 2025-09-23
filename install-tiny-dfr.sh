#!/bin/bash
# install-tiny-dfr.sh
# Installation script for tiny-dfr on T2 MacBooks

set -e

echo "Installing tiny-dfr for T2 MacBook..."

# Check if running on T2 Mac
if ! grep -q "MacBookPro16,1\|MacBookAir" /sys/class/dmi/id/product_name 2>/dev/null; then
    echo "Warning: This doesn't appear to be a T2 MacBook"
    read -p "Continue anyway? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Check if running as root
if [[ $EUID -eq 0 ]]; then
    echo "This script should not be run as root. Please run as a regular user."
    exit 1
fi

# Install dependencies based on distro
if command -v pacman &> /dev/null; then
    echo "Detected Arch-based system"
    sudo pacman -S --noconfirm rust cargo cairo libinput freetype2 fontconfig librsvg
elif command -v apt &> /dev/null; then
    echo "Detected Debian-based system"
    sudo apt update
    sudo apt install -y build-essential rustc cargo libcairo2-dev libinput-dev libfreetype6-dev libfontconfig1-dev librsvg2-dev
elif command -v dnf &> /dev/null; then
    echo "Detected Fedora-based system"
    sudo dnf install -y rust cargo cairo-devel libinput-devel freetype-devel fontconfig-devel librsvg2-devel
else
    echo "Unsupported distribution. Please install dependencies manually."
    echo "Required packages: rust, cargo, cairo, libinput, freetype, fontconfig, librsvg"
    exit 1
fi

# Build from source
echo "Building tiny-dfr..."
cargo build --release

# Install
echo "Installing tiny-dfr..."
# Stop service if running to avoid "Text file busy" error
sudo systemctl stop tiny-dfr 2>/dev/null || true
sudo cp target/release/tiny-dfr /usr/bin/
sudo mkdir -p /usr/share/tiny-dfr
sudo cp share/tiny-dfr/* /usr/share/tiny-dfr/
sudo cp etc/systemd/system/tiny-dfr.service /etc/systemd/system/

# Setup systemd service
sudo systemctl daemon-reload
sudo systemctl enable tiny-dfr

# Detect user environment for proper configuration
echo "Detecting user environment..."
CURRENT_USER=$(whoami)
USER_HOME="/home/$CURRENT_USER"
USER_UID=$(id -u $CURRENT_USER)
RUNTIME_DIR="/run/user/$USER_UID"

# Detect Wayland display
WAYLAND_DISPLAY_VALUE="wayland-1"  # default
if [ -d "$RUNTIME_DIR" ]; then
    for socket in "$RUNTIME_DIR"/wayland-*; do
        if [ -S "$socket" ] && [[ ! "$socket" == *.lock ]]; then
            WAYLAND_DISPLAY_VALUE=$(basename "$socket")
            break
        fi
    done
fi

# Detect user's actual PATH locations
USER_PATHS=""
for path_candidate in \
    "$USER_HOME/.local/share/omarchy/bin" \
    "$USER_HOME/.local/bin" \
    "$USER_HOME/.config/nvm/versions/node/latest/bin" \
    "$USER_HOME/.local/share/pnpm" \
    "$USER_HOME/.cargo/bin" \
    "$USER_HOME/.npm-global/bin" \
    "$USER_HOME/bin"; do
    if [ -d "$path_candidate" ]; then
        USER_PATHS="$USER_PATHS:$path_candidate"
    fi
done

echo "Detected user: $CURRENT_USER"
echo "Detected UID: $USER_UID"
echo "Detected Wayland display: $WAYLAND_DISPLAY_VALUE"
echo "Detected user paths: $USER_PATHS"

# Copy config and commands for customization
sudo mkdir -p /etc/tiny-dfr
sudo cp /usr/share/tiny-dfr/config.toml /etc/tiny-dfr/config.toml
sudo cp /usr/share/tiny-dfr/commands.toml /etc/tiny-dfr/commands.toml

# Create user-specific environment configuration
sudo tee /etc/tiny-dfr/user-env.toml > /dev/null <<EOF
# Auto-generated user environment configuration
[user_environment]
username = "$CURRENT_USER"
uid = $USER_UID
home_dir = "$USER_HOME"
runtime_dir = "$RUNTIME_DIR"
wayland_display = "$WAYLAND_DISPLAY_VALUE"
user_paths = "$USER_PATHS"
EOF

# Set MediaLayerDefault = true
echo "Setting MediaLayerDefault = true in config..."
sudo sed -i 's/MediaLayerDefault = false/MediaLayerDefault = true/' /etc/tiny-dfr/config.toml

# Restart the service to apply config changes
echo "Restarting tiny-dfr service..."
sudo systemctl restart tiny-dfr

echo ""
echo "Installation complete!"
echo "Edit /etc/tiny-dfr/config.toml to customize your Touch Bar"
echo "Service is now running!"
echo ""
echo "To check status: sudo systemctl status tiny-dfr"
echo "To view logs: sudo journalctl -u tiny-dfr -f"
echo "To restart: sudo systemctl restart tiny-dfr"
