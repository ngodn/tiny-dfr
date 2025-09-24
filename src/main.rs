use anyhow::{anyhow, Result};
use cairo::{Antialias, Context, Format, ImageSurface, Surface};
use chrono::{Local, Locale, Timelike, format::{StrftimeItems, Item as ChronoItem}};
use drm::control::ClipRect;
use freedesktop_icons::lookup;
use input::{
    event::{
        device::DeviceEvent,
        keyboard::{KeyState, KeyboardEvent, KeyboardEventTrait},
        touch::{TouchEvent, TouchEventPosition, TouchEventSlot},
        Event, EventTrait,
    },
    Device as InputDevice, Libinput, LibinputInterface,
};
use input_linux::{uinput::UInputHandle, EventKind, Key, SynchronizeKind};
use input_linux_sys::{input_event, input_id, timeval, uinput_setup};
use libc::{c_char, O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
use librsvg_rebind::{prelude::HandleExt, Handle, Rectangle};
use nix::{
    errno::Errno,
    sys::{
        epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags},
        signal::{SigSet, Signal},
    },
};
use std::{
    cmp::min,
    collections::HashMap,
    fs::{self, File, OpenOptions},
    os::{
        fd::{AsFd, AsRawFd},
        unix::{fs::OpenOptionsExt, io::OwnedFd},
    },
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};
use udev::MonitorBuilder;

mod backlight;
mod config;
mod display;
mod fonts;
mod hyprland;
mod keyboard_backlight;
mod pixel_shift;

use crate::config::ConfigManager;
use backlight::BacklightManager;
use config::{ButtonConfig, Config, ButtonAction, ButtonColor};
use display::DrmBackend;
use keyboard_backlight::KeyboardBacklightManager;
use pixel_shift::{PixelShiftManager, PIXEL_SHIFT_WIDTH_PX};

const BUTTON_SPACING_PX: i32 = 16;
const BUTTON_COLOR_INACTIVE: f64 = 0.200;
const BUTTON_COLOR_ACTIVE: f64 = 0.400;
const ICON_SIZE: i32 = 48;
const TIMEOUT_MS: i32 = 10 * 1000;

#[derive(Clone, Debug)]
struct NavigationState {
    navigation_stack: Vec<String>,
    current_expandable: Option<String>,
    last_interaction_time: std::time::Instant,
}

impl NavigationState {
    fn new() -> Self {
        NavigationState {
            navigation_stack: Vec::new(),
            current_expandable: None,
            last_interaction_time: std::time::Instant::now(),
        }
    }

    fn push_expandable(&mut self, expandable_name: String) {
        if let Some(current) = &self.current_expandable {
            self.navigation_stack.push(current.clone());
        }
        self.current_expandable = Some(expandable_name);
        self.last_interaction_time = std::time::Instant::now();
    }

    fn pop_expandable(&mut self) -> bool {
        if let Some(previous) = self.navigation_stack.pop() {
            self.current_expandable = Some(previous);
            self.last_interaction_time = std::time::Instant::now();
            true
        } else if self.current_expandable.is_some() {
            self.current_expandable = None;
            self.last_interaction_time = std::time::Instant::now();
            true
        } else {
            false
        }
    }

    fn reset_to_main(&mut self) {
        self.navigation_stack.clear();
        self.current_expandable = None;
        self.last_interaction_time = std::time::Instant::now();
    }

    fn update_interaction_time(&mut self) {
        self.last_interaction_time = std::time::Instant::now();
    }

    fn should_timeout(&self, timeout_seconds: u32) -> bool {
        timeout_seconds > 0 &&
        self.current_expandable.is_some() &&
        self.last_interaction_time.elapsed().as_secs() >= timeout_seconds as u64
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BatteryState {
    NotCharging,
    Charging,
    Low,
}

#[derive(Clone)]
struct BatteryImages {
    plain: Vec<Handle>,
    charging: Vec<Handle>,
    bolt: Handle,
}

#[derive(Eq, PartialEq, Copy, Clone)]
enum BatteryIconMode {
    Percentage,
    Icon,
    Both
}

impl BatteryIconMode {
    fn should_draw_icon(self) -> bool {
        self != BatteryIconMode::Percentage
    }
    fn should_draw_text(self) -> bool {
        self != BatteryIconMode::Icon
    }
}

#[derive(Clone)]
enum ButtonImage {
    Text(String),
    Svg(Handle),
    Bitmap(ImageSurface),
    Time(Vec<ChronoItem<'static>>, Locale),
    Battery(String, BatteryIconMode, BatteryImages),
    TextWithIcon(String, Handle),
}

#[derive(Clone)]
struct Button {
    image: ButtonImage,
    changed: bool,
    active: bool,
    action: ButtonAction,
    show_outline: Option<bool>,
    outline_color: Option<ButtonColor>,
}

fn try_load_svg(path: &str) -> Result<ButtonImage> {
    Ok(ButtonImage::Svg(
        Handle::from_file(path)?.ok_or(anyhow!("failed to load image"))?,
    ))
}

fn try_load_png(path: impl AsRef<Path>) -> Result<ButtonImage> {
    let mut file = File::open(path)?;
    let surf = ImageSurface::create_from_png(&mut file)?;
    if surf.height() == ICON_SIZE && surf.width() == ICON_SIZE {
        return Ok(ButtonImage::Bitmap(surf));
    }
    let resized = ImageSurface::create(Format::ARgb32, ICON_SIZE, ICON_SIZE).unwrap();
    let c = Context::new(&resized).unwrap();
    c.scale(
        ICON_SIZE as f64 / surf.width() as f64,
        ICON_SIZE as f64 / surf.height() as f64,
    );
    c.set_source_surface(surf, 0.0, 0.0).unwrap();
    c.set_antialias(Antialias::Best);
    c.paint().unwrap();
    Ok(ButtonImage::Bitmap(resized))
}

fn try_load_image(name: impl AsRef<str>, theme: Option<impl AsRef<str>>) -> Result<ButtonImage> {
    let name = name.as_ref();
    let locations;

    // Load list of candidate locations
    if let Some(theme) = theme {
        // Freedesktop icons
        let theme = theme.as_ref();
        let candidates = vec![
            lookup(name)
                .with_cache()
                .with_theme(theme)
                .with_size(ICON_SIZE as u16)
                .force_svg()
                .find(),
            lookup(name)
                .with_cache()
                .with_theme(theme)
                .force_svg()
                .find(),
        ];

        // .flatten() removes `None` and unwraps `Some` values
        locations = candidates.into_iter().flatten().collect();
    } else {
        // Standard file icons
        locations = vec![
            PathBuf::from(format!("/etc/tiny-dfr/{name}.svg")),
            PathBuf::from(format!("/etc/tiny-dfr/{name}.png")),
            PathBuf::from(format!("/usr/share/tiny-dfr/{name}.svg")),
            PathBuf::from(format!("/usr/share/tiny-dfr/{name}.png")),
        ];
    };

    // Try to load each candidate
    let mut last_err = anyhow!("no suitable icon path was found"); // in case locations is empty

    for location in locations {
        let result = match location.extension().and_then(|s| s.to_str()) {
            Some("png") => try_load_png(&location),
            Some("svg") => try_load_svg(
                location
                    .to_str()
                    .ok_or(anyhow!("image path is not unicode"))?,
            ),
            _ => Err(anyhow!("invalid file extension")),
        };

        match result {
            Ok(image) => return Ok(image),
            Err(err) => {
                last_err = err.context(format!("while loading path {}", location.display()));
            }
        };
    }

    // if function hasn't returned by now, all sources have been exhausted
    Err(last_err.context(format!("failed loading all possible paths for icon {name}")))
}

fn find_battery_device() -> Option<String> {
    let power_supply_path = "/sys/class/power_supply";
    if let Ok(entries) = fs::read_dir(power_supply_path) {
        for entry in entries.flatten() {
            let dev_path = entry.path();
            let type_path = dev_path.join("type");
            if let Ok(typ) = fs::read_to_string(&type_path) {
                if typ.trim() == "Battery" {
                    if let Some(name) = dev_path.file_name().and_then(|n| n.to_str()) {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}

fn get_battery_state(battery: &str) -> (u32, BatteryState) {
    let status_path = format!("/sys/class/power_supply/{}/status", battery);
    let status = fs::read_to_string(&status_path)
        .unwrap_or_else(|_| "Unknown".to_string());

    #[cfg(target_arch = "x86_64")]
    let capacity = {
        let charge_now_path = format!("/sys/class/power_supply/{}/charge_now", battery);
        let charge_full_path = format!("/sys/class/power_supply/{}/charge_full", battery);
        let charge_now = fs::read_to_string(&charge_now_path)
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok());
        let charge_full = fs::read_to_string(&charge_full_path)
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok());
        match (charge_now, charge_full) {
            (Some(now), Some(full)) if full > 0.0 => ((now / full) * 100.0).round() as u32,
            _ => 100,
        }
    };

    #[cfg(target_arch = "aarch64")]
    let capacity = {
        let capacity_path = format!("/sys/class/power_supply/{}/capacity", battery);
        fs::read_to_string(&capacity_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(100)
    };

    let status = match status.trim() {
        "Charging" | "Full" => BatteryState::Charging,
        "Discharging" if capacity < 10 => BatteryState::Low,
        _ => BatteryState::NotCharging,
    };
    (capacity, status)
}

impl Button {
    fn with_config(cfg: ButtonConfig) -> Button {
        let mut button = if let Some(text) = cfg.text {
            if text == "plugin-hyprland" {
                // Get Hyprland active window text - use "title" as default button title
                let (window_text, window_class) = match hyprland::get_active_window_info() {
                    Ok(info) => {
                        let title = info.get_text_by_button_title("title");
                        println!("Hyprland plugin: Got window title: '{}', class: '{}'", title, info.class);
                        (title, info.class.clone())
                    },
                    Err(e) => {
                        println!("Hyprland plugin error: {}", e);
                        ("Hyprland N/A".to_string(), String::new())
                    },
                };

                // For Hyprland buttons, always try to show app icon if available
                if !window_class.is_empty() {
                    // Try custom app_icon first, then auto-detect from class
                    let icon_name = if let Some(app_icon) = &cfg.app_icon {
                        app_icon.clone()
                    } else {
                        format!("app-{}", window_class)
                    };

                    // Check if the icon exists before trying to create TextWithIcon button
                    if try_load_image(&icon_name, cfg.theme.as_deref()).is_ok() {
                        Button::new_text_with_icon(window_text, icon_name, cfg.theme, cfg.action)
                    } else {
                        Button::new_text(window_text, cfg.action)
                    }
                } else {
                    Button::new_text(window_text, cfg.action)
                }
            } else {
                Button::new_text(text, cfg.action)
            }
        } else if let Some(icon) = cfg.icon {
            if icon == "plugin-hyprland" {
                // Get Hyprland active window icon
                let icon_name = match hyprland::get_active_window_info() {
                    Ok(info) => info.get_app_icon_name(),
                    Err(_) => "application-default-icon".to_string(),
                };
                Button::new_icon(&icon_name, cfg.theme, cfg.action)
            } else {
                Button::new_icon(&icon, cfg.theme, cfg.action)
            }
        } else if let Some(time) = cfg.time {
            Button::new_time(cfg.action, &time, cfg.locale.as_deref())
        } else if let Some(battery_mode) = cfg.battery {
            if let Some(battery) = find_battery_device() {
                Button::new_battery(cfg.action, battery, battery_mode, cfg.theme)
            } else {
                Button::new_text("Battery N/A".to_string(), cfg.action)
            }
        } else {
            panic!("Invalid config, a button must have either Text, Icon or Time")
        };

        button.show_outline = cfg.show_button_outlines;
        button.outline_color = cfg.button_outlines_color;
        button
    }
    fn new_text(text: String, action: ButtonAction) -> Button {
        Button {
            action,
            active: false,
            changed: false,
            image: ButtonImage::Text(text),
            show_outline: None,
            outline_color: None,
        }
    }
    fn new_text_with_icon(text: String, icon_name: String, theme: Option<impl AsRef<str>>, action: ButtonAction) -> Button {
        let icon = try_load_image(icon_name, theme).unwrap_or_else(|_| {
            // Fallback to a default icon if the specific app icon is not found
            try_load_image("application-default-icon", None::<&str>).unwrap_or_else(|_| {
                ButtonImage::Text("📱".to_string()) // Fallback emoji
            })
        });

        let icon_handle = match icon {
            ButtonImage::Svg(handle) => handle,
            _ => {
                // If not SVG, create a text button instead
                return Button::new_text(text, action);
            }
        };

        Button {
            action,
            active: false,
            changed: false,
            image: ButtonImage::TextWithIcon(format!(" {}", text), icon_handle), // Add space before text
            show_outline: None,
            outline_color: None,
        }
    }
    fn new_icon(path: impl AsRef<str>, theme: Option<impl AsRef<str>>, action: ButtonAction) -> Button {
        let image = try_load_image(path, theme).expect("failed to load icon");
        Button {
            action,
            image,
            active: false,
            changed: false,
            show_outline: None,
            outline_color: None,
        }
    }
    fn load_battery_image(icon: &str, theme: Option<impl AsRef<str>>) -> Handle {
        if let ButtonImage::Svg(svg) = try_load_image(icon, theme).unwrap() {
            return svg;
        }
        panic!("failed to load icon");
    }
    fn new_battery(action: ButtonAction, battery: String, battery_mode: String, theme: Option<impl AsRef<str>>) -> Button {
        let bolt = Self::load_battery_image("bolt", theme.as_ref());
        let mut plain = Vec::new();
        let mut charging = Vec::new();
        for icon in [
            "battery_0_bar", "battery_1_bar", "battery_2_bar", "battery_3_bar",
            "battery_4_bar", "battery_5_bar", "battery_6_bar", "battery_full",
        ] {
            plain.push(Self::load_battery_image(icon, theme.as_ref()));
        }
        for icon in [
            "battery_charging_20", "battery_charging_30", "battery_charging_50",
            "battery_charging_60", "battery_charging_80",
            "battery_charging_90", "battery_charging_full",
        ] {
            charging.push(Self::load_battery_image(icon, theme.as_ref()));
        }
        let battery_mode = match battery_mode.as_str() {
            "icon" => BatteryIconMode::Icon,
            "percentage" => BatteryIconMode::Percentage,
            "both" => BatteryIconMode::Both,
            _ => panic!("invalid battery mode, accepted modes: icon, percentage, both"),
        };
        Button {
            action,
            active: false,
            changed: false,
            image: ButtonImage::Battery(battery, battery_mode, BatteryImages {
                plain, bolt, charging
            }),
            show_outline: None,
            outline_color: None,
        }
    }

    fn new_time(action: ButtonAction, format: &str, locale_str: Option<&str>) -> Button {
        let format_str = if format == "24hr" {
            "%H:%M    %a %-e %b"
        } else if format == "12hr" {
            "%-l:%M %p    %a %-e %b"
        } else {
            format
        };

        let format_items = match StrftimeItems::new(format_str).parse_to_owned() {
            Ok(s) => s,
            Err(e) => panic!("Invalid time format, consult the configuration file for examples of correct ones: {e:?}"),
        };

        let locale = locale_str.and_then(|l| Locale::try_from(l).ok()).unwrap_or(Locale::POSIX);
        Button {
            action,
            active: false,
            changed: false,
            image: ButtonImage::Time(format_items, locale),
            show_outline: None,
            outline_color: None,
        }
    }
    fn render(
        &self,
        c: &Context,
        height: i32,
        button_left_edge: f64,
        button_width: u64,
        y_shift: f64,
    ) {
        match &self.image {
            ButtonImage::Text(text) => {
                let extents = c.text_extents(text).unwrap();
                c.move_to(
                    button_left_edge + (button_width as f64 / 2.0 - extents.width() / 2.0).round(),
                    y_shift + (height as f64 / 2.0 + extents.height() / 2.0).round(),
                );
                c.show_text(text).unwrap();
            }
            ButtonImage::TextWithIcon(text, svg) => {
                let text_extents = c.text_extents(text).unwrap();
                // Make icon fit button height with some padding, keeping aspect ratio
                let padding = 4.0;
                let icon_size = height as f64 - (padding * 2.0);
                let total_width = icon_size + text_extents.width(); // No extra spacing needed since text already has space

                // Center the combined icon+text in the button
                let start_x = button_left_edge + (button_width as f64 / 2.0 - total_width / 2.0).round();

                // Draw icon
                let icon_x = start_x;
                let icon_y = y_shift + padding;
                svg.render_document(c, &Rectangle::new(icon_x, icon_y, icon_size, icon_size))
                    .unwrap();

                // Draw text
                let text_x = start_x + icon_size;
                let text_y = y_shift + (height as f64 / 2.0 + text_extents.height() / 2.0).round();
                c.move_to(text_x, text_y);
                c.show_text(text).unwrap();
            }
            ButtonImage::Svg(svg) => {
                let x =
                    button_left_edge + (button_width as f64 / 2.0 - (ICON_SIZE / 2) as f64).round();
                let y = y_shift + ((height as f64 - ICON_SIZE as f64) / 2.0).round();

                svg.render_document(c, &Rectangle::new(x, y, ICON_SIZE as f64, ICON_SIZE as f64))
                    .unwrap();
            }
            ButtonImage::Bitmap(surf) => {
                let x =
                    button_left_edge + (button_width as f64 / 2.0 - (ICON_SIZE / 2) as f64).round();
                let y = y_shift + ((height as f64 - ICON_SIZE as f64) / 2.0).round();
                c.set_source_surface(surf, x, y).unwrap();
                c.rectangle(x, y, ICON_SIZE as f64, ICON_SIZE as f64);
                c.fill().unwrap();
            }
            ButtonImage::Time(format, locale) => {
                let current_time = Local::now();
                let formatted_time = current_time.format_localized_with_items(format.iter(), *locale).to_string();
                let time_extents = c.text_extents(&formatted_time).unwrap();
                c.move_to(
                    button_left_edge + (button_width as f64 / 2.0 - time_extents.width() / 2.0).round(),
                    y_shift + (height as f64 / 2.0 + time_extents.height() / 2.0).round()
                );
                c.show_text(&formatted_time).unwrap();
            }
            ButtonImage::Battery(battery, battery_mode, icons) => {
                let (capacity, state) = get_battery_state(battery);
                let icon = if battery_mode.should_draw_icon() {
                    Some(match state {
                        BatteryState::Charging => match capacity {
                            0..=20 => &icons.charging[0],
                            21..=30 => &icons.charging[1],
                            31..=50 => &icons.charging[2],
                            51..=60 => &icons.charging[3],
                            61..=80 => &icons.charging[4],
                            81..=99 => &icons.charging[5],
                            _ => &icons.charging[6],
                        },
                        _ => match capacity {
                            0 => &icons.plain[0],
                            1..=20 => &icons.plain[1],
                            21..=30 => &icons.plain[2],
                            31..=50 => &icons.plain[3],
                            51..=60 => &icons.plain[4],
                            61..=80 => &icons.plain[5],
                            81..=99 => &icons.plain[6],
                            _ => &icons.plain[7],
                        },
                    })
                } else if state == BatteryState::Charging {
                    Some(&icons.bolt)
                } else {
                    None
                };
                let percent_str = format!("{:.0}%", capacity);
                let extents = c.text_extents(&percent_str).unwrap();
                let mut width = extents.width();
                let mut text_offset = 0;
                if let Some(svg) = icon {
                    if !battery_mode.should_draw_text() {
                        width = ICON_SIZE as f64;
                    } else {
                        width += ICON_SIZE as f64;
                    }
                    text_offset = ICON_SIZE;
                    let x =
                        button_left_edge + (button_width as f64 / 2.0 - width / 2.0).round();
                    let y = y_shift + ((height as f64 - ICON_SIZE as f64) / 2.0).round();

                    svg.render_document(c, &Rectangle::new(x, y, ICON_SIZE as f64, ICON_SIZE as f64))
                        .unwrap();
                }
                if battery_mode.should_draw_text() {
                    c.move_to(
                        button_left_edge + (button_width as f64 / 2.0 - width / 2.0 + text_offset as f64).round(),
                        y_shift + (height as f64 / 2.0 + extents.height() / 2.0).round(),
                    );
                    c.show_text(&percent_str).unwrap();
                }
            }
        }
    }
    fn set_backround_color(&self, c: &Context, color: f64) {
        match &self.image {
            ButtonImage::Battery(battery, _, _) => {
                let (_, state) = get_battery_state(battery);
                match state {
                    BatteryState::NotCharging => c.set_source_rgb(color, color, color),
                    BatteryState::Charging => c.set_source_rgb(0.0, color, 0.0),
                    BatteryState::Low => c.set_source_rgb(color, 0.0, 0.0),
                }
            }
            _ => c.set_source_rgb(color, color, color),
        }
    }
}

#[derive(Default, Clone)]
pub struct FunctionLayer {
    displays_time: bool,
    displays_battery: bool,
    buttons: Vec<(usize, Button)>,
    virtual_button_count: usize,
}

impl FunctionLayer {
    fn with_config(cfg: Vec<ButtonConfig>) -> FunctionLayer {
        if cfg.is_empty() {
            panic!("Invalid configuration, layer has 0 buttons");
        }

        let mut virtual_button_count = 0;
        FunctionLayer {
            displays_time: cfg.iter().any(|cfg| cfg.time.is_some()),
            displays_battery: cfg.iter().any(|cfg| cfg.battery.is_some()),
            buttons: cfg
                .into_iter()
                .scan(&mut virtual_button_count, |state, cfg| {
                    let i = **state;
                    let mut stretch = cfg.stretch.unwrap_or(1);
                    if stretch < 1 {
                        println!("Stretch value must be at least 1, setting to 1.");
                        stretch = 1;
                    }
                    **state += stretch;
                    Some((i, Button::with_config(cfg)))
                })
                .collect(),
            virtual_button_count,
        }
    }
    fn draw(
        &mut self,
        config: &Config,
        width: i32,
        height: i32,
        surface: &Surface,
        pixel_shift: (f64, f64),
        complete_redraw: bool,
    ) -> Vec<ClipRect> {
        let c = Context::new(surface).unwrap();
        let mut modified_regions = if complete_redraw {
            vec![ClipRect::new(0, 0, height as u16, width as u16)]
        } else {
            Vec::new()
        };
        c.translate(height as f64, 0.0);
        c.rotate((90.0f64).to_radians());
        let pixel_shift_width = if config.enable_pixel_shift {
            PIXEL_SHIFT_WIDTH_PX
        } else {
            0
        };
        let virtual_button_width = ((width - pixel_shift_width as i32)
            - (BUTTON_SPACING_PX * (self.virtual_button_count - 1) as i32))
            as f64
            / self.virtual_button_count as f64;
        let radius = 8.0f64;
        let bot = (height as f64) * 0.15;
        let top = (height as f64) * 0.85;
        let (pixel_shift_x, pixel_shift_y) = pixel_shift;

        if complete_redraw {
            c.set_source_rgb(0.0, 0.0, 0.0);
            c.paint().unwrap();
        }
        c.set_font_face(&config.font_face);
        c.set_font_size(32.0);

        for i in 0..self.buttons.len() {
            let end = if i + 1 < self.buttons.len() {
                self.buttons[i + 1].0
            } else {
                self.virtual_button_count
            };
            let (start, button) = &mut self.buttons[i];
            let start = *start;

            if !button.changed && !complete_redraw {
                continue;
            };

            let left_edge = (start as f64 * (virtual_button_width + BUTTON_SPACING_PX as f64))
                .floor()
                + pixel_shift_x
                + (pixel_shift_width / 2) as f64;

            let button_width = virtual_button_width
                + ((end - start - 1) as f64 * (virtual_button_width + BUTTON_SPACING_PX as f64))
                    .floor();

            let show_outline = button.show_outline.unwrap_or(config.show_button_outlines);
            if !complete_redraw {
                c.set_source_rgb(0.0, 0.0, 0.0);
                c.rectangle(
                    left_edge,
                    bot - radius,
                    button_width,
                    top - bot + radius * 2.0,
                );
                c.fill().unwrap();
            }

            if button.active {
                button.set_backround_color(&c, BUTTON_COLOR_ACTIVE);
            } else if show_outline {
                if let Some(custom_color) = &button.outline_color {
                    custom_color.set_cairo_source(&c);
                } else {
                    button.set_backround_color(&c, BUTTON_COLOR_INACTIVE);
                }
            } else {
                button.set_backround_color(&c, 0.0);
            }
            // draw box with rounded corners
            c.new_sub_path();
            let left = left_edge + radius;
            let right = (left_edge + button_width.ceil()) - radius;
            c.arc(
                right,
                bot,
                radius,
                (-90.0f64).to_radians(),
                (0.0f64).to_radians(),
            );
            c.arc(
                right,
                top,
                radius,
                (0.0f64).to_radians(),
                (90.0f64).to_radians(),
            );
            c.arc(
                left,
                top,
                radius,
                (90.0f64).to_radians(),
                (180.0f64).to_radians(),
            );
            c.arc(
                left,
                bot,
                radius,
                (180.0f64).to_radians(),
                (270.0f64).to_radians(),
            );
            c.close_path();

            c.fill().unwrap();
            c.set_source_rgb(1.0, 1.0, 1.0);
            button.render(
                &c,
                height,
                left_edge,
                button_width.ceil() as u64,
                pixel_shift_y,
            );

            button.changed = false;

            if !complete_redraw {
                modified_regions.push(ClipRect::new(
                    height as u16 - top as u16 - radius as u16,
                    left_edge as u16,
                    height as u16 - bot as u16 + radius as u16,
                    left_edge as u16 + button_width as u16,
                ));
            }
        }

        modified_regions
    }

    fn hit(&self, width: u16, height: u16, x: f64, y: f64, i: Option<usize>) -> Option<usize> {
        let virtual_button_width =
            (width as i32 - (BUTTON_SPACING_PX * (self.virtual_button_count - 1) as i32)) as f64
                / self.virtual_button_count as f64;

        let i = i.unwrap_or_else(|| {
            let virtual_i = (x / (width as f64 / self.virtual_button_count as f64)) as usize;
            self.buttons
                .iter()
                .position(|(start, _)| *start > virtual_i)
                .unwrap_or(self.buttons.len())
                - 1
        });
        if i >= self.buttons.len() {
            return None;
        }

        let start = self.buttons[i].0;
        let end = if i + 1 < self.buttons.len() {
            self.buttons[i + 1].0
        } else {
            self.virtual_button_count
        };

        let left_edge = (start as f64 * (virtual_button_width + BUTTON_SPACING_PX as f64)).floor();

        let button_width = virtual_button_width
            + ((end - start - 1) as f64 * (virtual_button_width + BUTTON_SPACING_PX as f64))
                .floor();

        if x < left_edge
            || x > (left_edge + button_width)
            || y < 0.1 * height as f64
            || y > 0.9 * height as f64
        {
            return None;
        }

        Some(i)
    }
}

struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let mode = flags & O_ACCMODE;

        OpenOptions::new()
            .custom_flags(flags)
            .read(mode == O_RDONLY || mode == O_RDWR)
            .write(mode == O_WRONLY || mode == O_RDWR)
            .open(path)
            .map(|file| file.into())
            .map_err(|err| err.raw_os_error().unwrap())
    }
    fn close_restricted(&mut self, fd: OwnedFd) {
        _ = File::from(fd);
    }
}

fn emit<F>(uinput: &mut UInputHandle<F>, ty: EventKind, code: u16, value: i32)
where
    F: AsRawFd,
{
    uinput
        .write(&[input_event {
            value,
            type_: ty as u16,
            code,
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
        }])
        .unwrap();
}

fn toggle_key<F>(uinput: &mut UInputHandle<F>, code: Key, value: i32)
where
    F: AsRawFd,
{
    emit(uinput, EventKind::Key, code as u16, value);
    emit(
        uinput,
        EventKind::Synchronize,
        SynchronizeKind::Report as u16,
        0,
    );
}

fn update_layer_for_navigation(navigation_state: &NavigationState, config: &Config, layers: &mut [FunctionLayer; 2], active_layer: &mut usize, needs_complete_redraw: &mut bool, original_layers: &[FunctionLayer; 2], touches: &mut HashMap<u32, (usize, usize)>) {
    if let Some(expandable_name) = &navigation_state.current_expandable {
        if let Some(expandable_buttons) = config.expandables.get(expandable_name) {
            // Create back button
            let back_button = ButtonConfig {
                icon: Some("back".to_string()),
                text: Some("Back".to_string()),
                theme: None,
                time: None,
                battery: None,
                locale: None,
                action: ButtonAction::Command("Back".to_string()),
                stretch: None,
                show_button_outlines: Some(config.back_button_show_outlines),
                button_outlines_color: config.back_button_outline_color.clone(),
                show_app_icon_alongside_text: None,
                app_icon: None,
            };

            // Combine back button with expandable buttons
            let mut combined_buttons = vec![back_button];
            combined_buttons.extend_from_slice(expandable_buttons);

            // Replace the current layer with the expandable
            layers[*active_layer] = FunctionLayer::with_config(combined_buttons);
            *needs_complete_redraw = true;

            // Clear all active touches to prevent accidental triggering in new layout
            clear_all_touches(layers, touches);
        }
    } else {
        // Return to original configuration
        layers[0] = original_layers[0].clone();
        layers[1] = original_layers[1].clone();
        *needs_complete_redraw = true;

        // Clear all active touches to prevent accidental triggering in new layout
        clear_all_touches(layers, touches);
    }
}

fn clear_all_touches(layers: &mut [FunctionLayer; 2], touches: &mut HashMap<u32, (usize, usize)>) {
    // Only clear if there are actually touches to clear
    if touches.is_empty() {
        return;
    }

    // Deactivate only the buttons that are currently active
    for layer in layers.iter_mut() {
        for (_, button) in layer.buttons.iter_mut() {
            if button.active {
                button.active = false;
                button.changed = true;
            }
        }
    }

    // Clear the touches map
    touches.clear();
}

fn handle_hyprland_expand(hyprland_expand_name: &str, config: &Config, navigation_state: &mut NavigationState, layers: &mut [FunctionLayer; 2], active_layer: &mut usize, needs_complete_redraw: &mut bool, _original_layers: &[FunctionLayer; 2], touches: &mut HashMap<u32, (usize, usize)>) {
    // Get the active window information
    let active_window_info = match hyprland::get_active_window_info() {
        Ok(info) => info,
        Err(_) => {
            // If we can't get active window info, ignore the button press
            return;
        }
    };

    // Check if we have a Hyprland expandable configuration for this action
    if let Some(hyprland_configs) = config.hyprland_expandables.get(hyprland_expand_name) {
        // Find a matching configuration based on the active window class
        let matching_config = hyprland_configs.iter().find(|config| {
            config.class == active_window_info.class
        });

        if let Some(matched_config) = matching_config {
            // Create the button title based on configuration
            let button_title = matched_config.button_title.as_deref().unwrap_or("title");
            let window_text = active_window_info.get_text_by_button_title(button_title);

            // Create a dynamic back button showing the active window title
            let mut window_button_config = ButtonConfig {
                icon: Some("back".to_string()), // Show back arrow icon
                text: Some(window_text.clone()), // Show window title
                theme: None,
                time: None,
                battery: None,
                locale: None,
                action: ButtonAction::Command("Back".to_string()),
                stretch: None,
                show_button_outlines: Some(config.back_button_show_outlines),
                button_outlines_color: config.back_button_outline_color.clone(),
                show_app_icon_alongside_text: Some(true), // Show icon alongside text
                app_icon: Some("back".to_string()), // Use back icon
            };

            // Combine window button with expandable layer keys
            let mut combined_buttons = vec![window_button_config];
            combined_buttons.extend_from_slice(&matched_config.layer_keys);

            // Replace the current layer with the expandable
            layers[*active_layer] = FunctionLayer::with_config(combined_buttons);
            *needs_complete_redraw = true;

            // Push to navigation state to track this expansion
            navigation_state.push_expandable(format!("hyprland_{}", hyprland_expand_name));

            // Clear all active touches to prevent accidental triggering in new layout
            clear_all_touches(layers, touches);
        }
        // If no matching configuration found for the current window class, ignore the button press
    }
    // If no Hyprland expandable configuration found, ignore the button press
}

fn handle_button_action<F>(uinput: &mut UInputHandle<F>, action: &ButtonAction, config: &Config, active: bool, navigation_state: &mut NavigationState, layers: &mut [FunctionLayer; 2], active_layer: &mut usize, needs_complete_redraw: &mut bool, original_layers: &[FunctionLayer; 2], touches: &mut HashMap<u32, (usize, usize)>)
where
    F: AsRawFd,
{
    match action {
        ButtonAction::Key(key) => {
            toggle_key(uinput, *key, active as i32);
        }
        ButtonAction::KeyCombos(keys) => {
            if active {
                // Press all keys in the combination
                for key in keys {
                    toggle_key(uinput, *key, 1);
                }
            } else {
                // Release all keys in reverse order
                for key in keys.iter().rev() {
                    toggle_key(uinput, *key, 0);
                }
            }
        }
        ButtonAction::Command(command_id) => {
            if active {
                if command_id == "Back" {
                    // Handle back button
                    if navigation_state.pop_expandable() {
                        update_layer_for_navigation(navigation_state, config, layers, active_layer, needs_complete_redraw, original_layers, touches);
                    }
                } else {
                    execute_command(command_id, config);
                }
            }
        }
        ButtonAction::Expand(expandable_name) => {
            if active {
                navigation_state.push_expandable(expandable_name.clone());
                update_layer_for_navigation(navigation_state, config, layers, active_layer, needs_complete_redraw, original_layers, touches);
            }
        }
        ButtonAction::HyprlandExpand(hyprland_expand_name) => {
            if active {
                handle_hyprland_expand(hyprland_expand_name, config, navigation_state, layers, active_layer, needs_complete_redraw, original_layers, touches);
            }
        }
    }
}

fn execute_command(command_id: &str, config: &Config) {
    if let Some(command) = config.commands.get(command_id) {
        // Execute command in the background with user environment
        std::thread::spawn({
            let command = command.clone();
            let user_env = config.user_env.clone();
            move || {
                println!("Executing command: {}", command);

                if let Some(username) = detect_desktop_user() {
                    // Use runuser with login shell - no password required, reads .bash_profile, .bashrc, etc.
                    let mut cmd = std::process::Command::new("/usr/bin/runuser");
                    cmd.args(["-l", &username, "-c", &command]);

                    // Set essential environment variables for GUI apps
                    if let Some(uid) = get_user_id(&username) {
                        let runtime_dir = format!("/run/user/{}", uid);
                        let home_dir = format!("/home/{}", username);

                        // Set comprehensive PATH to include common user binary locations
                        let enhanced_path = format!(
                            "{}/.local/share/omarchy/bin:{}/.local/bin:{}/.config/nvm/versions/node/latest/bin:{}/.local/share/pnpm:/usr/local/sbin:/usr/local/bin:/usr/bin:/bin:/var/lib/flatpak/exports/bin",
                            home_dir, home_dir, home_dir, home_dir
                        );

                        // Use user environment config if available, otherwise detect
                        let wayland_display = if let Some(user_env) = &user_env {
                            user_env.wayland_display.clone()
                        } else {
                            detect_wayland_display(&runtime_dir).unwrap_or_else(|| "wayland-1".to_string())
                        };

                        // Build command with environment variables embedded (to work with runuser -l)
                        let env_command = format!(
                            "export PATH='{}' DISPLAY=':0' WAYLAND_DISPLAY='{}' XDG_RUNTIME_DIR='{}'; {}",
                            enhanced_path.clone(), wayland_display, runtime_dir, command
                        );

                        // Update command to use embedded environment
                        cmd = std::process::Command::new("/usr/bin/runuser");
                        cmd.args(["-l", &username, "-c", &env_command]);
                    }

                    if let Err(e) = cmd.spawn() {
                        eprintln!("Failed to execute command '{}' as user '{}': {}", command, username, e);

                        // Fallback to basic execution
                        fallback_execution(&command);
                    }
                } else {
                    fallback_execution(&command);
                }
            }
        });
    } else {
        eprintln!("Command '{}' not found in commands.toml", command_id);
    }
}

fn fallback_execution(command: &str) {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Err(e) = cmd.spawn() {
        eprintln!("Failed to execute command '{}': {}", command, e);
    }
}

fn detect_desktop_user() -> Option<String> {
    // Method 1: Check SUDO_USER if running via sudo
    if let Ok(user) = std::env::var("SUDO_USER") {
        return Some(user);
    }

    // Method 2: Check loginctl for active graphical sessions (works for both X11 and Wayland)
    if let Ok(output) = std::process::Command::new("loginctl")
        .args(&["list-sessions", "--no-legend"])
        .output() {
        if let Ok(sessions) = String::from_utf8(output.stdout) {
            for line in sessions.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 && parts[3] == "seat0" && parts[2] != "root" {
                    // Check if this session has a graphical environment
                    if let Ok(session_output) = std::process::Command::new("loginctl")
                        .args(&["show-session", parts[0], "-p", "Type"])
                        .output() {
                        if let Ok(session_info) = String::from_utf8(session_output.stdout) {
                            if session_info.contains("Type=wayland") || session_info.contains("Type=x11") {
                                return Some(parts[2].to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Method 3: Check who owns the Wayland runtime directory
    if let Ok(entries) = std::fs::read_dir("/run/user") {
        for entry in entries.flatten() {
            if let Some(uid_str) = entry.file_name().to_str() {
                if let Ok(uid) = uid_str.parse::<u32>() {
                    if uid >= 1000 && uid < 65534 { // Regular user UID range
                        let wayland_socket = entry.path().join("wayland-0");
                        if wayland_socket.exists() {
                            // Get username from UID
                            if let Ok(output) = std::process::Command::new("getent")
                                .args(&["passwd", uid_str])
                                .output() {
                                if let Ok(passwd_line) = String::from_utf8(output.stdout) {
                                    if let Some(username) = passwd_line.split(':').next() {
                                        return Some(username.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn get_user_id(username: &str) -> Option<u32> {
    if let Ok(output) = std::process::Command::new("id")
        .args(&["-u", username])
        .output() {
        if let Ok(uid_str) = String::from_utf8(output.stdout) {
            return uid_str.trim().parse().ok();
        }
    }
    None
}

fn expand_user_path(username: &str) -> Result<String, std::io::Error> {
    use std::fs;

    let mut paths = vec![
        "/usr/local/bin".to_string(),
        "/usr/bin".to_string(),
        "/bin".to_string(),
    ];

    // Add user's local bin
    paths.insert(0, format!("/home/{}/.local/bin", username));

    // Expand .local/share/*/bin directories
    let local_share_path = format!("/home/{}/.local/share", username);
    println!("Expanding path, checking: {}", local_share_path);

    match fs::read_dir(&local_share_path) {
        Ok(entries) => {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        let bin_path = entry.path().join("bin");
                        println!("Checking bin path: {:?}", bin_path);
                        if bin_path.exists() {
                            let bin_path_str = bin_path.to_string_lossy().to_string();
                            println!("Found bin directory: {}", bin_path_str);
                            paths.insert(1, bin_path_str);
                        }
                    }
                }
            }
        }
        Err(e) => {
            println!("Failed to read {}: {}", local_share_path, e);
            return Err(e);
        }
    }

    Ok(paths.join(":"))
}

fn detect_wayland_display(runtime_dir: &str) -> Option<String> {
    use std::fs;

    // Check for wayland-* sockets in the runtime directory
    if let Ok(entries) = fs::read_dir(runtime_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("wayland-") && !name.ends_with(".lock") {
                    return Some(name.to_string());
                }
            }
        }
    }

    None
}

fn main() {
    let mut drm = DrmBackend::open_card().unwrap();
    let (height, width) = drm.mode().size();
    let _ = panic::catch_unwind(AssertUnwindSafe(|| real_main(&mut drm)));
    let crash_bitmap = include_bytes!("crash_bitmap.raw");
    let mut map = drm.map().unwrap();
    let data = map.as_mut();
    let mut wptr = 0;
    for byte in crash_bitmap {
        for i in 0..8 {
            let bit = ((byte >> i) & 0x1) == 0;
            let color = if bit { 0xFF } else { 0x0 };
            data[wptr] = color;
            data[wptr + 1] = color;
            data[wptr + 2] = color;
            data[wptr + 3] = color;
            wptr += 4;
        }
    }
    drop(map);
    drm.dirty(&[ClipRect::new(0, 0, height, width)]).unwrap();
    let mut sigset = SigSet::empty();
    sigset.add(Signal::SIGTERM);
    sigset.wait().unwrap();
}

fn real_main(drm: &mut DrmBackend) {
    let (height, width) = drm.mode().size();
    let (db_width, db_height) = drm.fb_info().unwrap().size();
    let mut uinput = UInputHandle::new(OpenOptions::new().write(true).open("/dev/uinput").unwrap());
    let mut backlight = BacklightManager::new();
    let mut last_redraw_minute = Local::now().minute();
    let mut last_battery_update_minute = Local::now().minute();
    let mut cfg_mgr = ConfigManager::new();
    let (mut cfg, mut layers) = cfg_mgr.load_config(width);
    
    // Initialize keyboard backlight BEFORE dropping privileges
    let mut kbd_backlight = KeyboardBacklightManager::new_with_config(
        cfg.keyboard_brightness_step
    );
    
    // Log keyboard backlight availability
    if kbd_backlight.is_available() {
        println!("Keyboard backlight control enabled - Max brightness: {}", 
                 kbd_backlight.max_brightness());
    } else {
        println!("Keyboard backlight control disabled - falling back to key events");
    }
    
    let mut pixel_shift = PixelShiftManager::new();

    // Keep running as root to allow command execution
    // Note: Privilege dropping disabled to allow access to user files for command execution

    let mut surface =
        ImageSurface::create(Format::ARgb32, db_width as i32, db_height as i32).unwrap();
    let mut active_layer = 0;
    let mut needs_complete_redraw = true;
    let mut navigation_state = NavigationState::new();
    let mut original_layers = layers.clone(); // Store original layers for reset

    let mut input_tb = Libinput::new_with_udev(Interface);
    let mut input_main = Libinput::new_with_udev(Interface);
    input_tb.udev_assign_seat("seat-touchbar").unwrap();
    input_main.udev_assign_seat("seat0").unwrap();
    let udev_monitor = MonitorBuilder::new()
        .unwrap()
        .match_subsystem("power_supply")
        .unwrap()
        .listen()
        .unwrap();
    let epoll = Epoll::new(EpollCreateFlags::empty()).unwrap();
    epoll
        .add(input_main.as_fd(), EpollEvent::new(EpollFlags::EPOLLIN, 0))
        .unwrap();
    epoll
        .add(input_tb.as_fd(), EpollEvent::new(EpollFlags::EPOLLIN, 1))
        .unwrap();
    epoll
        .add(cfg_mgr.fd(), EpollEvent::new(EpollFlags::EPOLLIN, 2))
        .unwrap();
    epoll
        .add(&udev_monitor, EpollEvent::new(EpollFlags::EPOLLIN, 3))
        .unwrap();
    uinput.set_evbit(EventKind::Key).unwrap();
    for layer in &layers {
        for button in &layer.buttons {
            match &button.1.action {
                ButtonAction::Key(key) => {
                    uinput.set_keybit(*key).unwrap();
                }
                ButtonAction::KeyCombos(keys) => {
                    for key in keys {
                        uinput.set_keybit(*key).unwrap();
                    }
                }
                _ => {}
            }
        }
    }

    // Also register keys from expandables
    for expandable_buttons in cfg.expandables.values() {
        for button in expandable_buttons {
            match &button.action {
                ButtonAction::Key(key) => {
                    uinput.set_keybit(*key).unwrap();
                }
                ButtonAction::KeyCombos(keys) => {
                    for key in keys {
                        uinput.set_keybit(*key).unwrap();
                    }
                }
                _ => {}
            }
        }
    }

    // Also register keys from hyprland expandables
    for hyprland_expandable_configs in cfg.hyprland_expandables.values() {
        for hyprland_config in hyprland_expandable_configs {
            for button in &hyprland_config.layer_keys {
                match &button.action {
                    ButtonAction::Key(key) => {
                        uinput.set_keybit(*key).unwrap();
                    }
                    ButtonAction::KeyCombos(keys) => {
                        for key in keys {
                            uinput.set_keybit(*key).unwrap();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut dev_name_c = [0 as c_char; 80];
    let dev_name = "Dynamic Function Row Virtual Input Device".as_bytes();
    for i in 0..dev_name.len() {
        dev_name_c[i] = dev_name[i] as c_char;
    }
    uinput
        .dev_setup(&uinput_setup {
            id: input_id {
                bustype: 0x19,
                vendor: 0x1209,
                product: 0x316E,
                version: 1,
            },
            ff_effects_max: 0,
            name: dev_name_c,
        })
        .unwrap();
    uinput.dev_create().unwrap();

    let mut digitizer: Option<InputDevice> = None;
    let mut touches: HashMap<u32, (usize, usize)> = HashMap::new();
    loop {
        if cfg_mgr.update_config(&mut cfg, &mut layers, width) {
            active_layer = 0;
            needs_complete_redraw = true;
            original_layers = layers.clone(); // Update original layers
            navigation_state.reset_to_main(); // Reset navigation on config update

            // Update keyboard backlight step size only (can't recreate manager after privilege drop)
            kbd_backlight.update_brightness_step(cfg.keyboard_brightness_step);
        }

        // Check for timeout and return to main layer (only if we're actually in an expandable)
        if navigation_state.current_expandable.is_some() && navigation_state.should_timeout(cfg.expandable_timeout_seconds) {
            navigation_state.reset_to_main();
            layers[0] = original_layers[0].clone();
            layers[1] = original_layers[1].clone();
            needs_complete_redraw = true;
            // Clear touches to prevent accidental triggering after timeout
            clear_all_touches(&mut layers, &mut touches);
        }

        let now = Local::now();
        let ms_left = ((60 - now.second()) * 1000) as i32;
        let mut next_timeout_ms = min(ms_left, TIMEOUT_MS);

        if cfg.enable_pixel_shift {
            let (pixel_shift_needs_redraw, pixel_shift_next_timeout_ms) = pixel_shift.update();
            if pixel_shift_needs_redraw {
                needs_complete_redraw = true;
            }
            next_timeout_ms = min(next_timeout_ms, pixel_shift_next_timeout_ms);
        }

        // Add expandable timeout to the calculation if we're in an expandable
        if navigation_state.current_expandable.is_some() && cfg.expandable_timeout_seconds > 0 {
            let elapsed_ms = navigation_state.last_interaction_time.elapsed().as_millis() as i32;
            let timeout_ms = (cfg.expandable_timeout_seconds * 1000) as i32;
            let remaining_ms = timeout_ms - elapsed_ms;
            if remaining_ms > 0 {
                next_timeout_ms = min(next_timeout_ms, remaining_ms);
            }
        }

        let current_minute = now.minute();
        if layers[active_layer].displays_time && (current_minute != last_redraw_minute) {
            needs_complete_redraw = true;
            last_redraw_minute = current_minute;
        }
        if layers[active_layer].displays_battery && (current_minute != last_battery_update_minute) {
            for button in &mut layers[active_layer].buttons {
                if let ButtonImage::Battery(_, _, _) = button.1.image {
                    button.1.changed = true;
                }
            }
            last_battery_update_minute = current_minute;
        }

        // Check for Hyprland plugin updates and update button content
        if hyprland::check_and_reset_cache_updated() {
            if let Ok(window_info) = hyprland::get_active_window_info() {
                for button in &mut layers[active_layer].buttons {
                    // Check if this is a hyprland plugin button and update its content
                    match &button.1.action {
                        config::ButtonAction::HyprlandExpand(expand_name) => {
                            // This is definitely a hyprland button, always try to show app icon if available
                            let app_icon_name = window_info.get_app_icon_name();
                            if let Ok(icon) = try_load_image(&app_icon_name, None::<&str>) {
                                if let ButtonImage::Svg(icon_handle) = icon {
                                    // Show with app icon
                                    button.1.image = ButtonImage::TextWithIcon(
                                        format!(" {}", window_info.get_text_by_button_title("title")),
                                        icon_handle
                                    );
                                    button.1.changed = true;
                                } else {
                                    // Fallback to text only if icon can't be loaded as SVG
                                    button.1.image = ButtonImage::Text(window_info.get_text_by_button_title("title"));
                                    button.1.changed = true;
                                }
                            } else {
                                // Fallback to text only if app-{class} icon not found
                                button.1.image = ButtonImage::Text(window_info.get_text_by_button_title("title"));
                                button.1.changed = true;
                            }
                        }
                        _ => {
                            // For other buttons, check if they might be plugin-hyprland text buttons
                            match &button.1.image {
                                ButtonImage::Text(text) if text.contains("Alacritty") || text.contains("code") || text.contains("Visual Studio") => {
                                    button.1.image = ButtonImage::Text(window_info.get_text_by_button_title("title"));
                                    button.1.changed = true;
                                }
                                ButtonImage::TextWithIcon(text, _) if text.contains("Alacritty") || text.contains("code") || text.contains("Visual Studio") => {
                                    // For non-Hyprland expand buttons, keep as text only
                                    button.1.image = ButtonImage::Text(window_info.get_text_by_button_title("title"));
                                    button.1.changed = true;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        if needs_complete_redraw || layers[active_layer].buttons.iter().any(|b| b.1.changed) {
            let shift = if cfg.enable_pixel_shift {
                pixel_shift.get()
            } else {
                (0.0, 0.0)
            };
            let clips = layers[active_layer].draw(
                &cfg,
                width as i32,
                height as i32,
                &surface,
                shift,
                needs_complete_redraw,
            );
            let data = surface.data().unwrap();
            drm.map().unwrap().as_mut()[..data.len()].copy_from_slice(&data);
            drm.dirty(&clips).unwrap();
            needs_complete_redraw = false;
        }

        match epoll.wait(
            &mut [EpollEvent::new(EpollFlags::EPOLLIN, 0)],
            next_timeout_ms as u16,
        ) {
            Err(Errno::EINTR) | Ok(_) => 0,
            e => e.unwrap(),
        };

        _ = udev_monitor.iter().last();

        input_tb.dispatch().unwrap();
        input_main.dispatch().unwrap();
        for event in &mut input_tb.clone().chain(input_main.clone()) {
            backlight.process_event(&event);
            match event {
                Event::Device(DeviceEvent::Added(evt)) => {
                    let dev = evt.device();
                    if dev.name().contains(" Touch Bar") {
                        digitizer = Some(dev);
                    }
                }
                Event::Keyboard(KeyboardEvent::Key(key)) => {
                    if key.key() == Key::Fn as u32 {
                        let new_layer = match key.key_state() {
                            KeyState::Pressed => 1,
                            KeyState::Released => 0,
                        };
                        if active_layer != new_layer {
                            active_layer = new_layer;
                            needs_complete_redraw = true;
                        }
                    }
                }
                Event::Touch(te) => {
                    if Some(te.device()) != digitizer || backlight.current_bl() == 0 {
                        continue;
                    }
                    match te {
                        TouchEvent::Down(dn) => {
                            let x = dn.x_transformed(width as u32);
                            let y = dn.y_transformed(height as u32);
                            if let Some(btn) = layers[active_layer].hit(width, height, x, y, None) {
                                touches.insert(dn.seat_slot(), (active_layer, btn));
                                
                                // Get the button action before borrowing layers mutably
                                let button_action = &layers[active_layer].buttons[btn].1.action;
                                
                                // Handle keyboard backlight actions directly
                                let handled_by_keyboard_backlight = if cfg.keyboard_brightness_enabled {
                                    match button_action {
                                        ButtonAction::Key(Key::IllumUp) => {
                                            kbd_backlight.increase_brightness()
                                        }
                                        ButtonAction::Key(Key::IllumDown) => {
                                            kbd_backlight.decrease_brightness()
                                        }
                                        _ => false
                                    }
                                } else {
                                    false
                                };
                                
                                // Only send key event if we didn't handle it with keyboard backlight
                                if !handled_by_keyboard_backlight {
                                    // Extract the button action to avoid borrowing conflict
                                    let action = layers[active_layer].buttons[btn].1.action.clone();
                                    let old_active = layers[active_layer].buttons[btn].1.active;
                                    if old_active != true {
                                        layers[active_layer].buttons[btn].1.active = true;
                                        layers[active_layer].buttons[btn].1.changed = true;
                                        handle_button_action(&mut uinput, &action, &cfg, true, &mut navigation_state, &mut layers, &mut active_layer, &mut needs_complete_redraw, &original_layers, &mut touches);
                                    }
                                } else {
                                    // Show visual feedback for keyboard backlight buttons (without key event)
                                    layers[active_layer].buttons[btn].1.active = true;
                                    layers[active_layer].buttons[btn].1.changed = true;
                                }

                                // Update interaction time for any touch
                                navigation_state.update_interaction_time();
                            }
                        }
                        TouchEvent::Motion(mtn) => {
                            if !touches.contains_key(&mtn.seat_slot()) {
                                continue;
                            }

                            let x = mtn.x_transformed(width as u32);
                            let y = mtn.y_transformed(height as u32);
                            let (layer, btn) = *touches.get(&mtn.seat_slot()).unwrap();
                            let hit = layers[active_layer]
                                .hit(width, height, x, y, Some(btn))
                                .is_some();
                            
                            // Check if this is a keyboard backlight button
                            let button_action = &layers[layer].buttons[btn].1.action;
                            let is_kbd_backlight_button = cfg.keyboard_brightness_enabled &&
                                matches!(button_action, ButtonAction::Key(Key::IllumUp) | ButtonAction::Key(Key::IllumDown));
                            
                            if !is_kbd_backlight_button {
                                // Extract the button action to avoid borrowing conflict
                                let action = layers[layer].buttons[btn].1.action.clone();
                                let old_active = layers[layer].buttons[btn].1.active;
                                if old_active != hit {
                                    layers[layer].buttons[btn].1.active = hit;
                                    layers[layer].buttons[btn].1.changed = true;
                                    handle_button_action(&mut uinput, &action, &cfg, hit, &mut navigation_state, &mut layers, &mut active_layer, &mut needs_complete_redraw, &original_layers, &mut touches);
                                }
                            } else {
                                // Handle visual feedback for keyboard backlight buttons (without key event)
                                layers[layer].buttons[btn].1.active = hit;
                                layers[layer].buttons[btn].1.changed = true;
                            }

                            // Update interaction time for motion
                            navigation_state.update_interaction_time();
                        }
                        TouchEvent::Up(up) => {
                            if !touches.contains_key(&up.seat_slot()) {
                                continue;
                            }
                            let (layer, btn) = *touches.get(&up.seat_slot()).unwrap();
                            
                            // Check if this was a keyboard backlight button
                            let button_action = &layers[layer].buttons[btn].1.action;
                            let is_kbd_backlight_button = cfg.keyboard_brightness_enabled &&
                                matches!(button_action, ButtonAction::Key(Key::IllumUp) | ButtonAction::Key(Key::IllumDown));

                            if !is_kbd_backlight_button {
                                // Extract the button action to avoid borrowing conflict
                                let action = layers[layer].buttons[btn].1.action.clone();
                                let old_active = layers[layer].buttons[btn].1.active;
                                if old_active != false {
                                    layers[layer].buttons[btn].1.active = false;
                                    layers[layer].buttons[btn].1.changed = true;
                                    handle_button_action(&mut uinput, &action, &cfg, false, &mut navigation_state, &mut layers, &mut active_layer, &mut needs_complete_redraw, &original_layers, &mut touches);
                                }
                            } else {
                                // Reset visual state for keyboard backlight buttons
                                layers[layer].buttons[btn].1.active = false;
                                layers[layer].buttons[btn].1.changed = true;
                            }

                            // Update interaction time for release
                            navigation_state.update_interaction_time();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        backlight.update_backlight(&cfg);
    }
}
