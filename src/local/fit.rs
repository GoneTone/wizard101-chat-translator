//! 記憶體需求估算、相容等級與自動挑選量化。

use super::gguf::ModelMeta;
use super::hardware::{GpuKind, Hardware, effective};
use super::hf::{Catalog, Choice};

/// 估算 KV cache 用的上下文長度。
pub const CONTEXT_TOKENS: u64 = 8192;
const OVERHEAD: u64 = 512 << 20;
const RAM_RESERVE: u64 = 2 << 30;

/// KV cache 位元組數（f16；SWA 不扣，寧可高估）。
pub fn kv_bytes(meta: &ModelMeta) -> u64 {
    [
        CONTEXT_TOKENS,
        meta.head_count_kv as u64,
        meta.key_length as u64 + meta.value_length as u64,
        2,
    ]
    .iter()
    .fold(meta.block_count as u64, |a, &b| a.saturating_mul(b))
}

/// 載入主模型（與選用的 MTP）預估需要的位元組數。
pub fn required(meta: &ModelMeta, main_bytes: u64, mtp_bytes: u64) -> u64 {
    main_bytes
        .saturating_add(mtp_bytes)
        .saturating_add(kv_bytes(meta))
        .saturating_add(OVERHEAD)
        .saturating_add(main_bytes / 20)
}

/// 相容等級，越大越好；`Offload` 是 MoE 專家放在 RAM。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    NoFit,
    Slow,
    Offload,
    Smooth,
}

/// 依可用記憶體判斷等級；內顯 VRAM 為 0，故永不 `Smooth`。
pub fn fit(required: u64, moe: bool, hw: &Hardware) -> Fit {
    let (vram, ram) = effective(hw);
    if required <= vram {
        Fit::Smooth
    } else if required <= vram.saturating_add(ram.saturating_sub(RAM_RESERVE)) {
        if moe { Fit::Offload } else { Fit::Slow }
    } else {
        Fit::NoFit
    }
}

/// 預估記憶體要顯示的種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Vram,
    VramPlusRam,
    Ram,
}

/// 內顯為 RAM、MoE 為 VRAM＋RAM、其餘為 VRAM。
pub fn memory_kind(moe: bool, hw: &Hardware) -> MemoryKind {
    if hw
        .gpu
        .as_ref()
        .is_some_and(|g| g.kind == GpuKind::Integrated)
    {
        MemoryKind::Ram
    } else if moe {
        MemoryKind::VramPlusRam
    } else {
        MemoryKind::Vram
    }
}

/// 自動挑選結果；`choice`／`mtp` 為 `Catalog` 內的索引。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub choice: usize,
    pub mtp: Option<usize>,
    pub fit: Fit,
    pub required: u64,
}

fn total_size(c: &Choice) -> u64 {
    c.files.iter().fold(0u64, |a, f| a.saturating_add(f.size))
}

/// 最大的 Smooth；沒有則 Slow／Offload 裡最小；全 NoFit 挑最小。只在 `choices` 為空時回 `None`。
pub fn auto_pick(cat: &Catalog, meta: &ModelMeta, hw: &Hardware) -> Option<Pick> {
    let moe = meta.expert_count > 0;
    let rated: Vec<(usize, u64, u64, Fit)> = cat
        .choices
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let size = total_size(c);
            let req = required(meta, size, 0);
            (i, size, req, fit(req, moe, hw))
        })
        .collect();
    let best = rated
        .iter()
        .filter(|r| r.3 == Fit::Smooth)
        .max_by_key(|r| r.1)
        .or_else(|| {
            rated
                .iter()
                .filter(|r| matches!(r.3, Fit::Slow | Fit::Offload))
                .min_by_key(|r| r.1)
        })
        .or_else(|| rated.iter().min_by_key(|r| r.1))?;
    let (choice, size, req, level) = *best;

    let main_bits = cat.choices[choice].quant.bits;
    let mtp = cat
        .mtp
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.shared)
        .min_by_key(|(_, m)| {
            let dist = match (main_bits, m.quant.bits) {
                (Some(a), Some(b)) => a.abs_diff(b) as u32,
                _ => u32::MAX,
            };
            (dist, m.file.size)
        })
        .and_then(|(i, m)| {
            let with = required(meta, size, m.file.size);
            (level == Fit::Smooth && fit(with, moe, hw) == Fit::Smooth).then_some((i, with))
        });
    Some(Pick {
        choice,
        mtp: mtp.map(|m| m.0),
        fit: level,
        required: mtp.map_or(req, |m| m.1),
    })
}

/// 多個模型同時載入：所需量加總，任一為 MoE 則以 MoE 判；空清單視為 `Smooth`。
pub fn combined_fit(requirements: &[(u64, bool)], hw: &Hardware) -> Fit {
    let total = requirements.iter().fold(0u64, |a, r| a.saturating_add(r.0));
    fit(total, requirements.iter().any(|r| r.1), hw)
}

#[cfg(test)]
mod tests {
    use super::super::hardware::Gpu;
    use super::super::hf::{MtpChoice, Quant, RepoFile};
    use super::*;

    const G: u64 = 1 << 30;

    fn gpu(kind: GpuKind) -> Gpu {
        Gpu {
            name: "t".into(),
            vendor: 0,
            kind,
        }
    }

    fn hw(vram: u64, ram: u64) -> Hardware {
        Hardware {
            gpu: Some(gpu(GpuKind::Discrete)),
            vram_total: vram,
            vram_free: vram,
            ram_total: ram,
            ram_free: ram,
        }
    }

    fn integrated(ram: u64) -> Hardware {
        Hardware {
            gpu: Some(gpu(GpuKind::Integrated)),
            vram_total: 0,
            vram_free: 0,
            ram_total: ram,
            ram_free: ram,
        }
    }

    /// 小 KV（約 16 MiB），讓估算接近檔案大小。
    fn meta() -> ModelMeta {
        ModelMeta {
            block_count: 4,
            head_count_kv: 2,
            key_length: 64,
            value_length: 64,
            ..Default::default()
        }
    }

    fn file(size: u64) -> RepoFile {
        RepoFile {
            path: "m.gguf".into(),
            size,
            sha256: None,
        }
    }

    fn quant(bits: u8) -> Quant {
        Quant {
            label: format!("Q{bits}"),
            bits: Some(bits),
        }
    }

    fn cat(sizes: &[(u8, u64)], mtp: &[(u8, u64, bool)]) -> Catalog {
        Catalog {
            choices: sizes
                .iter()
                .map(|&(b, s)| Choice {
                    quant: quant(b),
                    files: vec![file(s)],
                })
                .collect(),
            mtp: mtp
                .iter()
                .map(|&(b, s, shared)| MtpChoice {
                    quant: quant(b),
                    file: file(s),
                    shared,
                })
                .collect(),
        }
    }

    #[test]
    fn dense_model_tiers() {
        assert_eq!(fit(5 * G, false, &hw(6 * G, 16 * G)), Fit::Smooth);
        assert_eq!(fit(7 * G, false, &hw(6 * G, 16 * G)), Fit::Slow);
        assert_eq!(fit(30 * G, false, &hw(6 * G, 16 * G)), Fit::NoFit);
    }

    #[test]
    fn moe_spilling_to_ram_is_offload() {
        assert_eq!(fit(18 * G, true, &hw(8 * G, 32 * G)), Fit::Offload);
    }

    #[test]
    fn integrated_gpu_never_smooth() {
        assert_eq!(fit(G, false, &integrated(16 * G)), Fit::Slow);
    }

    #[test]
    fn memory_kind_by_hardware() {
        assert_eq!(memory_kind(false, &integrated(16 * G)), MemoryKind::Ram);
        assert_eq!(
            memory_kind(true, &hw(8 * G, 16 * G)),
            MemoryKind::VramPlusRam
        );
        assert_eq!(memory_kind(false, &hw(8 * G, 16 * G)), MemoryKind::Vram);
    }

    #[test]
    fn auto_pick_takes_largest_smooth_then_smallest_slow() {
        let c = cat(&[(2, 3 * G), (4, 5 * G), (8, 9 * G)], &[]);
        let m = meta();
        assert_eq!(auto_pick(&c, &m, &hw(6 * G, 16 * G)).unwrap().choice, 1);
        let slow = auto_pick(&c, &m, &hw(3 * G, 16 * G)).unwrap();
        assert_eq!((slow.choice, slow.fit), (0, Fit::Slow));
        let none = auto_pick(&c, &m, &hw(G / 2, G)).unwrap();
        assert_eq!((none.choice, none.fit), (0, Fit::NoFit));
        assert!(auto_pick(&Catalog::default(), &m, &hw(6 * G, 16 * G)).is_none());
    }

    #[test]
    fn mtp_only_when_still_smooth() {
        let c = cat(&[(4, 5 * G)], &[(4, G, false), (4, G / 2, true)]);
        let m = meta();
        assert_eq!(auto_pick(&c, &m, &hw(7 * G, 16 * G)).unwrap().mtp, Some(0));
        assert_eq!(
            auto_pick(&c, &m, &hw(6 * G + G / 5, 16 * G)).unwrap().mtp,
            None
        );
    }

    #[test]
    fn combined_sums_requirements() {
        assert_eq!(
            combined_fit(&[(5 * G, false), (5 * G, false)], &hw(8 * G, 3 * G)),
            Fit::NoFit
        );
        assert_eq!(combined_fit(&[], &hw(G, G)), Fit::Smooth);
    }
}
