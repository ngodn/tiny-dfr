use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use freedesktop_icons::lookup;

use crate::ICON_SIZE;

// Global icon cache
static ICON_CACHE: std::sync::LazyLock<Arc<Mutex<IconCache>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(IconCache::new())));

#[derive(Debug, Clone)]
struct CacheEntry {
    path: PathBuf,
    last_accessed: Instant,
}

pub struct IconCache {
    cache: HashMap<String, CacheEntry>,
    pending_requests: HashMap<String, Vec<std::sync::mpsc::Sender<Option<PathBuf>>>>,
}

impl IconCache {
    fn new() -> Self {
        IconCache {
            cache: HashMap::new(),
            pending_requests: HashMap::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<PathBuf> {
        if let Some(entry) = self.cache.get_mut(key) {
            entry.last_accessed = Instant::now();
            Some(entry.path.clone())
        } else {
            None
        }
    }

    fn insert(&mut self, key: String, path: PathBuf) {
        let entry = CacheEntry {
            path: path.clone(),
            last_accessed: Instant::now(),
        };
        self.cache.insert(key.clone(), entry);

        // Notify any pending requests
        if let Some(senders) = self.pending_requests.remove(&key) {
            for sender in senders {
                let _ = sender.send(Some(path.clone()));
            }
        }
    }

    fn add_pending_request(&mut self, key: String, sender: std::sync::mpsc::Sender<Option<PathBuf>>) {
        self.pending_requests.entry(key).or_insert_with(Vec::new).push(sender);
    }

    fn cleanup_old_entries(&mut self) {
        let now = Instant::now();
        let max_age = Duration::from_secs(300); // 5 minutes

        self.cache.retain(|_, entry| {
            now.duration_since(entry.last_accessed) < max_age
        });
    }
}

// Background icon loader
pub struct IconLoader {
    _handle: thread::JoinHandle<()>,
    request_sender: std::sync::mpsc::Sender<LoadRequest>,
}

struct LoadRequest {
    cache_key: String,
    name: String,
    theme: Option<String>,
    response_sender: std::sync::mpsc::Sender<Option<PathBuf>>,
}

impl IconLoader {
    pub fn new() -> Self {
        let (request_sender, request_receiver) = std::sync::mpsc::channel::<LoadRequest>();

        let handle = thread::spawn(move || {
            Self::worker_loop(request_receiver);
        });

        IconLoader {
            _handle: handle,
            request_sender,
        }
    }

    fn worker_loop(request_receiver: std::sync::mpsc::Receiver<LoadRequest>) {
        while let Ok(request) = request_receiver.recv() {
            // Try to find the icon path
            let path = Self::find_icon_path(&request.name, request.theme.as_deref());

            // Update cache
            if let Some(ref p) = path {
                if let Ok(mut cache) = ICON_CACHE.lock() {
                    cache.insert(request.cache_key, p.clone());
                }
            }

            // Send response
            let _ = request.response_sender.send(path);
        }
    }

    fn find_icon_path(name: &str, theme: Option<&str>) -> Option<PathBuf> {
        let locations = if let Some(theme) = theme {
            // Freedesktop icons
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
            candidates.into_iter().flatten().collect()
        } else {
            // Standard file icons
            vec![
                PathBuf::from(format!("/etc/tiny-dfr/{name}.svg")),
                PathBuf::from(format!("/etc/tiny-dfr/{name}.png")),
                PathBuf::from(format!("/usr/share/tiny-dfr/{name}.svg")),
                PathBuf::from(format!("/usr/share/tiny-dfr/{name}.png")),
            ]
        };

        // Return first existing file
        for location in locations {
            if location.exists() {
                return Some(location);
            }
        }

        None
    }

    pub fn load_async(&self, name: String, theme: Option<String>) -> std::sync::mpsc::Receiver<Option<PathBuf>> {
        let cache_key = format!("{}:{}", name, theme.as_deref().unwrap_or(""));

        // Check cache first
        {
            if let Ok(mut cache) = ICON_CACHE.lock() {
                if let Some(path) = cache.get(&cache_key) {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    let _ = sender.send(Some(path));
                    return receiver;
                }
            }
        }

        let (response_sender, response_receiver) = std::sync::mpsc::channel();

        // Add to pending requests in cache
        {
            if let Ok(mut cache) = ICON_CACHE.lock() {
                cache.add_pending_request(cache_key.clone(), response_sender.clone());
            }
        }

        let request = LoadRequest {
            cache_key,
            name,
            theme,
            response_sender,
        };

        if let Err(_) = self.request_sender.send(request) {
            // Worker thread died, return empty result
            let (sender, receiver) = std::sync::mpsc::channel();
            let _ = sender.send(None);
            receiver
        } else {
            response_receiver
        }
    }
}

// Global icon loader instance
static ICON_LOADER: std::sync::LazyLock<IconLoader> =
    std::sync::LazyLock::new(|| IconLoader::new());

// Public API
pub fn get_icon_cached(name: String, theme: Option<String>) -> Option<PathBuf> {
    let cache_key = format!("{}:{}", name, theme.as_deref().unwrap_or(""));

    if let Ok(mut cache) = ICON_CACHE.lock() {
        cache.get(&cache_key)
    } else {
        None
    }
}

pub fn load_icon_async(name: String, theme: Option<String>) -> std::sync::mpsc::Receiver<Option<PathBuf>> {
    ICON_LOADER.load_async(name, theme)
}

pub fn cleanup_cache() {
    if let Ok(mut cache) = ICON_CACHE.lock() {
        cache.cleanup_old_entries();
    }
}

// Preload common icons
pub fn preload_common_icons() {
    let common_icons = vec![
        ("back", None::<String>),
        ("settings", None::<String>),
        ("application-default-icon", None::<String>),
        ("plugin-hyprland", None::<String>),
        ("bolt", None::<String>),
        ("brightness_low", None::<String>),
        ("brightness_high", None::<String>),
        ("volume_up", None::<String>),
        ("volume_down", None::<String>),
        ("play_pause", None::<String>),
        ("omarchy", None::<String>),
    ];

    for (name, theme) in common_icons {
        let _ = ICON_LOADER.load_async(name.to_string(), theme.map(|s| s.to_string()));
    }

    // Also preload common app icons
    preload_app_icons();
}

// Preload common application icons
pub fn preload_app_icons() {
    let common_app_classes = vec![
        "code", "Code", "VSCode",
        "firefox", "Firefox",
        "chromium", "Chromium", "chrome", "Chrome",
        "alacritty", "Alacritty",
        "terminal", "Terminal", "gnome-terminal",
        "nautilus", "Nautilus", "Files",
        "discord", "Discord",
        "spotify", "Spotify",
        "steam", "Steam",
        "obs", "OBS",
        "gimp", "GIMP",
        "inkscape", "Inkscape",
        "blender", "Blender",
        "thunderbird", "Thunderbird",
        "libreoffice", "LibreOffice",
        "vlc", "VLC",
    ];

    for class in common_app_classes {
        let icon_name = format!("app-{}", class.to_lowercase());
        let _ = ICON_LOADER.load_async(icon_name, None);
    }
}

// Background preloader that can be started after initial setup
pub fn start_background_preloader() {
    thread::spawn(move || {
        // Wait a bit for the system to settle
        thread::sleep(Duration::from_secs(5));

        // Preload additional icons that might be used
        preload_extended_icons();
    });
}

fn preload_extended_icons() {
    let extended_icons = vec![
        ("mic_off", None::<String>),
        ("mic_on", None::<String>),
        ("backlight_low", None::<String>),
        ("backlight_high", None::<String>),
        ("fast_rewind", None::<String>),
        ("fast_forward", None::<String>),
        ("apps", None::<String>),
        ("terminal", None::<String>),
        ("color_picker", None::<String>),
        ("command", None::<String>),
        ("screenrecord", None::<String>),
        ("screenshot", None::<String>),
    ];

    for (name, theme) in extended_icons {
        let _ = ICON_LOADER.load_async(name.to_string(), theme.map(|s| s.to_string()));
        // Small delay to avoid overwhelming the system
        thread::sleep(Duration::from_millis(100));
    }
}