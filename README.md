# tiny-dfr

The most basic dynamic function row daemon possible for T2 MacBooks (MacBook Pro 16,1 and other T2 models).

## Overview

tiny-dfr provides customizable Touch Bar functionality on T2 MacBooks running Linux. By default, the Touch Bar works in the same mode as Windows Bootcamp, but tiny-dfr allows you to customize it with your own layouts and functions.

**Inspired by**: [T2 Linux Wiki - Adding support for customisable Touch Bar](https://wiki.t2linux.org/guides/postinstall/#adding-support-for-customisable-touch-bar)

## Prerequisites

### System Requirements
- T2 MacBook (MacBook Pro 16,1, MacBook Air, etc.)
- Linux with T2 kernel support
- Required kernel modules: `apple-bce`, `hid-appletb-kbd`, `hid-appletb-bl`

### Dependencies
- cairo
- libinput
- freetype
- fontconfig
- librsvg 2.59 or later
- uinput enabled in kernel config

## Installation

### Quick Install (Recommended)

Run the automated installation script:

```bash
git clone https://github.com/your-repo/tiny-dfr.git
cd tiny-dfr
chmod +x install-tiny-dfr.sh
./install-tiny-dfr.sh
```

The script will:
- Detect your Linux distribution and install dependencies
- Build tiny-dfr from source
- Install and configure the systemd service
- Set up the default configuration
- Start the service automatically

## Configuration

### Basic Setup
1. Copy the default config:
```bash
sudo cp /usr/share/tiny-dfr/config.toml /etc/tiny-dfr/config.toml
```

2. Edit the configuration:
```bash
sudo nano /etc/tiny-dfr/config.toml
```

### Configuration Options
The config file allows you to customize:
- Touch Bar layout and buttons
- Icon sets and themes
- Key bindings and functions
- Display settings

See the config file comments for detailed options.

### Restart Service After Config Changes
```bash
sudo systemctl restart tiny-dfr
```


## Troubleshooting

### Touch Bar Not Working
1. Ensure T2 kernel modules are loaded:
```bash
lsmod | grep -E "(apple-bce|hid-appletb)"
```

2. Check if uinput is enabled:
```bash
lsmod | grep uinput
```

3. Verify service status:
```bash
sudo systemctl status tiny-dfr
```

### Service Issues
```bash
# View logs
sudo journalctl -u tiny-dfr -f

# Restart service
sudo systemctl restart tiny-dfr

# Check config syntax
sudo tiny-dfr --check-config
```

## License

tiny-dfr is licensed under the MIT license, as included in the [LICENSE](LICENSE) file.

* Copyright The Asahi Linux Contributors

Please see the Git history for authorship information.

tiny-dfr embeds Google's [material-design-icons](https://github.com/google/material-design-icons)
which are licensed under [Apache License Version 2.0](LICENSE.material)
Some icons are derivatives of material-icons, with edits made by kekrby.
