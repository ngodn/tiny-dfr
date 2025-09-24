use chrono::{Local, Timelike};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SystemState {
    pub current_minute: u32,
    pub last_updated: Instant,
    pub cache_cleanup_due: bool,
}

impl SystemState {
    fn new() -> Self {
        let now = Local::now();
        SystemState {
            current_minute: now.minute(),
            last_updated: Instant::now(),
            cache_cleanup_due: false,
        }
    }
}

// Global system state
static SYSTEM_STATE: std::sync::LazyLock<Arc<Mutex<SystemState>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(SystemState::new())));

pub struct SystemMonitor {
    _handle: thread::JoinHandle<()>,
}

impl SystemMonitor {
    pub fn new() -> Self {
        let handle = thread::spawn(move || {
            Self::monitor_loop();
        });

        SystemMonitor { _handle: handle }
    }

    fn monitor_loop() {
        let mut last_minute_check = Instant::now();
        let mut last_cache_cleanup = Instant::now();

        let minute_check_interval = Duration::from_secs(5); // Check every 5 seconds for minute changes
        let cache_cleanup_interval = Duration::from_secs(300); // Cleanup every 5 minutes

        loop {
            let now = Instant::now();

            // Check for minute changes
            if now.duration_since(last_minute_check) >= minute_check_interval {
                let current_time = Local::now();
                let current_minute = current_time.minute();

                if let Ok(mut state) = SYSTEM_STATE.lock() {
                    if state.current_minute != current_minute {
                        state.current_minute = current_minute;
                        state.last_updated = now;

                        // Signal that UI components should update
                        // (This will be checked by the main loop)
                    }
                }

                last_minute_check = now;
            }

            // Schedule cache cleanup
            if now.duration_since(last_cache_cleanup) >= cache_cleanup_interval {
                if let Ok(mut state) = SYSTEM_STATE.lock() {
                    state.cache_cleanup_due = true;
                }
                last_cache_cleanup = now;
            }

            thread::sleep(Duration::from_secs(1));
        }
    }
}

// Public API
pub fn get_current_minute() -> u32 {
    if let Ok(state) = SYSTEM_STATE.lock() {
        state.current_minute
    } else {
        Local::now().minute()
    }
}

pub fn has_minute_changed(last_known_minute: u32) -> bool {
    get_current_minute() != last_known_minute
}

pub fn should_cleanup_cache() -> bool {
    if let Ok(mut state) = SYSTEM_STATE.lock() {
        if state.cache_cleanup_due {
            state.cache_cleanup_due = false;
            true
        } else {
            false
        }
    } else {
        false
    }
}

pub fn get_system_state() -> SystemState {
    if let Ok(state) = SYSTEM_STATE.lock() {
        state.clone()
    } else {
        SystemState::new()
    }
}