//! PC vitals for the phone's dashboard. Only sampled while the phone is looking (it renews a
//! subscription every few seconds), so an idle companion does no polling at all.

use serde_json::{Value, json};
use sysinfo::{Components, System};

pub struct Stats {
    sys: System,
    components: Components,
}

impl Stats {
    pub fn new() -> anyhow::Result<Self> {
        let mut sys = System::new()?;
        sys.refresh_cpu_usage();
        Ok(Self { sys, components: Components::new_with_refreshed_list()? })
    }

    /// One sample. CPU usage is measured since the previous call, so call it at a steady rate.
    pub fn sample(&mut self) -> Value {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.components.refresh(false);
        json!({
            "cpu": round1(self.sys.global_cpu_usage()),
            "memUsed": self.sys.used_memory(),
            "memTotal": self.sys.total_memory(),
            "cpuTemp": self.cpu_temp().map(round1),
            "gpu": gpu_busy(),
            "uptime": System::uptime().ok(),
        })
    }

    /// The package sensor: k10temp's Tctl on AMD, coretemp's Package on Intel, else the hottest CPU one.
    fn cpu_temp(&self) -> Option<f32> {
        let readings: Vec<(String, f32)> = self
            .components
            .iter()
            .filter_map(|c| Some((c.label().to_lowercase(), c.temperature()?)))
            .filter(|(_, t)| t.is_finite() && *t > 0.0)
            .collect();
        readings
            .iter()
            .find(|(l, _)| l.contains("tctl") || l.contains("package"))
            .or_else(|| readings.iter().filter(|(l, _)| l.contains("cpu") || l.contains("core")).max_by(|a, b| a.1.total_cmp(&b.1)))
            .map(|(_, t)| *t)
    }
}

/// amdgpu (and some others) expose a busy percentage in sysfs; nothing to spawn or link against.
fn gpu_busy() -> Option<u8> {
    let cards = std::fs::read_dir("/sys/class/drm").ok()?;
    cards
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("card") && !e.file_name().to_string_lossy().contains('-'))
        .filter_map(|e| std::fs::read_to_string(e.path().join("device/gpu_busy_percent")).ok())
        .filter_map(|s| s.trim().parse::<u8>().ok())
        .max()
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}
