//! 硬體偵測：GPU 種類、系統記憶體，以及 `llama-server --list-devices` 的輸出解析。

use std::sync::{LazyLock, Once};

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
    pub kind: GpuKind,
}

/// 硬體快照（位元組）；`vram_*` 為 0 代表引擎還沒量。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Hardware {
    pub gpu: Option<Gpu>,
    pub vram_total: u64,
    pub vram_free: u64,
    pub ram_total: u64,
    pub ram_free: u64,
}

/// 列舉 DX12 adapter，取第一個獨顯，沒有就取內顯；列舉失敗回 `None`。
pub fn detect_gpu() -> Option<Gpu> {
    static LOGGED: Once = Once::new();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    // 原生後端的列舉是立即完成的 future
    let Some(adapters) = instance
        .enumerate_adapters(wgpu::Backends::DX12)
        .now_or_never()
    else {
        LOGGED.call_once(|| log("[local] gpu detection: adapter enumeration did not complete"));
        return None;
    };
    let pick = |wanted: wgpu::DeviceType, kind: GpuKind| {
        adapters.iter().map(|a| a.get_info()).find_map(|info| {
            (info.device_type == wanted).then_some(Gpu {
                name: info.name,
                vendor: info.vendor,
                kind,
            })
        })
    };
    let gpu = pick(wgpu::DeviceType::DiscreteGpu, GpuKind::Discrete)
        .or_else(|| pick(wgpu::DeviceType::IntegratedGpu, GpuKind::Integrated));
    if gpu.is_none() {
        LOGGED.call_once(|| {
            log(format!(
                "[local] gpu detection: no usable adapter among {}",
                adapters.len()
            ))
        });
    }
    gpu
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
pub fn effective(hw: &Hardware) -> (u64, u64) {
    match &hw.gpu {
        Some(g) if g.kind == GpuKind::Integrated => (0, hw.ram_free),
        _ => (hw.vram_free, hw.ram_free),
    }
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

    /// 內顯的記憶體只算一次、當成 RAM。
    #[test]
    fn integrated_gpu_counts_memory_once_as_ram() {
        let hw = Hardware {
            gpu: Some(Gpu {
                name: "Intel".into(),
                vendor: 0x8086,
                kind: GpuKind::Integrated,
            }),
            vram_total: 16 << 30,
            vram_free: 10 << 30,
            ram_total: 16 << 30,
            ram_free: 10 << 30,
        };
        assert_eq!(effective(&hw), (0, 10 << 30));
    }
}
