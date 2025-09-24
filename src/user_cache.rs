use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CachedUserEnvironment {
    pub username: String,
    pub uid: u32,
    pub home_dir: String,
    pub runtime_dir: String,
    pub wayland_display: String,
    pub enhanced_path: String,
    pub last_updated: Instant,
}

impl CachedUserEnvironment {
    fn new() -> Option<Self> {
        if let Some(username) = detect_desktop_user() {
            if let Some(uid) = get_user_id(&username) {
                let home_dir = format!("/home/{}", username);
                let runtime_dir = format!("/run/user/{}", uid);
                let wayland_display = detect_wayland_display(&runtime_dir)
                    .unwrap_or_else(|| "wayland-1".to_string());
                let enhanced_path = format!(
                    "{}/.local/share/omarchy/bin:{}/.local/bin:{}/.config/nvm/versions/node/latest/bin:{}/.local/share/pnpm:/usr/local/sbin:/usr/local/bin:/usr/bin:/bin:/var/lib/flatpak/exports/bin",
                    home_dir, home_dir, home_dir, home_dir
                );

                return Some(CachedUserEnvironment {
                    username,
                    uid,
                    home_dir,
                    runtime_dir,
                    wayland_display,
                    enhanced_path,
                    last_updated: Instant::now(),
                });
            }
        }
        None
    }

    fn is_stale(&self) -> bool {
        self.last_updated.elapsed() > Duration::from_secs(300) // 5 minutes
    }
}

// Global user environment cache
static USER_ENV_CACHE: std::sync::LazyLock<Arc<Mutex<Option<CachedUserEnvironment>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

pub struct UserEnvironmentMonitor {
    _handle: thread::JoinHandle<()>,
}

impl UserEnvironmentMonitor {
    pub fn new() -> Self {
        // Initialize cache immediately
        {
            if let Ok(mut cache) = USER_ENV_CACHE.lock() {
                *cache = CachedUserEnvironment::new();
            }
        }

        let handle = thread::spawn(move || {
            Self::monitor_loop();
        });

        UserEnvironmentMonitor { _handle: handle }
    }

    fn monitor_loop() {
        let mut last_refresh = Instant::now();
        let refresh_interval = Duration::from_secs(60); // Check every minute

        loop {
            let now = Instant::now();

            if now.duration_since(last_refresh) >= refresh_interval {
                // Check if cache needs refresh
                let should_refresh = {
                    if let Ok(cache) = USER_ENV_CACHE.lock() {
                        cache.as_ref().map_or(true, |env| env.is_stale())
                    } else {
                        true
                    }
                };

                if should_refresh {
                    if let Ok(mut cache) = USER_ENV_CACHE.lock() {
                        *cache = CachedUserEnvironment::new();
                    }
                }

                last_refresh = now;
            }

            thread::sleep(Duration::from_secs(10));
        }
    }
}

// Public API
pub fn get_cached_user_environment() -> Option<CachedUserEnvironment> {
    if let Ok(cache) = USER_ENV_CACHE.lock() {
        cache.clone()
    } else {
        None
    }
}

// Move the detection functions here to avoid blocking main thread
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

fn detect_wayland_display(runtime_dir: &str) -> Option<String> {
    // Check for common Wayland display sockets
    let candidates = ["wayland-0", "wayland-1"];

    for candidate in &candidates {
        let socket_path = format!("{}/{}", runtime_dir, candidate);
        if std::path::Path::new(&socket_path).exists() {
            return Some(candidate.to_string());
        }
    }

    // Default fallback
    Some("wayland-1".to_string())
}

// Global monitor instance
static USER_ENV_MONITOR: std::sync::LazyLock<UserEnvironmentMonitor> =
    std::sync::LazyLock::new(|| UserEnvironmentMonitor::new());

pub fn initialize_user_environment_cache() {
    std::sync::LazyLock::force(&USER_ENV_MONITOR);
}