//! 硬體偵測：GPU 種類、系統記憶體、顯示卡的專用記憶體用量，以及 `llama-server --list-devices` 的輸出解析。

use std::sync::LazyLock;

use futures_util::FutureExt;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use super::usage::{self, Usage};
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
    /// 量到 VRAM 了（系統計數器或引擎的 `--list-devices`，見 [`vram`]）；沒量過時相容等級未知。量到 0（只有 CPU）也算量過。
    pub measured: bool,
    /// 算進 VRAM 的卡名，長度即卡數；沒量到或 VRAM 為 0 時為空。
    pub vram_cards: Vec<String>,
    /// 量測當下本程式自己的 llama-server 實際佔用；可用量照實記，相容判斷（`effective`）才加回去。
    pub own: Usage,
}

/// 列舉 DX12 adapter，回傳所有獨顯（列舉順序）；沒有獨顯時只回第一張內顯，都沒有或列舉失敗回空。
pub fn detect_gpus() -> Vec<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    // 原生後端的列舉是立即完成的 future
    let Some(adapters) = instance
        .enumerate_adapters(wgpu::Backends::DX12)
        .now_or_never()
    else {
        log("[local] gpu detection: adapter enumeration did not complete");
        return Vec::new();
    };
    let of = |wanted: wgpu::DeviceType, kind: GpuKind| {
        adapters
            .iter()
            .map(|a| a.get_info())
            .filter(move |info| info.device_type == wanted)
            .map(move |info| Gpu {
                nvidia_driver: nvidia_driver(info.vendor, &info.driver),
                name: info.name,
                vendor: info.vendor,
                device: info.device,
                kind,
            })
    };
    let mut gpus: Vec<Gpu> = of(wgpu::DeviceType::DiscreteGpu, GpuKind::Discrete).collect();
    if gpus.is_empty() {
        gpus.extend(of(wgpu::DeviceType::IntegratedGpu, GpuKind::Integrated).take(1));
    }
    if gpus.is_empty() {
        log(format!(
            "[local] gpu detection: no usable adapter among {}",
            adapters.len()
        ));
    }
    gpus
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

/// 每張 `gpus` 的專用記憶體 `(總量, 可用量)`，與 `gpus` 等長：可用量是總量減去所有程序合計的用量（同工作管理員）；
/// 找不到 adapter 或讀不到計數器的那張為 `None`。
///
/// llama.cpp CUDA 版的 `--list-devices` 在 Windows 回報固定的預算，不扣遊戲等其他程式的佔用，所以改讀系統計數器。
pub fn adapter_memory(gpus: &[Gpu]) -> Vec<Option<(u64, u64)>> {
    // SAFETY: 只建立 DXGI factory 並查詢 adapter 描述，介面的參考計數由 windows crate 管理
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(f) => f,
        Err(e) => {
            log(format!("[local] CreateDXGIFactory1 failed: {e}"));
            return vec![None; gpus.len()];
        }
    };
    let descs: Vec<_> = (0..)
        .map_while(|i| unsafe { factory.EnumAdapters1(i) }.ok())
        .filter_map(|a| unsafe { a.GetDesc1() }.ok())
        .collect();
    let ids: Vec<_> = descs.iter().map(|d| (d.VendorId, d.DeviceId)).collect();
    gpus.iter()
        .zip(pair_adapters(gpus, &ids))
        .map(|(gpu, paired)| {
            let Some(desc) = paired.map(|i| &descs[i]) else {
                log(format!(
                    "[local] no DXGI adapter matches {} ({:04x}:{:04x})",
                    gpu.name, gpu.vendor, gpu.device
                ));
                return None;
            };
            let total = desc.DedicatedVideoMemory as u64;
            let luid = (desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart);
            match usage::adapter_dedicated_usage(luid) {
                Ok(used) => Some((total, total.saturating_sub(used))),
                Err(e) => {
                    log(format!(
                        "[local] reading adapter memory of {} failed: {e}",
                        gpu.name
                    ));
                    None
                }
            }
        })
        .collect()
}

/// 每張 gpu 配對 `adapters`（DXGI 列舉順序的 `(VendorId, DeviceId)`）裡第一個還沒用過、型號相同的索引；
/// 同型號多張才不會讀到同一張。
fn pair_adapters(gpus: &[Gpu], adapters: &[(u32, u32)]) -> Vec<Option<usize>> {
    let mut used = vec![false; adapters.len()];
    gpus.iter()
        .map(|g| {
            let i =
                (0..adapters.len()).find(|&i| !used[i] && adapters[i] == (g.vendor, g.device))?;
            used[i] = true;
            Some(i)
        })
        .collect()
}

/// 算進相容判斷的 VRAM：總量與可用量（位元組），以及算進去的卡名。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Vram {
    pub total: u64,
    pub free: u64,
    pub cards: Vec<String>,
}

/// 加總引擎會用到的卡的 VRAM；`None` 表示還量不到，要等引擎。
///
/// `counters` 與 `gpus` 等長（[`adapter_memory`]）；`devices` 是引擎 `--list-devices` 列出的裝置，沒裝引擎或執行失敗為 `None`。
/// 引擎沒列出任何裝置（CPU 版）時為 0；否則只算名稱對得上的獨顯，全部讀得到系統計數器才用計數器，不然用引擎的數字；
/// 一張都對不上時退回單一裝置。沒裝引擎時算所有獨顯；沒有 GPU 或是內顯不必等引擎：相容判斷不算內顯的 VRAM，當 0。
pub fn vram(
    gpus: &[Gpu],
    counters: &[Option<(u64, u64)>],
    devices: Option<Vec<(String, u64, u64)>>,
) -> Option<Vram> {
    let sum = |sizes: Vec<(u64, u64)>, cards: Vec<String>| Vram {
        total: sizes.iter().map(|s| s.0).sum(),
        free: sizes.iter().map(|s| s.1).sum(),
        cards,
    };
    let counted = |indices: &[usize]| -> Option<Vec<(u64, u64)>> {
        indices
            .iter()
            .map(|&i| counters.get(i).copied().flatten())
            .collect()
    };
    let names = |indices: &[usize]| indices.iter().map(|&i| gpus[i].name.clone()).collect();
    match devices {
        Some(d) if d.is_empty() => Some(Vram::default()),
        Some(d) => {
            let mut paired = Vec::new();
            let mut listed = Vec::new();
            for (name, total, free) in &d {
                let found = (0..gpus.len()).find(|&i| {
                    let g = &gpus[i];
                    g.kind == GpuKind::Discrete && g.name == *name && !paired.contains(&i)
                });
                if let Some(i) = found {
                    paired.push(i);
                    listed.push((*total, *free));
                }
            }
            if paired.is_empty() {
                return match gpus.first().zip(counters.first().copied().flatten()) {
                    Some((g, c)) => Some(sum(vec![c], vec![g.name.clone()])),
                    None => pick_device(d, gpus.first())
                        .map(|(name, total, free)| sum(vec![(total, free)], vec![name])),
                };
            }
            let sizes = counted(&paired).unwrap_or(listed);
            Some(sum(sizes, names(&paired)))
        }
        None => match gpus.first() {
            None => Some(Vram::default()),
            Some(g) if g.kind == GpuKind::Integrated => Some(Vram::default()),
            Some(_) => {
                let all: Vec<usize> = (0..gpus.len()).collect();
                counted(&all).map(|sizes| sum(sizes, names(&all)))
            }
        },
    }
}

/// `--list-devices` 的裝置裡取偵測到的 GPU（名稱相同）；對不上時取記憶體最大的。
///
/// Vulkan 版會把內顯也列出來，內顯回報的是共用的系統記憶體，取最大的會選到它。
pub fn pick_device(
    devices: Vec<(String, u64, u64)>,
    gpu: Option<&Gpu>,
) -> Option<(String, u64, u64)> {
    let named = gpu.and_then(|g| devices.iter().position(|d| d.0 == g.name));
    match named {
        Some(i) => devices.into_iter().nth(i),
        None => devices.into_iter().max_by_key(|d| d.1),
    }
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
/// 本程式自己的 llama-server 佔用（`own`）算回可用，都不超過總量；內顯的 GPU 記憶體算回 RAM。
pub fn effective(hw: &Hardware) -> (u64, u64) {
    let integrated = hw
        .gpu
        .as_ref()
        .is_some_and(|g| g.kind == GpuKind::Integrated);
    let (vram, ram_back) = if integrated {
        (0, hw.own.vram + hw.own.ram)
    } else {
        let vram = (hw.vram_free + hw.own.vram).min(hw.vram_total.max(hw.vram_free));
        (vram, hw.own.ram)
    };
    let ram = (hw.ram_free + ram_back).min(hw.ram_total.max(hw.ram_free));
    (vram, ram)
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

    /// 有獨顯又有內顯（Vulkan 會兩個都列）時取偵測到的那張，不取共用記憶體較大的內顯。
    #[test]
    fn the_detected_gpu_wins_over_a_larger_integrated_one() {
        let devices = || {
            vec![
                (
                    "NVIDIA GeForce RTX 4080 SUPER".to_string(),
                    16 << 30,
                    15 << 30,
                ),
                ("Intel(R) Graphics".to_string(), 36 << 30, 36 << 30),
            ]
        };
        let gpu = Gpu {
            name: "NVIDIA GeForce RTX 4080 SUPER".into(),
            vendor: NVIDIA,
            device: 0x2702,
            kind: GpuKind::Discrete,
            nvidia_driver: None,
        };
        let picked = pick_device(devices(), Some(&gpu)).unwrap();
        assert_eq!(picked.0, gpu.name);
        assert_eq!(pick_device(devices(), None).unwrap().0, "Intel(R) Graphics");
        assert!(pick_device(vec![], Some(&gpu)).is_none());
    }

    /// 引擎列出的裝置優先決定有沒有 VRAM（CPU 版為 0），數字以系統計數器為準；沒裝引擎時獨顯靠計數器，
    /// 沒有 GPU 或內顯不必等引擎。
    #[test]
    fn vram_prefers_the_counters_and_needs_no_engine() {
        const G: u64 = 1 << 30;
        let gpu = |kind| Gpu {
            name: "GPU".into(),
            vendor: NVIDIA,
            device: 1,
            kind,
            nvidia_driver: None,
        };
        let discrete = [gpu(GpuKind::Discrete)];
        let listed = || Some(vec![("GPU".to_string(), 8 * G, 7 * G)]);
        let counters = [Some((8 * G, 3 * G))];
        let sizes = |v: Option<Vram>| v.map(|v| (v.total, v.free));
        assert_eq!(
            sizes(vram(&discrete, &counters, listed())),
            Some((8 * G, 3 * G))
        );
        assert_eq!(
            sizes(vram(&discrete, &[None], listed())),
            Some((8 * G, 7 * G))
        );
        assert_eq!(
            vram(&discrete, &counters, Some(vec![])),
            Some(Vram::default())
        );
        assert_eq!(
            sizes(vram(&discrete, &counters, None)),
            Some((8 * G, 3 * G))
        );
        assert_eq!(vram(&discrete, &[None], None), None);
        assert_eq!(
            vram(&[gpu(GpuKind::Integrated)], &[None], None),
            Some(Vram::default())
        );
        assert_eq!(vram(&[], &[], None), Some(Vram::default()));
    }

    const G: u64 = 1 << 30;

    fn card(name: &str, vendor: u32, device: u32) -> Gpu {
        Gpu {
            name: name.into(),
            vendor,
            device,
            kind: GpuKind::Discrete,
            nvidia_driver: None,
        }
    }

    fn rtx() -> Gpu {
        card("RTX 4060", NVIDIA, 1)
    }

    /// 同型號兩張各自對到不同的 adapter。
    #[test]
    fn identical_cards_pair_with_distinct_adapters() {
        assert_eq!(
            pair_adapters(&[rtx(), rtx()], &[(NVIDIA, 1), (0x8086, 9), (NVIDIA, 1)]),
            vec![Some(0), Some(2)]
        );
        assert_eq!(pair_adapters(&[rtx()], &[]), vec![None]);
    }

    fn two_rtx_listed() -> Option<Vec<(String, u64, u64)>> {
        Some(vec![
            ("RTX 4060".into(), 8 * G, 7 * G),
            ("RTX 4060".into(), 8 * G, 7 * G),
        ])
    }

    /// 兩張同型號：引擎列出兩張、計數器都讀得到，加總計數器。
    #[test]
    fn two_listed_cards_sum_their_counters() {
        let v = vram(
            &[rtx(), rtx()],
            &[Some((8 * G, 5 * G)), Some((8 * G, 6 * G))],
            two_rtx_listed(),
        )
        .unwrap();
        assert_eq!((v.total, v.free), (16 * G, 11 * G));
        assert_eq!(v.cards, vec!["RTX 4060", "RTX 4060"]);
    }

    /// 一張計數器讀不到：整組用引擎數字。
    #[test]
    fn one_missing_counter_uses_engine_numbers_for_all() {
        let v = vram(
            &[rtx(), rtx()],
            &[Some((8 * G, 5 * G)), None],
            two_rtx_listed(),
        )
        .unwrap();
        assert_eq!((v.total, v.free, v.cards.len()), (16 * G, 14 * G, 2));
    }

    /// Vulkan 列出內顯、CUDA 不列 AMD：只算對得上的獨顯。
    #[test]
    fn only_listed_discrete_cards_count() {
        let gpus = [rtx(), card("RX 7600", 0x1002, 2)];
        let devices = Some(vec![
            ("RTX 4060".into(), 8 * G, 7 * G),
            ("Intel(R) Graphics".into(), 16 * G, 16 * G),
        ]);
        let v = vram(
            &gpus,
            &[Some((8 * G, 6 * G)), Some((8 * G, 8 * G))],
            devices,
        )
        .unwrap();
        assert_eq!((v.total, v.free), (8 * G, 6 * G));
        assert_eq!(v.cards, vec!["RTX 4060"]);
    }

    /// 沒有任何名稱對得上：退回第一張的計數器，再不行取最大的裝置。
    #[test]
    fn unmatched_names_fall_back_to_a_single_device() {
        let devices = || {
            Some(vec![
                ("Other".to_string(), 4 * G, 3 * G),
                ("Bigger".to_string(), 6 * G, 5 * G),
            ])
        };
        let v = vram(&[rtx()], &[Some((8 * G, 2 * G))], devices()).unwrap();
        assert_eq!(
            (v.total, v.free, v.cards),
            (8 * G, 2 * G, vec!["RTX 4060".into()])
        );
        let v = vram(&[rtx()], &[None], devices()).unwrap();
        assert_eq!(
            (v.total, v.free, v.cards),
            (6 * G, 5 * G, vec!["Bigger".into()])
        );
    }

    /// 計數器比偵測到的卡多（不等長）時不會 panic，退回最大的裝置。
    #[test]
    fn unmatched_names_without_gpus_ignore_stray_counters() {
        let v = vram(
            &[],
            &[Some((8 * G, 2 * G))],
            Some(vec![("Other".into(), 4 * G, 3 * G)]),
        );
        assert_eq!(
            v,
            Some(Vram {
                total: 4 * G,
                free: 3 * G,
                cards: vec!["Other".into()]
            })
        );
    }

    /// 沒裝引擎：所有獨顯的計數器加總；有一張讀不到就還量不到。
    #[test]
    fn without_engine_all_discrete_cards_sum() {
        let gpus = [rtx(), card("RTX 3060", NVIDIA, 3)];
        let v = vram(&gpus, &[Some((8 * G, 5 * G)), Some((12 * G, 10 * G))], None).unwrap();
        assert_eq!((v.total, v.free), (20 * G, 15 * G));
        assert_eq!(v.cards, vec!["RTX 4060", "RTX 3060"]);
        assert_eq!(vram(&gpus, &[Some((8 * G, 5 * G)), None], None), None);
    }

    /// 偵測到的每張 GPU 都讀得到專用記憶體，可用量不超過總量；沒有 GPU 的機器略過。
    #[test]
    fn reads_the_detected_adapter_memory() {
        let gpus = detect_gpus();
        let memory = adapter_memory(&gpus);
        assert_eq!(memory.len(), gpus.len());
        for m in memory {
            let (total, free) = m.expect("adapter memory");
            assert!(total > 0 && free <= total, "total={total} free={free}");
        }
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
            vram_cards: Vec::new(),
            own: Default::default(),
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
            vram_cards: Vec::new(),
            own: Usage {
                vram: 3 * G,
                ram: 4 * G,
            },
        };
        assert_eq!(effective(&hw), (5 * G, 16 * G), "RAM capped at the total");
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
        (hw.vram_free, hw.ram_free) = (2 * G, 10 * G);
        hw.own = Usage {
            vram: G,
            ram: 2 * G,
        };
        assert_eq!(effective(&hw), (0, 13 * G), "integrated: all back to RAM");
    }
}
