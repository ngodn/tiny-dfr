use anyhow::{anyhow, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const DEFAULT_DISPLAY_BRIGHTNESS_STEP_PERCENT: u32 = 5;
const MIN_DISPLAY_BRIGHTNESS: u32 = 1;

pub struct DisplayBacklightManager {
    bl_file: Option<File>,
    bl_path: Option<PathBuf>,
    max_brightness: u32,
    step: u32,
}

impl DisplayBacklightManager {
    pub fn new() -> Self {
        Self::new_with_step_percent(DEFAULT_DISPLAY_BRIGHTNESS_STEP_PERCENT)
    }

    pub fn new_with_step_percent(percent: u32) -> Self {
        match find_display_backlight() {
            Ok(path) => {
                let max = read_attr(&path, "max_brightness").unwrap_or(0);
                let step = step_from_percent(max, percent);
                let brightness_path = path.join("brightness");
                let file = match OpenOptions::new().write(true).open(&brightness_path) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        eprintln!(
                            "Failed to open display backlight brightness file ({}): {}",
                            brightness_path.display(),
                            e
                        );
                        None
                    }
                };
                if file.is_some() {
                    println!(
                        "Display backlight: {} (max={}, step={})",
                        path.display(),
                        max,
                        step
                    );
                }
                Self {
                    bl_file: file,
                    bl_path: Some(path),
                    max_brightness: max,
                    step,
                }
            }
            Err(e) => {
                println!(
                    "Display backlight not found ({}) - tiny-dfr will not handle BrightnessUp/Down",
                    e
                );
                Self {
                    bl_file: None,
                    bl_path: None,
                    max_brightness: 0,
                    step: 1,
                }
            }
        }
    }

    pub fn is_available(&self) -> bool {
        self.bl_file.is_some()
    }

    pub fn update_step_percent(&mut self, percent: u32) {
        self.step = step_from_percent(self.max_brightness, percent);
    }

    pub fn increase_brightness(&mut self) -> bool {
        let Some(current) = self.read_current() else { return false; };
        let new = current.saturating_add(self.step).min(self.max_brightness);
        if new != current {
            self.write_brightness(new)
        } else {
            false
        }
    }

    pub fn decrease_brightness(&mut self) -> bool {
        let Some(current) = self.read_current() else { return false; };
        let floor = MIN_DISPLAY_BRIGHTNESS.min(self.max_brightness);
        let new = current.saturating_sub(self.step).max(floor);
        if new != current {
            self.write_brightness(new)
        } else {
            false
        }
    }

    fn read_current(&self) -> Option<u32> {
        let path = self.bl_path.as_ref()?;
        read_attr(path, "brightness")
    }

    fn write_brightness(&mut self, value: u32) -> bool {
        let Some(file) = self.bl_file.as_mut() else { return false; };
        match file.write_all(format!("{}\n", value).as_bytes()) {
            Ok(()) => match file.flush() {
                Ok(()) => {
                    println!(
                        "Display brightness set to: {}/{}",
                        value, self.max_brightness
                    );
                    true
                }
                Err(e) => {
                    eprintln!("Failed to flush display brightness: {}", e);
                    false
                }
            },
            Err(e) => {
                eprintln!("Failed to write display brightness: {}", e);
                false
            }
        }
    }
}

fn step_from_percent(max: u32, percent: u32) -> u32 {
    let pct = percent.clamp(1, 100);
    ((max.saturating_mul(pct)) / 100).max(1)
}

fn find_display_backlight() -> Result<PathBuf> {
    for entry in fs::read_dir("/sys/class/backlight/")? {
        let entry = entry?;
        if [
            "apple-panel-bl",
            "gmux_backlight",
            "intel_backlight",
            "acpi_video0",
        ]
        .iter()
        .any(|s| entry.file_name().to_string_lossy().contains(s))
        {
            return Ok(entry.path());
        }
    }
    Err(anyhow!("No display backlight device found"))
}

fn read_attr(path: &Path, attr: &str) -> Option<u32> {
    fs::read_to_string(path.join(attr))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
}
