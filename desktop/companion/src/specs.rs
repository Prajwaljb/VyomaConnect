//! What this PC is, for the phone's PC details page: model, system, processor, graphics, memory,
//! storage and screens. Read once per request (the phone asks on every link and keeps the answer),
//! from sysinfo and sysfs only, so no tool to install. Never a serial number or machine id.

use serde_json::{Value, json};
use std::path::Path;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Product, RefreshKind, System};

pub fn read() -> Value {
    let sys = System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram())).ok();
    let cpu = sys.as_ref().and_then(|s| s.cpus().first()).map(|c| tidy_cpu(c.brand())).filter(|b| !b.is_empty());
    let threads = sys.as_ref().map(|s| s.cpus().len()).filter(|&n| n > 0);
    let gpus = gpus();
    let (disk_total, disk_free) = home_disk().unzip();
    let session = std::env::var("XDG_SESSION_TYPE").ok().map(|s| match s.as_str() {
        "wayland" => "Wayland".to_owned(),
        "x11" => "X11".to_owned(),
        _ => s,
    });
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok().map(|d| if d.split(':').any(|p| p == "KDE") { "KDE Plasma".to_owned() } else { d.replace(':', " ") });
    json!({
        "model": model(),
        "os": pretty_os().or_else(|| System::name().ok()),
        "kernel": System::kernel_version().ok(),
        "desktop": desktop,
        "session": session,
        "cpu": cpu,
        "cores": System::physical_core_count().ok().filter(|&n| n > 0),
        "threads": threads,
        "gpus": gpus,
        "memTotal": sys.as_ref().map(|s| s.total_memory()).filter(|&m| m > 0),
        "diskTotal": disk_total,
        "diskFree": disk_free,
        "displays": displays(),
    })
}

/// The maker and model, unless the firmware left a placeholder there.
fn model() -> Option<String> {
    let junk = |s: &str| {
        let l = s.to_ascii_lowercase();
        l.is_empty() || l.contains("o.e.m") || l.contains("default string") || l.contains("not specified") || l.contains("system product") || l.contains("system manufacturer")
    };
    let name = Product::name().ok().map(|s| s.trim().to_owned()).filter(|s| !junk(s))?;
    let vendor = Product::vendor_name().ok().map(|s| s.trim().to_owned()).filter(|s| !junk(s));
    Some(match vendor {
        // Lenovo names its models in `product_version` and puts a part number in `product_name`.
        Some(v) if v.eq_ignore_ascii_case("lenovo") => {
            format!("Lenovo {}", Product::version().ok().map(|s| s.trim().to_owned()).filter(|s| !junk(s)).unwrap_or(name))
        }
        Some(v) if !name.to_ascii_lowercase().starts_with(&v.to_ascii_lowercase()) => format!("{} {name}", short_vendor(&v)),
        _ => name,
    })
}

fn short_vendor(v: &str) -> &str {
    match v {
        "ASUSTeK COMPUTER INC." => "ASUS",
        "Micro-Star International Co., Ltd." => "MSI",
        "Hewlett-Packard" | "HP" => "HP",
        "Dell Inc." => "Dell",
        "Acer" => "Acer",
        "LENOVO" => "Lenovo",
        _ => v.trim_end_matches(" Inc.").trim_end_matches(" Corporation"),
    }
}

/// "AMD Ryzen 7 7840HS w/ Radeon 780M Graphics" → "AMD Ryzen 7 7840HS"; "(R)", "(TM)" and the
/// clock speed go too.
fn tidy_cpu(brand: &str) -> String {
    let mut s = brand.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    for cut in [" w/ ", " with ", " @ "] {
        if let Some(i) = s.find(cut) {
            s.truncate(i);
        }
    }
    s.replace(" CPU", "").replace(" Processor", "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The distribution's own name for itself ("Fedora Linux 43"), not sysinfo's "Linux (Fedora Linux 43)".
fn pretty_os() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).ok()?;
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_owned()).filter(|v| !v.is_empty());
    field("PRETTY_NAME").or_else(|| field("NAME"))
}

/// Display controllers (PCI class 03), named from the PCI id list. Only sysfs's cached ids are
/// read: sysinfo's own listing loads NVML and Vulkan, which wake a sleeping NVIDIA GPU.
fn gpus() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/sys/bus/pci/devices") else { return Vec::new() };
    let read = |p: &Path, f: &str| std::fs::read_to_string(p.join(f)).ok().and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok());
    let mut ids: Vec<(u32, u32)> = dir
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if read(&p, "class")? >> 16 != 0x03 {
                return None;
            }
            Some((read(&p, "vendor")?, read(&p, "device")?))
        })
        .collect();
    // Integrated (Intel, AMD) before discrete, as the PC lists them.
    ids.sort_by_key(|&(v, _)| v == 0x10de);
    ids.dedup();
    let names = std::fs::read_to_string("/usr/share/hwdata/pci.ids").or_else(|_| std::fs::read_to_string("/usr/share/misc/pci.ids")).unwrap_or_default();
    dedupe(ids.into_iter().filter_map(|(v, d)| pci_name(&names, v, d)).collect())
}

/// "AMD Radeon 780M" for 1002:15bf. pci.ids has a vendor at the line's start and its devices
/// indented by one tab under it.
fn pci_name(ids: &str, vendor: u32, device: u32) -> Option<String> {
    let v = format!("{vendor:04x}  ");
    let d = format!("\t{device:04x}  ");
    let mut lines = ids.lines().skip_while(|l| !l.starts_with(&v));
    let vendor_name = lines.next()?[v.len()..].trim().to_owned();
    let model = lines.take_while(|l| l.starts_with('\t') || l.starts_with('#') || l.is_empty()).find_map(|l| l.strip_prefix(&d))?.trim().to_owned();
    let model = tidy_gpu(&model);
    let brand = match vendor {
        0x8086 => "Intel",
        0x1002 => "AMD",
        0x10de => "NVIDIA",
        _ => vendor_name.split_whitespace().next().unwrap_or(""),
    };
    Some(if brand.is_empty() || model.starts_with(brand) { model } else { format!("{brand} {model}") })
}

/// pci.ids names read "Phoenix1 [Radeon 740M / 760M / 780M]": the bracketed marketing name is the one people know.
fn tidy_gpu(model: &str) -> String {
    match (model.rfind('['), model.rfind(']')) {
        (Some(a), Some(b)) if b > a + 1 => model[a + 1..b].trim().to_owned(),
        _ => model.trim().to_owned(),
    }
}

fn dedupe(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// The size and free space of the disk the user's home is on.
fn home_disk() -> Option<(u64, u64)> {
    let home = std::env::var_os("HOME")?;
    let home = Path::new(&home);
    let disks = Disks::new_with_refreshed_list().ok()?;
    disks
        .iter()
        .filter(|d| home.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| (d.total_space(), d.available_space()))
}

/// Connected screens and their native size (the first mode listed is the preferred one).
fn displays() -> Vec<Value> {
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut out: Vec<(bool, String, Value)> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // card1-eDP-1: a connector, not the card itself.
            let connector = name.split_once('-')?.1.to_owned();
            let path = e.path();
            if std::fs::read_to_string(path.join("status")).ok()?.trim() != "connected" {
                return None;
            }
            let mode = std::fs::read_to_string(path.join("modes")).ok()?.lines().next()?.to_owned();
            let (w, h) = mode.split_once('x')?;
            let (w, h) = (w.parse::<u32>().ok()?, h.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().ok()?);
            let internal = ["eDP", "LVDS", "DSI"].iter().any(|p| connector.starts_with(p));
            Some((internal, connector, json!({ "w": w, "h": h, "internal": internal })))
        })
        .collect();
    // Built-in first, then by connector name, so the order is stable.
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.into_iter().map(|(_, _, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidies_names() {
        assert_eq!(tidy_cpu("AMD Ryzen 7 7840HS w/ Radeon 780M Graphics"), "AMD Ryzen 7 7840HS");
        assert_eq!(tidy_cpu("Intel(R) Core(TM) i7-1165G7 @ 2.80GHz"), "Intel Core i7-1165G7");
        assert_eq!(tidy_gpu("Phoenix1 [Radeon 740M / 760M / 780M]"), "Radeon 740M / 760M / 780M");
        assert_eq!(tidy_gpu("GA107M [GeForce RTX 3050 Mobile]"), "GeForce RTX 3050 Mobile");
        let ids = "1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t15bf  Phoenix1 [Radeon 780M]\n10de  NVIDIA Corporation\n\t28a1  AD107M [GeForce RTX 4050 Max-Q / Mobile]\n";
        assert_eq!(pci_name(ids, 0x1002, 0x15bf).as_deref(), Some("AMD Radeon 780M"));
        assert_eq!(pci_name(ids, 0x10de, 0x28a1).as_deref(), Some("NVIDIA GeForce RTX 4050 Max-Q / Mobile"));
        assert_eq!(pci_name(ids, 0x8086, 0x1234), None);
    }

    #[test]
    fn reads_this_pc() {
        let v = read();
        assert!(v["os"].is_string() && v["cpu"].is_string() && v["memTotal"].is_u64());
    }
}
