use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HyprlandWindow {
    pub address: String,
    pub mapped: bool,
    pub hidden: bool,
    pub at: [i32; 2],
    pub size: [i32; 2],
    #[serde(rename = "workspace")]
    pub workspace: HyprlandWorkspace,
    pub floating: bool,
    pub pseudo: bool,
    pub monitor: i32,
    #[serde(rename = "class")]
    pub class: String,
    pub title: String,
    #[serde(rename = "initialClass")]
    pub initial_class: String,
    #[serde(rename = "initialTitle")]
    pub initial_title: String,
    pub pid: i32,
    pub xwayland: bool,
    pub pinned: bool,
    pub fullscreen: i32,
    #[serde(rename = "fullscreenClient")]
    pub fullscreen_client: i32,
    pub grouped: Vec<String>,
    pub tags: Vec<String>,
    pub swallowing: String,
    #[serde(rename = "focusHistoryID")]
    pub focus_history_id: i32,
    #[serde(rename = "inhibitingIdle")]
    pub inhibiting_idle: bool,
    #[serde(rename = "xdgTag")]
    pub xdg_tag: String,
    #[serde(rename = "xdgDescription")]
    pub xdg_description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HyprlandWorkspace {
    pub id: i32,
    pub name: String,
}

pub struct HyprlandIpc {
    socket_path: String,
    socket2_path: String,
}

// Global cache for the active window info
static CACHED_WINDOW_INFO: std::sync::LazyLock<Arc<Mutex<Option<ActiveWindowInfo>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

static EVENT_LISTENER_STARTED: std::sync::LazyLock<Arc<Mutex<bool>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(false)));

static CACHE_UPDATED: std::sync::LazyLock<Arc<Mutex<bool>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(false)));

impl HyprlandIpc {
    pub fn new() -> Result<Self> {
        // Try to get HYPRLAND_INSTANCE_SIGNATURE from environment first
        if let Ok(signature) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
            let socket_path = format!("/tmp/hypr/{}/.socket.sock", signature);
            let socket2_path = format!("/tmp/hypr/{}/.socket2.sock", signature);
            return Ok(HyprlandIpc { socket_path, socket2_path });
        }

        // If not available, try to find the socket automatically
        // First try /tmp/hypr (traditional location)
        if let Ok(entries) = std::fs::read_dir("/tmp/hypr") {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        let socket_path = entry.path().join(".socket.sock");
                        if socket_path.exists() {
                            let socket2_path = entry.path().join(".socket2.sock");
                            return Ok(HyprlandIpc {
                                socket_path: socket_path.to_string_lossy().to_string(),
                                socket2_path: socket2_path.to_string_lossy().to_string()
                            });
                        }
                    }
                }
            }
        }

        // Then try /run/user/*/hypr/ (user session location)
        if let Ok(run_user_entries) = std::fs::read_dir("/run/user") {
            for user_entry in run_user_entries.flatten() {
                if let Ok(file_type) = user_entry.file_type() {
                    if file_type.is_dir() {
                        let hypr_path = user_entry.path().join("hypr");
                        if hypr_path.exists() {
                            if let Ok(hypr_entries) = std::fs::read_dir(&hypr_path) {
                                for hypr_entry in hypr_entries.flatten() {
                                    if let Ok(hypr_file_type) = hypr_entry.file_type() {
                                        if hypr_file_type.is_dir() {
                                            let socket_path = hypr_entry.path().join(".socket.sock");
                                            if socket_path.exists() {
                                                let path_str = socket_path.to_string_lossy().to_string();
                                                let socket2_path = hypr_entry.path().join(".socket2.sock");
                                                let socket2_path_str = socket2_path.to_string_lossy().to_string();
                                                println!("Found Hyprland socket at: {}", path_str);
                                                return Ok(HyprlandIpc {
                                                    socket_path: path_str,
                                                    socket2_path: socket2_path_str
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(anyhow!("Could not find Hyprland socket. Make sure Hyprland is running."))
    }

    pub fn send_command(&self, command: &str) -> Result<String> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .map_err(|e| anyhow!("Failed to connect to Hyprland socket: {}", e))?;

        stream.write_all(command.as_bytes())
            .map_err(|e| anyhow!("Failed to send command: {}", e))?;

        let mut response = String::new();
        stream.read_to_string(&mut response)
            .map_err(|e| anyhow!("Failed to read response: {}", e))?;

        Ok(response)
    }

    pub fn get_active_window(&self) -> Result<HyprlandWindow> {
        let response = self.send_command("j/activewindow")?;
        let window: HyprlandWindow = serde_json::from_str(&response)
            .map_err(|e| anyhow!("Failed to parse active window response: {}", e))?;
        Ok(window)
    }

    pub fn get_clients(&self) -> Result<Vec<HyprlandWindow>> {
        let response = self.send_command("j/clients")?;
        let clients: Vec<HyprlandWindow> = serde_json::from_str(&response)
            .map_err(|e| anyhow!("Failed to parse clients response: {}", e))?;
        Ok(clients)
    }

    pub fn start_event_listener(&self) -> Result<()> {
        let socket2_path = self.socket2_path.clone();

        thread::spawn(move || {
            if let Err(e) = Self::event_listener_loop(&socket2_path) {
                println!("Hyprland event listener error: {}", e);
            }
        });

        Ok(())
    }

    fn event_listener_loop(socket2_path: &str) -> Result<()> {
        println!("Starting Hyprland event listener on: {}", socket2_path);

        loop {
            match UnixStream::connect(socket2_path) {
                Ok(stream) => {
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        match line {
                            Ok(event_line) => {
                                Self::handle_event(&event_line);
                            }
                            Err(e) => {
                                println!("Error reading from Hyprland event socket: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("Failed to connect to Hyprland event socket: {}", e);
                    thread::sleep(std::time::Duration::from_secs(5));
                }
            }
        }
    }

    fn handle_event(event_line: &str) {
        if event_line.starts_with("activewindow>>") {
            // Parse the activewindow event and update cache
            // Format: activewindow>>CLASS,TITLE
            if let Some(data) = event_line.strip_prefix("activewindow>>") {
                let parts: Vec<&str> = data.splitn(2, ',').collect();
                if parts.len() == 2 {
                    let class = parts[0].to_string();
                    let title = parts[1].to_string();

                    let window_info = ActiveWindowInfo {
                        title: title.clone(),
                        class: class.clone(),
                        initial_title: title.clone(), // We don't have this from events
                        initial_class: class.clone(), // We don't have this from events
                    };

                    if let Ok(mut cache) = CACHED_WINDOW_INFO.lock() {
                        *cache = Some(window_info);
                        println!("Updated cached window: {} - {}", class, title);

                        // Mark cache as updated
                        if let Ok(mut updated) = CACHE_UPDATED.lock() {
                            *updated = true;
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActiveWindowInfo {
    pub title: String,
    pub class: String,
    pub initial_title: String,
    pub initial_class: String,
}

impl ActiveWindowInfo {
    pub fn from_hyprland_window(window: HyprlandWindow) -> Self {
        ActiveWindowInfo {
            title: window.title,
            class: window.class,
            initial_title: window.initial_title,
            initial_class: window.initial_class,
        }
    }

    pub fn get_text_by_button_title(&self, button_title: &str) -> String {
        match button_title {
            "title" => self.title.clone(),
            "class" => self.class.clone(),
            "initialTitle" => self.initial_title.clone(),
            "initialClass" => self.initial_class.clone(),
            _ => self.title.clone(), // default fallback
        }
    }

    pub fn get_app_icon_name(&self) -> String {
        format!("app-{}", self.class)
    }
}

pub fn get_active_window_info() -> Result<ActiveWindowInfo> {
    // Try to create IPC connection - if it fails, Hyprland isn't ready yet
    let ipc = match HyprlandIpc::new() {
        Ok(ipc) => ipc,
        Err(_) => {
            // Hyprland not available - return a default "waiting" state
            return Err(anyhow!("Hyprland not available"));
        }
    };

    // Start event listener if not already started
    {
        let mut started = EVENT_LISTENER_STARTED.lock().unwrap();
        if !*started {
            if let Ok(()) = ipc.start_event_listener() {
                *started = true;
            }
        }
    }

    // Try to get from cache first
    if let Ok(cache) = CACHED_WINDOW_INFO.lock() {
        if let Some(ref cached_info) = *cache {
            return Ok(cached_info.clone());
        }
    }

    // If no cache, get it directly and update cache
    let window = ipc.get_active_window()
        .map_err(|e| anyhow!("Failed to get active window: {}", e))?;
    let window_info = ActiveWindowInfo::from_hyprland_window(window);

    // Update cache
    if let Ok(mut cache) = CACHED_WINDOW_INFO.lock() {
        *cache = Some(window_info.clone());
    }

    Ok(window_info)
}

pub fn check_and_reset_cache_updated() -> bool {
    if let Ok(mut updated) = CACHE_UPDATED.lock() {
        let was_updated = *updated;
        *updated = false;
        was_updated
    } else {
        false
    }
}

pub fn parse_key_combos(action: &str) -> Vec<input_linux::Key> {
    if !action.starts_with("KeyCombos_") {
        return Vec::new();
    }

    let combo_part = &action[10..]; // Remove "KeyCombos_" prefix
    let key_parts: Vec<&str> = combo_part.split('_').collect();

    let mut keys = Vec::new();

    for part in key_parts {
        let key = match part.to_uppercase().as_str() {
            "CTRL" => input_linux::Key::LeftCtrl,
            "SHIFT" => input_linux::Key::LeftShift,
            "ALT" => input_linux::Key::LeftAlt,
            "META" | "CMD" | "SUPER" => input_linux::Key::LeftMeta,
            "A" => input_linux::Key::A,
            "B" => input_linux::Key::B,
            "C" => input_linux::Key::C,
            "D" => input_linux::Key::D,
            "E" => input_linux::Key::E,
            "F" => input_linux::Key::F,
            "G" => input_linux::Key::G,
            "H" => input_linux::Key::H,
            "I" => input_linux::Key::I,
            "J" => input_linux::Key::J,
            "K" => input_linux::Key::K,
            "L" => input_linux::Key::L,
            "M" => input_linux::Key::M,
            "N" => input_linux::Key::N,
            "O" => input_linux::Key::O,
            "P" => input_linux::Key::P,
            "Q" => input_linux::Key::Q,
            "R" => input_linux::Key::R,
            "S" => input_linux::Key::S,
            "T" => input_linux::Key::T,
            "U" => input_linux::Key::U,
            "V" => input_linux::Key::V,
            "W" => input_linux::Key::W,
            "X" => input_linux::Key::X,
            "Y" => input_linux::Key::Y,
            "Z" => input_linux::Key::Z,
            "F1" => input_linux::Key::F1,
            "F2" => input_linux::Key::F2,
            "F3" => input_linux::Key::F3,
            "F4" => input_linux::Key::F4,
            "F5" => input_linux::Key::F5,
            "F6" => input_linux::Key::F6,
            "F7" => input_linux::Key::F7,
            "F8" => input_linux::Key::F8,
            "F9" => input_linux::Key::F9,
            "F10" => input_linux::Key::F10,
            "F11" => input_linux::Key::F11,
            "F12" => input_linux::Key::F12,
            "ENTER" | "RETURN" => input_linux::Key::Enter,
            "ESC" | "ESCAPE" => input_linux::Key::Esc,
            "SPACE" => input_linux::Key::Space,
            "TAB" => input_linux::Key::Tab,
            "BACKSPACE" => input_linux::Key::Backspace,
            "DELETE" => input_linux::Key::Delete,
            "HOME" => input_linux::Key::Home,
            "END" => input_linux::Key::End,
            "PAGEUP" => input_linux::Key::PageUp,
            "PAGEDOWN" => input_linux::Key::PageDown,
            "UP" => input_linux::Key::Up,
            "DOWN" => input_linux::Key::Down,
            "LEFT" => input_linux::Key::Left,
            "RIGHT" => input_linux::Key::Right,
            "1" => input_linux::Key::Num1,
            "2" => input_linux::Key::Num2,
            "3" => input_linux::Key::Num3,
            "4" => input_linux::Key::Num4,
            "5" => input_linux::Key::Num5,
            "6" => input_linux::Key::Num6,
            "7" => input_linux::Key::Num7,
            "8" => input_linux::Key::Num8,
            "9" => input_linux::Key::Num9,
            "0" => input_linux::Key::Num0,
            _ => continue, // Skip unknown keys
        };
        keys.push(key);
    }

    keys
}