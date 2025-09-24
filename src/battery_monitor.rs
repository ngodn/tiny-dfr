use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BatteryState {
    NotCharging,
    Charging,
    Low,
}

#[derive(Debug, Clone)]
pub struct BatteryInfo {
    pub capacity: u32,
    pub state: BatteryState,
    pub last_updated: Instant,
}

impl BatteryInfo {
    fn new() -> Self {
        BatteryInfo {
            capacity: 100,
            state: BatteryState::NotCharging,
            last_updated: Instant::now(),
        }
    }
}

// Global battery state
static BATTERY_STATE: std::sync::LazyLock<Arc<Mutex<Option<BatteryInfo>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

pub struct BatteryMonitor {
    _handle: thread::JoinHandle<()>,
}

impl BatteryMonitor {
    pub fn new(battery_name: String) -> Self {
        let handle = thread::spawn(move || {
            Self::monitor_loop(&battery_name);
        });

        BatteryMonitor { _handle: handle }
    }

    fn monitor_loop(battery_name: &str) {
        let mut last_update = Instant::now();
        let update_interval = Duration::from_secs(30); // Update every 30 seconds

        loop {
            if last_update.elapsed() >= update_interval {
                let battery_info = Self::read_battery_state(battery_name);

                if let Ok(mut state) = BATTERY_STATE.lock() {
                    *state = Some(battery_info);
                }

                last_update = Instant::now();
            }

            thread::sleep(Duration::from_secs(1));
        }
    }

    fn read_battery_state(battery: &str) -> BatteryInfo {
        let status_path = format!("/sys/class/power_supply/{}/status", battery);
        let status = fs::read_to_string(&status_path)
            .unwrap_or_else(|_| "Unknown".to_string());

        let capacity = Self::read_capacity(battery);

        let state = match status.trim() {
            "Charging" | "Full" => BatteryState::Charging,
            "Discharging" if capacity < 10 => BatteryState::Low,
            _ => BatteryState::NotCharging,
        };

        BatteryInfo {
            capacity,
            state,
            last_updated: Instant::now(),
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn read_capacity(battery: &str) -> u32 {
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
    }

    #[cfg(target_arch = "aarch64")]
    fn read_capacity(battery: &str) -> u32 {
        let capacity_path = format!("/sys/class/power_supply/{}/capacity", battery);
        fs::read_to_string(&capacity_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(100)
    }
}

// Public API
pub fn get_cached_battery_state() -> Option<(u32, BatteryState)> {
    if let Ok(state) = BATTERY_STATE.lock() {
        state.as_ref().map(|info| (info.capacity, info.state))
    } else {
        None
    }
}

pub fn is_battery_data_fresh() -> bool {
    if let Ok(state) = BATTERY_STATE.lock() {
        if let Some(info) = state.as_ref() {
            info.last_updated.elapsed() < Duration::from_secs(60)
        } else {
            false
        }
    } else {
        false
    }
}