# Enhanced tiny-dfr

The most basic dynamic function row daemon possible for macbook touchbars

## Overview

this enhanced tiny-dfr provides customizable Touch Bar functionality on macbooks running arch linux with Omarchy specifically, thou might support other distros as well. By default, the Touch Bar works in the same mode as Windows Bootcamp, but tiny-dfr allows you to customize it with your own layouts and functions.

### Features
- Customizable Touch Bar layouts and functions
- **Hyprland integration** - Real-time active window display with expandable actions
- **Hardware keyboard backlight control** - Automatically detects and controls keyboard backlight on supported MacBooks
- **Command execution** - Execute custom shell commands and applications from Touch Bar buttons
- **Per-button outline customization** - Individual control over button outline visibility and colors
- **Performance optimized** - Background threading and caching for responsive UI
- Icon themes and customization
- Systemd integration for automatic startup

## Prerequisites

### System Requirements
- Required kernel modules for T2: `apple-bce`, `hid-appletb-kbd`, `hid-appletb-bl`

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
1. Copy the default config and commands:
```bash
sudo cp /usr/share/tiny-dfr/config.toml /etc/tiny-dfr/config.toml
sudo cp /usr/share/tiny-dfr/commands.toml /etc/tiny-dfr/commands.toml
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
- **Per-button outline customization** - Control visibility and colors of button outlines individually
- **Custom button outline colors** - Use grayscale or RGB colors for unique button styling

See the config file comments for detailed options.

### Restart Service After Config Changes
```bash
sudo systemctl restart tiny-dfr
```

## Command Execution

enhanced tiny-dfr supports executing custom shell commands and applications from Touch Bar buttons using the `Command_X` action system.

### Configuration

Commands are loaded with the following priority:
- Base commands from `/usr/share/tiny-dfr/commands.toml`
- User overrides from `/etc/tiny-dfr/commands.toml` (takes precedence)

1. **Define commands** in `/etc/tiny-dfr/commands.toml`:
```toml
# Example commands
Command_1 = "firefox"
Command_2 = "omarchy-menu capture"
Command_3 = "walker -p \"Launch…\""
Command_4 = "alacritty"
Command_5 = "rofi -show drun"
```

2. **Use commands** in `/etc/tiny-dfr/config.toml`:
```toml
MediaLayerKeys = [
    { Icon = "firefox", Action = "Command_1" },
    { Icon = "capture", Action = "Command_2" },
    { Icon = "apps",    Action = "Command_3" },
    # ... other buttons
]
```

### Features

- **Full user environment**: Commands run with your complete shell environment and theming
- **GUI application support**: Automatic display environment setup for Wayland/X11 applications
- **Dynamic user detection**: Works across different users without hardcoded paths
- **Background execution**: Commands run asynchronously without blocking the Touch Bar

**Note**: Command execution has been tested on Arch Linux with Omarchy. Other distributions and desktop environments may require additional configuration.

## Hyprland Integration

Enhanced tiny-dfr provides deep integration with Hyprland window manager, offering real-time active window display and context-aware expandable actions.

### Features

- **Real-time active window tracking** - Displays current window title and class instantly
- **Dynamic app icon detection** - Automatically shows app-specific icons using `app-{class}` pattern
- **Expandable context actions** - Touch Bar buttons can expand to show window-specific actions
- **Multiple display modes** - Text-only, icon-only, or combined text+icon display
- **Performance optimized** - Uses background threading and caching for <5ms response times

### Configuration

#### Basic Hyprland Button

Add a Hyprland plugin button to your Touch Bar configuration:

```toml
MediaLayerKeys = [
    # Text mode - shows window title with optional app icon
    { Text = "plugin-hyprland", Action = "Hyprland_Expand_ActiveWindow",
      ShowButtonOutlines = true, ButtonTitle = "title",
      ShowAppIconAlongsideText = true, Stretch = 8 },

    # Icon mode - shows only app icon
    { Icon = "plugin-hyprland-icon", Action = "Hyprland_Expand_ActiveWindow",
      ShowButtonOutlines = true, Stretch = 8 },
]
```

#### Expandable Actions

Configure window-specific actions in `/etc/tiny-dfr/hyprland.toml`:

```toml
[[Hyprland_Expand_ActiveWindow]]
class = "code"
button_title = "title"
layer_keys = [
    { Icon = "color_picker", Action = "Command_ColorPicker" },
    { Icon = "terminal", Action = "Command_Terminal" },
    { Icon = "screenshot", Action = "Command_Screenshot" },
]

[[Hyprland_Expand_ActiveWindow]]
class = "firefox"
button_title = "title"
layer_keys = [
    { Icon = "screenrecord", Action = "Command_ScreenRecord" },
    { Icon = "screenshot", Action = "Command_Screenshot" },
]
```

### Display Modes

| Mode | Configuration | Description |
|------|---------------|-------------|
| **Text Only** | `Text = "plugin-hyprland"` | Shows window title as text |
| **Text + Icon** | `Text = "plugin-hyprland"` + `ShowAppIconAlongsideText = true` | Shows app icon with window title |
| **Icon Only** | `Icon = "plugin-hyprland-icon"` | Shows only app icon |

### Button Behavior

1. **Main Layer**: Shows current active window information
2. **Tap to Expand**: Button expands to show window-specific actions plus Back button
3. **Back Navigation**: Returns to main layer while preserving button display mode
4. **Real-time Updates**: Instantly updates when switching between applications

### App Icon Detection

The system automatically detects app icons using this priority:
1. **App-specific icon**: `/usr/share/tiny-dfr/app-{class}.svg` (e.g., `app-code.svg`, `app-firefox.svg`)
2. **System theme icon**: Uses freedesktop.org icon themes
3. **Default fallback**: `application-default-icon`
4. **Plugin fallback**: `plugin-hyprland.svg`

### Performance Features

- **Background icon caching** - Icons are pre-loaded and cached for instant display
- **Async icon loading** - Missing icons load in background without blocking UI
- **Optimized window tracking** - Uses Hyprland's real-time event system
- **Response time**: <5ms for window switching and button updates

## Performance Optimizations

Enhanced tiny-dfr includes comprehensive performance optimizations for responsive Touch Bar experience:

### Background Threading Architecture

- **Icon Cache System** - Pre-loads and caches commonly used icons at startup
- **Battery Monitor Thread** - Monitors battery state in background, updates every 30 seconds
- **System Monitor Thread** - Handles time updates and cache cleanup scheduling
- **User Environment Cache** - Pre-detects user environment for instant command execution
- **Hyprland Event Listener** - Real-time window tracking via background socket connection

### Caching Strategy

- **Icon preloading** - Common icons (`back`, `settings`, `application-default-icon`) loaded at startup
- **App icon caching** - Frequently used app icons cached with LRU eviction (5-minute TTL)
- **Path-based caching** - Thread-safe file path caching instead of loaded image objects
- **Fallback chains** - Multi-level fallback with pre-cached alternatives

### Performance Metrics

| Operation | Before Optimization | After Optimization |
|-----------|-------------------|------------------|
| **Command execution** | 50-200ms (blocking) | ~0ms (cached environment) |
| **Icon loading** | 10-50ms per icon | ~1-5ms (cache hit) |
| **Window switching** | 40-200ms | <5ms (real-time events) |
| **Battery updates** | Blocking I/O | Background cached |
| **UI responsiveness** | Can lag during I/O | Consistent <10ms |

### Memory Management

- **Automatic cache cleanup** - Removes unused icons after 5 minutes
- **Bounded cache size** - Prevents unlimited memory growth
- **Efficient data structures** - Path-based storage reduces memory footprint
- **Background garbage collection** - Non-blocking cleanup operations

## Button Outline Customization

tiny-dfr supports advanced per-button outline customization, allowing you to create visually distinct button categories with custom colors and visibility settings.

### Per-Button Outline Control

You can control outline visibility for individual buttons, overriding the global `ShowButtonOutlines` setting:

```toml
MediaLayerKeys = [
    { Icon = "omarchy", Action = "Command_1", ShowButtonOutlines = false },
    { Battery = "both", Action = "Battery", ShowButtonOutlines = true },
    { Time = "%I:%M%P", Action = "Time", ShowButtonOutlines = true },
]
```

### Custom Outline Colors

Each button can have its own outline color using the `ButtonOutlinesColor` field:

```toml
MediaLayerKeys = [
    # Grayscale color (0.0 = black, 1.0 = white)
    { Icon = "some-icon", Action = "SomeAction", ShowButtonOutlines = true, ButtonOutlinesColor = 0.5 },

    # RGB color array [red, green, blue] (0.0 to 1.0 range)
    { Time = "%I:%M%P", Action = "Time", ShowButtonOutlines = true, ButtonOutlinesColor = [0.2, 0.8, 1.0] },
]
```

### Color Formats

- **Grayscale**: Single floating-point value from 0.0 (black) to 1.0 (white)
- **RGB**: Array of three floating-point values [red, green, blue], each from 0.0 to 1.0
- **Fallback**: If no custom color is specified, uses the default outline color

### Battery Button Colors

The battery button automatically changes colors based on its state, regardless of custom outline colors:
- **Charging**: Green outline/background
- **Low battery (<10%)**: Red outline/background
- **Normal**: Uses configured outline color or default gray

## Keyboard Backlight Support

tiny-dfr automatically detects and controls hardware keyboard backlight on supported MacBooks. The system searches for keyboard backlight devices in the following priority order:

1. **T2 Mac specific path**: `/sys/class/leds/:white:kbd_backlight`
2. **Path with SMC prefix**: `/sys/class/leds/smc::kbd_backlight`
3. **Generic search**: Any device in `/sys/class/leds/` containing "kbd" or "keyboard"

### Testing and Feedback Needed

**Are you using a T1 or T2 MacBook?** I need your help to improve keyboard backlight compatibility!

If keyboard backlight isn't working on your MacBook, please:
1. Check what keyboard backlight devices exist on your system:
   ```bash
   ls -la /sys/class/leds/ | grep -i kbd
   # or
   find /sys/class/leds/ -name "*kbd*" -o -name "*keyboard*"
   ```

2. Report your findings in a GitHub issue with:
   - Your MacBook model (e.g., MacBook Pro 13,3, MacBook Air 8,1)
   - The path(s) found by the commands above
   - Whether you're using T1 or T2 hardware

This helps me add support for more MacBook touchbar models if any!

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

### Keyboard Backlight Issues
1. Check if keyboard backlight was detected at startup:
```bash
sudo journalctl -u tiny-dfr | grep -i "keyboard"
```

2. Manually check for keyboard backlight devices:
```bash
ls -la /sys/class/leds/ | grep -E "(kbd|keyboard)"
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

enhanced tiny-dfr has the same license like the original AsahiLinux/tiny-dfr which is under the MIT license, as included in the [LICENSE](LICENSE) file.

* Copyright The Asahi Linux Contributors

Please see the Git history for authorship information.

tiny-dfr embeds Google's [material-design-icons](https://github.com/google/material-design-icons)
which are licensed under [Apache License Version 2.0](LICENSE.material)
Some icons are derivatives of material-icons, with edits made by kekrby.
