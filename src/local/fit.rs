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

/// 一個量化版本所有分檔的大小總和。
pub fn total_size(c: &Choice) -> u64 {
    c.files.iter().fold(0u64, |a, f| a.saturating_add(f.size))
}

/// 預設挑的量化位元數。
const DEFAULT_BITS: u8 = 4;

struct Rated {
    choice: usize,
    size: u64,
    required: u64,
    fit: Fit,
}

/// 預設的量化，不依硬體跳選（相容等級另外標給使用者看）：有 `preferred`（HF 的預設）用它。
///
/// 否則固定 4-bit（低於 4-bit 品質掉太多）：有 `Q4_K_M` 用它，否則最大的 4-bit；
/// 沒有 4-bit 挑高於 4-bit 裡最小的，再沒有就挑最大的。只在 `choices` 為空時回 `None`。
pub fn auto_pick(
    cat: &Catalog,
    meta: &ModelMeta,
    hw: &Hardware,
    preferred: Option<usize>,
) -> Option<Pick> {
    let moe = meta.expert_count > 0;
    let rated: Vec<Rated> = cat
        .choices
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let size = total_size(c);
            let req = required(meta, size, 0);
            Rated {
                choice: i,
                size,
                required: req,
                fit: fit(req, moe, hw),
            }
        })
        .collect();
    let bits = |r: &&Rated| cat.choices[r.choice].quant.bits;
    let four_bit = || rated.iter().filter(|r| bits(r) == Some(DEFAULT_BITS));
    let best = preferred
        .and_then(|i| rated.get(i))
        .or_else(|| {
            four_bit().find(|r| {
                cat.choices[r.choice]
                    .quant
                    .label
                    .eq_ignore_ascii_case("Q4_K_M")
            })
        })
        .or_else(|| four_bit().max_by_key(|r| r.size))
        .or_else(|| {
            rated
                .iter()
                .filter(|r| bits(r).is_some_and(|b| b > DEFAULT_BITS))
                .min_by_key(|r| r.size)
        })
        .or_else(|| rated.iter().max_by_key(|r| r.size))?;
    let (choice, size, level) = (best.choice, best.size, best.fit);

    let main_bits = cat.choices[choice].quant.bits;
    // 加上 MTP 仍 Smooth 才帶；加總後 Smooth 代表主模型本身也是 Smooth
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
            (fit(with, moe, hw) == Fit::Smooth).then_some((i, with))
        });
    Some(Pick {
        choice,
        mtp: mtp.map(|m| m.0),
        fit: level,
        required: mtp.map_or(best.required, |m| m.1),
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
            nvidia_driver: None,
            device: 0,
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
            measured: true,
        }
    }

    fn integrated(ram: u64) -> Hardware {
        Hardware {
            gpu: Some(gpu(GpuKind::Integrated)),
            vram_total: 0,
            vram_free: 0,
            ram_total: ram,
            ram_free: ram,
            measured: true,
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
    fn auto_pick_takes_four_bit_whatever_the_hardware() {
        let c = cat(&[(2, 3 * G), (4, 5 * G), (8, 9 * G)], &[]);
        let m = meta();
        assert_eq!(
            auto_pick(&c, &m, &hw(20 * G, 16 * G), None).unwrap().choice,
            1
        );
        let slow = auto_pick(&c, &m, &hw(3 * G, 16 * G), None).unwrap();
        assert_eq!((slow.choice, slow.fit), (1, Fit::Slow));
        let none = auto_pick(&c, &m, &hw(G / 2, G), None).unwrap();
        assert_eq!((none.choice, none.fit), (1, Fit::NoFit));
        assert!(auto_pick(&Catalog::default(), &m, &hw(6 * G, 16 * G), None).is_none());
    }

    #[test]
    fn auto_pick_prefers_q4_k_m_then_the_next_quant_above_four_bit() {
        let mut c = cat(&[(4, 4 * G), (4, 5 * G), (4, 6 * G)], &[]);
        c.choices[1].quant.label = "Q4_K_M".into();
        let (m, h) = (meta(), hw(20 * G, 16 * G));
        assert_eq!(auto_pick(&c, &m, &h, None).unwrap().choice, 1);
        c.choices[1].quant.label = "Q4_K_S".into();
        assert_eq!(auto_pick(&c, &m, &h, None).unwrap().choice, 2);
        let c = cat(&[(3, 3 * G), (8, 9 * G), (6, 7 * G)], &[]);
        assert_eq!(auto_pick(&c, &m, &h, None).unwrap().choice, 2);
        let c = cat(&[(2, 2 * G), (3, 3 * G)], &[]);
        assert_eq!(auto_pick(&c, &m, &h, None).unwrap().choice, 1);
    }

    /// HF 有預設的量化就用它，不套 4-bit 規則。
    #[test]
    fn auto_pick_follows_the_hf_default() {
        let c = cat(&[(4, 5 * G), (5, 6 * G), (8, 9 * G)], &[]);
        let (m, h) = (meta(), hw(20 * G, 16 * G));
        assert_eq!(auto_pick(&c, &m, &h, Some(1)).unwrap().choice, 1);
        assert_eq!(auto_pick(&c, &m, &h, Some(9)).unwrap().choice, 0);
    }

    #[test]
    fn mtp_only_when_still_smooth() {
        let c = cat(&[(4, 5 * G)], &[(4, G, false), (4, G / 2, true)]);
        let m = meta();
        assert_eq!(
            auto_pick(&c, &m, &hw(7 * G, 16 * G), None).unwrap().mtp,
            Some(0)
        );
        assert_eq!(
            auto_pick(&c, &m, &hw(6 * G + G / 5, 16 * G), None)
                .unwrap()
                .mtp,
            None
        );
    }

    #[test]
    fn kv_and_required_match_hand_computed_values() {
        let m = meta();
        assert_eq!(kv_bytes(&m), 4 * 8192 * 2 * 128 * 2);
        assert_eq!(
            required(&m, 20 * G, G),
            21 * G + kv_bytes(&m) + (512 << 20) + G
        );
    }

    #[test]
    fn moe_auto_pick_offloads() {
        let c = cat(&[(4, 18 * G)], &[]);
        let m = ModelMeta {
            expert_count: 8,
            ..meta()
        };
        let p = auto_pick(&c, &m, &hw(8 * G, 32 * G), None).unwrap();
        assert_eq!(p.fit, Fit::Offload);
    }

    #[test]
    fn mtp_picks_closest_bits_and_none_is_farthest() {
        let mut c = cat(&[(4, 5 * G)], &[(8, G, false), (3, G, false)]);
        let m = meta();
        let h = hw(20 * G, 16 * G);
        assert_eq!(auto_pick(&c, &m, &h, None).unwrap().mtp, Some(1));
        c.mtp.push(MtpChoice {
            quant: Quant {
                label: "x".into(),
                bits: None,
            },
            file: file(G / 4),
            shared: false,
        });
        assert_eq!(auto_pick(&c, &m, &h, None).unwrap().mtp, Some(1));
        c.mtp.clear();
        c.mtp.push(MtpChoice {
            quant: Quant {
                label: "x".into(),
                bits: None,
            },
            file: file(G / 4),
            shared: false,
        });
        c.mtp.push(MtpChoice {
            quant: quant(8),
            file: file(G),
            shared: false,
        });
        assert_eq!(auto_pick(&c, &m, &h, None).unwrap().mtp, Some(1));
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
