use anyhow::{anyhow, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const DEFAULT_KEYBOARD_BRIGHTNESS: u32 = 128;
const KEYBOARD_BRIGHTNESS_STEP: u32 = 1466;

pub struct KeyboardBacklightManager {
    kbd_bl_file: Option<File>,
    max_brightness: u32,
    current_brightness: u32,
    brightness_step: u32,
}

impl KeyboardBacklightManager {
    pub fn new() -> KeyboardBacklightManager {
        let (kbd_bl_file, max_brightness, current_brightness) = 
            if let Ok(path) = find_keyboard_backlight() {
                println!("Found keyboard backlight at: {}", path.display());
                
                // Open the brightness file BEFORE dropping privileges
                let brightness_path = path.join("brightness");
                let file = match OpenOptions::new()
                    .write(true)
                    .open(&brightness_path) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        eprintln!("Failed to open keyboard backlight brightness file: {}", e);
                        None
                    }
                };
                
                let max_bl = read_attr(&path, "max_brightness").unwrap_or(255);
                let current_bl = read_attr(&path, "brightness").unwrap_or(0);
                
                println!("Keyboard backlight - Max: {}, Current: {}", max_bl, current_bl);
                
                (file, max_bl, current_bl)
            } else {
                println!("No keyboard backlight device found - keyboard backlight control disabled");
                (None, 255, 0)
            };

        KeyboardBacklightManager {
            kbd_bl_file,
            max_brightness,
            current_brightness,
            brightness_step: KEYBOARD_BRIGHTNESS_STEP,
        }
    }

    pub fn new_with_config(brightness_step: u32) -> KeyboardBacklightManager {
        let mut manager = Self::new();
        manager.brightness_step = brightness_step.max(1); // Ensure step is at least 1
        manager
    }

    pub fn increase_brightness(&mut self) -> bool {
        if self.kbd_bl_file.is_none() {
            return false;
        }
        
        let new_brightness = (self.current_brightness + self.brightness_step)
            .min(self.max_brightness);
        
        if new_brightness != self.current_brightness {
            if self.set_brightness(new_brightness) {
                println!("Keyboard backlight increased to: {}/{}", self.current_brightness, self.max_brightness);
                return true;
            }
        }
        false
    }

    pub fn decrease_brightness(&mut self) -> bool {
        if self.kbd_bl_file.is_none() {
            return false;
        }
        
        let new_brightness = self.current_brightness.saturating_sub(self.brightness_step);
        
        if new_brightness != self.current_brightness {
            if self.set_brightness(new_brightness) {
                println!("Keyboard backlight decreased to: {}/{}", self.current_brightness, self.max_brightness);
                return true;
            }
        }
        false
    }

    pub fn set_brightness(&mut self, brightness: u32) -> bool {
        if let Some(ref mut file) = self.kbd_bl_file {
            let clamped_brightness = brightness.min(self.max_brightness);
            
            match file.write_all(format!("{}\n", clamped_brightness).as_bytes()) {
                Ok(()) => {
                    // Flush to ensure the write is committed immediately
                    match file.flush() {
                        Ok(()) => {
                            self.current_brightness = clamped_brightness;
                            return true;
                        }
                        Err(e) => {
                            eprintln!("Failed to flush keyboard backlight brightness: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to set keyboard backlight brightness: {}", e);
                }
            }
        }
        false
    }

    pub fn current_brightness(&self) -> u32 {
        self.current_brightness
    }

    pub fn max_brightness(&self) -> u32 {
        self.max_brightness
    }

    pub fn is_available(&self) -> bool {
        self.kbd_bl_file.is_some()
    }

    pub fn brightness_percentage(&self) -> f32 {
        if self.max_brightness == 0 {
            0.0
        } else {
            (self.current_brightness as f32 / self.max_brightness as f32) * 100.0
        }
    }

    pub fn update_brightness_step(&mut self, new_step: u32) {
        self.brightness_step = new_step.max(1); // Ensure step is at least 1
    }
}

fn find_keyboard_backlight() -> Result<PathBuf> {
    // Priority 1: T2 Mac specific path (your working solution)
    let t2_path = PathBuf::from("/sys/class/leds/:white:kbd_backlight");
    if t2_path.exists() && t2_path.join("brightness").exists() {
        return Ok(t2_path);
    }
    
    // Priority 2: Common SMC keyboard backlight
    let smc_path = PathBuf::from("/sys/class/leds/smc::kbd_backlight");
    if smc_path.exists() && smc_path.join("brightness").exists() {
        return Ok(smc_path);
    }
    
    // Priority 3: Search for any keyboard backlight LED
    if let Ok(entries) = fs::read_dir("/sys/class/leds/") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if (name.contains("kbd") || name.contains("keyboard")) 
                && entry.path().join("brightness").exists() {
                return Ok(entry.path());
            }
        }
    }
    
    Err(anyhow!("No keyboard backlight device found in /sys/class/leds/"))
}

fn read_attr(path: &Path, attr: &str) -> Option<u32> {
    fs::read_to_string(path.join(attr))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brightness_clamping() {
        let mut manager = KeyboardBacklightManager {
            kbd_bl_file: None,
            max_brightness: 100,
            current_brightness: 50,
            brightness_step: 25,
        };

        // Test normal increase
        assert_eq!(manager.current_brightness, 50);
        
        // Test clamping at max
        manager.current_brightness = 90;
        let new_brightness = (manager.current_brightness + manager.brightness_step)
            .min(manager.max_brightness);
        assert_eq!(new_brightness, 100); // Should clamp to max
    }

    #[test]
    fn test_brightness_percentage() {
        let manager = KeyboardBacklightManager {
            kbd_bl_file: None,
            max_brightness: 200,
            current_brightness: 100,
            brightness_step: 25,
        };

        assert_eq!(manager.brightness_percentage(), 50.0);
    }
}
