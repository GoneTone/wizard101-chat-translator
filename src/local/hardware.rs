//! 硬體偵測：GPU 種類、系統記憶體，以及 `llama-server --list-devices` 的輸出解析。

use std::sync::LazyLock;

use futures_util::FutureExt;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use crate::logs::log;

/// GPU 種類；`None` 供呼叫端需要明確值時使用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuKind {
    Discrete,
    Integrated,
    None,
}

/// 偵測到的 GPU。
#[derive(Clone, Debug, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub vendor: u32,
    /// PCI device ID。
    pub device: u32,
    pub kind: GpuKind,
    /// NVIDIA 驅動版本乘以 100（581.57 → 58157）；其他廠牌或讀不到為 `None`。
    pub nvidia_driver: Option<u32>,
}

/// 硬體快照（位元組）；可用量不含本程式自己載入的模型。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Hardware {
    pub gpu: Option<Gpu>,
    pub vram_total: u64,
    pub vram_free: u64,
    pub ram_total: u64,
    pub ram_free: u64,
    /// 引擎量過 VRAM（`--list-devices` 成功）；沒量過時相容等級未知。量到 0（只有 CPU）也算量過。
    pub measured: bool,
    /// 量測當下本程式自己載入的模型估計佔用；可用量照實記，相容判斷（`effective`）才加回去。
    pub own: u64,
}

/// 列舉 DX12 adapter，取第一個獨顯，沒有就取內顯；列舉失敗回 `None`。
pub fn detect_gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    // 原生後端的列舉是立即完成的 future
    let Some(adapters) = instance
        .enumerate_adapters(wgpu::Backends::DX12)
        .now_or_never()
    else {
        log("[local] gpu detection: adapter enumeration did not complete");
        return None;
    };
    let pick = |wanted: wgpu::DeviceType, kind: GpuKind| {
        adapters.iter().map(|a| a.get_info()).find_map(|info| {
            (info.device_type == wanted).then(|| Gpu {
                nvidia_driver: nvidia_driver(info.vendor, &info.driver),
                name: info.name,
                vendor: info.vendor,
                device: info.device,
                kind,
            })
        })
    };
    let gpu = pick(wgpu::DeviceType::DiscreteGpu, GpuKind::Discrete)
        .or_else(|| pick(wgpu::DeviceType::IntegratedGpu, GpuKind::Integrated));
    if gpu.is_none() {
        log(format!(
            "[local] gpu detection: no usable adapter among {}",
            adapters.len()
        ));
    }
    gpu
}

/// PCI 廠商代碼：NVIDIA。
pub const NVIDIA: u32 = 0x10de;

/// DX12 回報的 Windows 驅動版本（如 `32.0.15.8157`）換成 NVIDIA 版號乘以 100（58157）。
fn nvidia_driver(vendor: u32, version: &str) -> Option<u32> {
    if vendor != NVIDIA {
        return None;
    }
    let parts: Vec<u32> = version
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    // NVIDIA 版號是第三段個位數接第四段（15.8157 → 581.57）；wgpu 讀不到時回 0.0.0.0
    match parts[..] {
        [_, _, third, fourth] if fourth < 10_000 => Some(third % 10 * 10_000 + fourth),
        _ => None,
    }
    .filter(|&d| d > 0)
}

/// 回傳 `(實體記憶體總量, 可用量)`；呼叫失敗回 `(0, 0)`。
pub fn system_memory() -> (u64, u64) {
    // SAFETY: 結構已清零並設好 dwLength，API 只寫入該結構。
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        log("[local] GlobalMemoryStatusEx failed");
        return (0, 0);
    }
    (status.ullTotalPhys, status.ullAvailPhys)
}

/// 解析 `llama-server --list-devices` 的裝置行，回 `(名稱, total, free)` 位元組；其他行忽略。
pub fn parse_list_devices(output: &str) -> Vec<(String, u64, u64)> {
    static LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^\s*\S+:\s+(.+?)\s+\((\d+) MiB, (\d+) MiB free\)\s*$").unwrap()
    });
    output
        .lines()
        .filter_map(|line| {
            let c = LINE.captures(line)?;
            let mib = |i: usize| c[i].parse::<u64>().ok().map(|v| v << 20);
            Some((c[1].to_string(), mib(2)?, mib(3)?))
        })
        .collect()
}

/// 相容判斷用的 `(vram, ram)` 可用量；內顯的共享記憶體只算一次、當成 RAM。
///
/// 本程式自己載入的模型（`own`）算回可用：先補回 VRAM 已用的部分，其餘補 RAM，都不超過總量。
pub fn effective(hw: &Hardware) -> (u64, u64) {
    let integrated = hw
        .gpu
        .as_ref()
        .is_some_and(|g| g.kind == GpuKind::Integrated);
    let (vram_total, vram_free) = if integrated {
        (0, 0)
    } else {
        (hw.vram_total, hw.vram_free)
    };
    let to_vram = hw.own.min(vram_total.saturating_sub(vram_free));
    let to_ram = (hw.own - to_vram).min(hw.ram_total.saturating_sub(hw.ram_free));
    (vram_free + to_vram, hw.ram_free + to_ram)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析裝置行，忽略標頭與括號名稱。
    #[test]
    fn parses_list_devices_lines() {
        let out = "ggml_vulkan: Found 1 Vulkan devices:\nAvailable devices:\n  Vulkan0: NVIDIA GeForce RTX 4060 (8188 MiB, 7000 MiB free)\n  Vulkan1: AMD Radeon(TM) Graphics (512 MiB, 500 MiB free)\n";
        assert_eq!(
            parse_list_devices(out),
            vec![
                ("NVIDIA GeForce RTX 4060".into(), 8188 << 20, 7000 << 20),
                ("AMD Radeon(TM) Graphics".into(), 512 << 20, 500 << 20)
            ]
        );
        assert!(parse_list_devices("Available devices:\n").is_empty());
    }

    #[test]
    fn nvidia_driver_comes_from_the_windows_driver_version() {
        assert_eq!(nvidia_driver(NVIDIA, "32.0.15.8157"), Some(58157));
        assert_eq!(nvidia_driver(NVIDIA, "31.0.15.2857"), Some(52857));
        assert_eq!(nvidia_driver(NVIDIA, "0.0.0.0"), None);
        assert_eq!(nvidia_driver(NVIDIA, "bogus"), None);
        assert_eq!(nvidia_driver(0x1002, "32.0.15.8157"), None);
    }

    /// 內顯的記憶體只算一次、當成 RAM。
    #[test]
    fn integrated_gpu_counts_memory_once_as_ram() {
        let hw = Hardware {
            gpu: Some(Gpu {
                name: "Intel".into(),
                vendor: 0x8086,
                kind: GpuKind::Integrated,
                nvidia_driver: None,
                device: 0,
            }),
            vram_total: 16 << 30,
            vram_free: 10 << 30,
            ram_total: 16 << 30,
            ram_free: 10 << 30,
            measured: true,
            own: 0,
        };
        assert_eq!(effective(&hw), (0, 10 << 30));
    }

    /// 加回自己載入的模型：先補 VRAM 已用的部分，多的補 RAM，兩者都不超過總量。
    #[test]
    fn effective_adds_own_models_back_to_vram_then_ram() {
        const G: u64 = 1 << 30;
        let mut hw = Hardware {
            gpu: None,
            vram_total: 8 * G,
            vram_free: 2 * G,
            ram_total: 16 * G,
            ram_free: 15 * G,
            measured: true,
            own: 10 * G,
        };
        assert_eq!(effective(&hw), (8 * G, 16 * G));
        assert_eq!(
            (hw.vram_free, hw.ram_free),
            (2 * G, 15 * G),
            "kept as measured"
        );
        hw.gpu = Some(Gpu {
            name: "Intel".into(),
            vendor: 0x8086,
            kind: GpuKind::Integrated,
            nvidia_driver: None,
            device: 0,
        });
        (hw.vram_free, hw.ram_free, hw.own) = (2 * G, 10 * G, 3 * G);
        assert_eq!(effective(&hw), (0, 13 * G));
    }
}
