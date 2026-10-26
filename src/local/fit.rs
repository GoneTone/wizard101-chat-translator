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

/// 預設的量化，不依硬體跳選（相容等級另外標給使用者看）：與 HF 模型頁「Use this model」的預設一致。
///
/// 依檔案路徑順序找 `UD-Q4_K*`、`Q4_K_M`、`Q4_K*`；都沒有時固定 4-bit（低於 4-bit 品質掉太多）：
/// 最大的 4-bit，沒有就挑高於 4-bit 裡最小的，再沒有就挑最大的。只在 `choices` 為空時回 `None`。
pub fn auto_pick(cat: &Catalog, meta: &ModelMeta, hw: &Hardware) -> Option<Pick> {
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
    let mut by_path: Vec<&Rated> = rated.iter().collect();
    by_path.sort_by_key(|r| cat.choices[r.choice].files.first().map(|f| f.path.as_str()));
    let label = |r: &&&Rated| cat.choices[r.choice].quant.label.to_ascii_uppercase();
    // HF 頁面的規則（huggingface.co 前端挑 `{{QUANT_TAG}}` 預設值的順序）
    let hf_page = by_path
        .iter()
        .find(|r| label(r).starts_with("UD-Q4_K"))
        .or_else(|| by_path.iter().find(|r| label(r) == "Q4_K_M"))
        .or_else(|| by_path.iter().find(|r| label(r).starts_with("Q4_K")))
        .copied();
    let best = hf_page
        .or_else(|| four_bit().max_by_key(|r| r.size))
        .or_else(|| {
            rated
                .iter()
                .filter(|r| bits(r).is_some_and(|b| b > DEFAULT_BITS))
                .min_by_key(|r| r.size)
        })
        .or_else(|| rated.iter().max_by_key(|r| r.size))?;
    let (choice, size, level) = (best.choice, best.size, best.fit);

    let mtp = pick_mtp(cat, choice);
    let required = match mtp {
        Some(m) => required(meta, size, cat.mtp[m].file.size),
        None => best.required,
    };
    Some(Pick {
        choice,
        mtp,
        fit: mtp.map_or(level, |_| fit(required, moe, hw)),
        required,
    })
}

/// 有 MTP 就帶，照 llama.cpp `-hf` 找 MTP 檔的規則挑（`common/download.cpp` 的 `find_best_sibling`）。
///
/// 只看主模型資料夾或其上層裡的，取資料夾最深的，再取檔名含 `-{量化}.` 的，再取位元數最接近的；
/// 同分取路徑在前的。
fn pick_mtp(cat: &Catalog, choice: usize) -> Option<usize> {
    let main = &cat.choices[choice];
    let dirs = |path: &str| -> Vec<String> {
        let mut parts: Vec<String> = path.split('/').map(str::to_owned).collect();
        parts.pop();
        parts
    };
    let model_dir = dirs(&main.files.first()?.path);
    let tag = format!("-{}.", main.quant.label.to_ascii_uppercase());
    let bits = |b: Option<u8>| i32::from(b.unwrap_or(0));
    let mut best: Option<(usize, usize, bool, i32)> = None;
    for (i, m) in cat.mtp.iter().enumerate() {
        let dir = dirs(&m.file.path);
        if !model_dir.starts_with(&dir) {
            continue;
        }
        let exact = m.file.path.to_ascii_uppercase().contains(&tag);
        let diff = (bits(m.quant.bits) - bits(main.quant.bits)).abs();
        let better = match best {
            None => true,
            Some((_, depth, best_exact, best_diff)) => {
                dir.len() > depth
                    || (dir.len() == depth && exact && !best_exact)
                    || (dir.len() == depth && exact == best_exact && diff < best_diff)
            }
        };
        if better {
            best = Some((i, dir.len(), exact, diff));
        }
    }
    best.map(|b| b.0)
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
        assert_eq!(auto_pick(&c, &m, &hw(20 * G, 16 * G)).unwrap().choice, 1);
        let slow = auto_pick(&c, &m, &hw(3 * G, 16 * G)).unwrap();
        assert_eq!((slow.choice, slow.fit), (1, Fit::Slow));
        let none = auto_pick(&c, &m, &hw(G / 2, G)).unwrap();
        assert_eq!((none.choice, none.fit), (1, Fit::NoFit));
        assert!(auto_pick(&Catalog::default(), &m, &hw(6 * G, 16 * G)).is_none());
    }

    #[test]
    fn auto_pick_follows_the_hf_page_then_the_next_quant_above_four_bit() {
        let mut c = cat(&[(4, 4 * G), (4, 5 * G), (4, 6 * G), (4, 7 * G)], &[]);
        let names = [
            "m-UD-Q4_K_XL.gguf",
            "m-Q4_K_M.gguf",
            "m-UD-Q4_K_M.gguf",
            "m-Q4_0.gguf",
        ];
        for (choice, name) in c.choices.iter_mut().zip(names) {
            choice.quant.label = name[2..name.len() - 5].into();
            choice.files[0].path = name.into();
        }
        let (m, h) = (meta(), hw(20 * G, 16 * G));
        assert_eq!(
            auto_pick(&c, &m, &h).unwrap().choice,
            2,
            "UD-Q4_K by path order"
        );
        c.choices[2].quant.label = "IQ4_XS".into();
        assert_eq!(auto_pick(&c, &m, &h).unwrap().choice, 0);
        c.choices[0].quant.label = "IQ4_NL".into();
        assert_eq!(auto_pick(&c, &m, &h).unwrap().choice, 1, "then Q4_K_M");
        c.choices[1].quant.label = "Q4_K_S".into();
        assert_eq!(auto_pick(&c, &m, &h).unwrap().choice, 1, "then Q4_K*");
        c.choices[1].quant.label = "Q4_1".into();
        assert_eq!(
            auto_pick(&c, &m, &h).unwrap().choice,
            3,
            "then the largest 4-bit"
        );
        let c = cat(&[(3, 3 * G), (8, 9 * G), (6, 7 * G)], &[]);
        assert_eq!(auto_pick(&c, &m, &h).unwrap().choice, 2);
        let c = cat(&[(2, 2 * G), (3, 3 * G)], &[]);
        assert_eq!(auto_pick(&c, &m, &h).unwrap().choice, 1);
    }

    #[test]
    fn mtp_is_included_whatever_the_hardware() {
        let c = cat(&[(4, 5 * G)], &[(4, G, false)]);
        let m = meta();
        let roomy = auto_pick(&c, &m, &hw(20 * G, 16 * G)).unwrap();
        assert_eq!((roomy.mtp, roomy.fit), (Some(0), Fit::Smooth));
        let tight = auto_pick(&c, &m, &hw(6 * G + G / 5, 16 * G)).unwrap();
        assert_eq!((tight.mtp, tight.fit), (Some(0), Fit::Slow));
        assert_eq!(tight.required, required(&m, 5 * G, G));
    }

    /// 同 llama.cpp：只看主模型資料夾或上層的 MTP，資料夾深的優先，再來同名量化，再來位元數接近。
    #[test]
    fn mtp_follows_llama_cpp_sibling_rules() {
        let mtp = |path: &str, bits: u8| MtpChoice {
            quant: quant(bits),
            file: RepoFile {
                path: path.into(),
                size: G,
                sha256: None,
            },
            shared: false,
        };
        let mut c = cat(&[(4, 5 * G)], &[]);
        c.choices[0].quant.label = "Q4_K_M".into();
        c.choices[0].files[0].path = "Q4_K_M/m-Q4_K_M.gguf".into();
        let (m, h) = (meta(), hw(20 * G, 16 * G));
        c.mtp = vec![mtp("MTP/mtp-m-Q4_K_M.gguf", 4)];
        assert_eq!(auto_pick(&c, &m, &h).unwrap().mtp, None, "other folder");
        c.mtp = vec![mtp("mtp-m-Q8_0.gguf", 8), mtp("Q4_K_M/mtp-m-Q8_0.gguf", 8)];
        assert_eq!(auto_pick(&c, &m, &h).unwrap().mtp, Some(1), "deeper folder");
        c.mtp = vec![
            mtp("Q4_K_M/mtp-m-Q4_0.gguf", 4),
            mtp("Q4_K_M/mtp-m-Q4_K_M.gguf", 4),
        ];
        assert_eq!(auto_pick(&c, &m, &h).unwrap().mtp, Some(1), "same quant");
        c.mtp = vec![
            mtp("Q4_K_M/mtp-m-Q8_0.gguf", 8),
            mtp("Q4_K_M/mtp-m-Q3_K.gguf", 3),
        ];
        assert_eq!(auto_pick(&c, &m, &h).unwrap().mtp, Some(1), "closest bits");
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
        let p = auto_pick(&c, &m, &hw(8 * G, 32 * G)).unwrap();
        assert_eq!(p.fit, Fit::Offload);
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
