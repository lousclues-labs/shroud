#!/usr/bin/env bash
# setup.sh - Native Arch Linux setup/update script for openvpn-tray
# This script is idempotent and can be run multiple times safely.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BINARY_NAME="openvpn-tray"
INSTALL_DIR="$HOME/.local/bin"
SERVICE_DIR="$HOME/.config/systemd/user"
SERVICE_NAME="openvpn-tray.service"

# Flags
SKIP_GIT_PULL=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-pull)
            SKIP_GIT_PULL=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --skip-pull    Skip git pull (useful for local development)"
            echo "  -h, --help     Show this help message"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            exit 1
            ;;
    esac
done

# Helper functions
info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

check_dependency() {
    local cmd=$1
    local package=$2
    local install_cmd=$3
    
    if command -v "$cmd" &> /dev/null; then
        success "$cmd is installed"
        return 0
    else
        error "$cmd is not installed"
        echo "  Install with: $install_cmd"
        return 1
    fi
}

# Main script
echo ""
echo -e "${BLUE}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║           OpenVPN Tray - Arch Linux Setup                ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════╝${NC}"
echo ""

# Step 1: Check dependencies
info "Checking dependencies..."
echo ""

DEPS_OK=true

if ! check_dependency "cargo" "rust" "sudo pacman -S rust"; then
    DEPS_OK=false
fi

if ! check_dependency "openvpn" "openvpn" "sudo pacman -S openvpn"; then
    DEPS_OK=false
fi

if ! check_dependency "nmcli" "networkmanager" "sudo pacman -S networkmanager"; then
    DEPS_OK=false
fi

if ! check_dependency "pkexec" "polkit" "sudo pacman -S polkit"; then
    DEPS_OK=false
fi

echo ""

if [ "$DEPS_OK" = false ]; then
    error "Missing dependencies. Please install them and run this script again."
    exit 1
fi

success "All dependencies are installed"
echo ""

# Step 2: Git pull (if in a git repo and not skipped)
if [ "$SKIP_GIT_PULL" = false ]; then
    if [ -d ".git" ]; then
        info "Updating repository with git pull --ff-only..."
        if git pull --ff-only; then
            success "Repository updated"
        else
            warn "git pull failed - you may need to resolve conflicts manually"
            warn "Continuing with current code..."
        fi
    else
        info "Not a git repository, skipping git pull"
    fi
else
    info "Skipping git pull (--skip-pull flag set)"
fi
echo ""

# Step 3: Build release
info "Building release binary..."
if cargo build --release; then
    success "Build completed successfully"
else
    error "Build failed"
    exit 1
fi
echo ""

# Step 4: Stop running app before installing binary
WAS_SERVICE_RUNNING=false
WAS_PROCESS_RUNNING=false

if systemctl --user is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
    info "Stopping running $SERVICE_NAME service..."
    if systemctl --user stop "$SERVICE_NAME"; then
        WAS_SERVICE_RUNNING=true
        success "Service stopped"
    else
        warn "Could not stop service"
    fi
elif pgrep -x "$BINARY_NAME" > /dev/null 2>&1; then
    info "Stopping running $BINARY_NAME process..."
    if pkill -TERM -x "$BINARY_NAME"; then
        WAS_PROCESS_RUNNING=true
        # Wait briefly for process to exit
        sleep 1
        if pgrep -x "$BINARY_NAME" > /dev/null 2>&1; then
            warn "Process did not exit cleanly, forcing kill"
            pkill -KILL -x "$BINARY_NAME"
            sleep 1
        fi
        success "Process stopped"
    else
        warn "Could not stop process"
    fi
else
    info "No running instance detected"
fi
echo ""

# Step 5: Install binary
info "Installing binary to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
cp "target/release/$BINARY_NAME" "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/$BINARY_NAME"
success "Binary installed to $INSTALL_DIR/$BINARY_NAME"
echo ""

# Step 6: Install systemd user service
info "Installing systemd user service..."
mkdir -p "$SERVICE_DIR"
cp "systemd/$SERVICE_NAME" "$SERVICE_DIR/"
success "Service file installed to $SERVICE_DIR/$SERVICE_NAME"
echo ""

# Step 7: Reload systemd and enable service
info "Reloading systemd user daemon..."
systemctl --user daemon-reload
success "Systemd daemon reloaded"

info "Enabling service..."
# Enable may show "Created symlink" or warn if already enabled - both are OK
if ! systemctl --user enable "$SERVICE_NAME" 2>&1 | grep -qE "^Failed"; then
    : # Enable succeeded or already enabled
else
    warn "Could not enable service"
fi

# Restart service if it was running, or if we stopped a standalone process
if [ "$WAS_SERVICE_RUNNING" = true ] || [ "$WAS_PROCESS_RUNNING" = true ]; then
    info "Restarting service..."
    if systemctl --user restart "$SERVICE_NAME" 2>&1 | grep -qE "^Failed"; then
        warn "Could not restart service (this is normal if no graphical session is active)"
    else
        success "Service restarted successfully"
    fi
else
    info "Service not started (was not previously running)"
fi
echo ""

# Step 8: Check PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    warn "$INSTALL_DIR is not in your PATH"
    echo "  Add this to your ~/.bashrc or ~/.zshrc:"
    echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
fi

# Step 9: Print next steps
echo -e "${GREEN}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║                    Setup Complete!                       ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Next steps:"
echo ""
echo "  1. Configure OpenVPN files (requires root):"
echo "     ${YELLOW}sudo mkdir -p /etc/openvpn${NC}"
echo "     ${YELLOW}sudo cp your-config.ovpn /etc/openvpn/${NC}"
echo ""
echo "  2. Create authentication file:"
echo "     ${YELLOW}sudo nano /etc/openvpn/auth.txt${NC}"
echo "     Add username on first line, password on second line"
echo "     ${YELLOW}sudo chmod 600 /etc/openvpn/auth.txt${NC}"
echo ""
echo "  3. Check service status:"
echo "     ${YELLOW}systemctl --user status $SERVICE_NAME${NC}"
echo ""
echo "  4. View logs:"
echo "     ${YELLOW}journalctl --user -u $SERVICE_NAME -f${NC}"
echo ""
echo "  5. For debug logging:"
echo "     ${YELLOW}RUST_LOG=debug $INSTALL_DIR/$BINARY_NAME${NC}"
echo ""
echo "After updating (git pull), simply run this script again to rebuild and restart."
echo ""
