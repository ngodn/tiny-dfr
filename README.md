# Enhanced tiny-dfr

A Macbook Touch Bar daemon with enhanced features including Hyprland integration, expandable menus, and keyboard backlight control.

## tiny-dfr with Enhanced Features

- **Hyprland Integration**: Context-aware buttons that change based on active window/application
- **Expandable Menus**: Multi-level navigation with customizable button groups
- **Custom Commands**: Define unlimited custom commands and actions

## Notes for T2
- Required kernel modules for T2: `apple-bce`, `hid-appletb-kbd`, `hid-appletb-bl`

## Installation

Use the provided installation script:

```bash
./install-tiny-dfr.sh
```

The script will:
- Install required dependencies
- Build from source
- Configure systemd service
- Set up user environment
- Apply default configuration



## Configuration

Configuration files are located in `/etc/tiny-dfr/`:

### Main Configuration (`config.toml`)
See [share/tiny-dfr/config.toml](share/tiny-dfr/config.toml) for examples


### Commands (`commands.toml`)
Define custom commands:
- **Command_[Name]**: Named commands

See [share/tiny-dfr/commands.toml](share/tiny-dfr/commands.toml) for examples


### Expandable Menus (`expandables.toml`)
Multi-level menu configurations:
- **Expand_[Name]**: Named Expandables

See [share/tiny-dfr/expandables.toml](share/tiny-dfr/expandables.toml) for examples


### Hyprland Integration (`hyprland.toml`)
Application-specific button layouts:
- **Class-based configurations**: Different buttons per application
- **Dynamic context switching**: Buttons change based on active window

See [share/tiny-dfr/hyprland.toml](share/tiny-dfr/hyprland.toml) for examples

## Keyboard Backlight Support

The daemon supports keyboard backlight control on the following device paths:
1. `/sys/class/leds/:white:kbd_backlight`
2. `/sys/class/leds/smc::kbd_backlight`
3. **Generic detection**: Any device in `/sys/class/leds/` containing "kbd" or "keyboard"

The system automatically detects available keyboard backlight devices and provides hardware-level brightness control with configurable step sizes.

Check what keyboard backlight devices exist on your system:
```bash
ls -la /sys/class/leds/ | grep -i kbd
# or
find /sys/class/leds/ -name "*kbd*" -o -name "*keyboard*"
```