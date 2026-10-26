//! 記憶體需求估算、相容等級與自動挑選量化。

use std::cmp::Reverse;

use super::gguf::ModelMeta;
use super::hardware::{GpuKind, Hardware, effective};
use super::hf::{Catalog, Choice};
use super::store::Installed;

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

/// 需求數字的來源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// 這個模型照同樣的 MTP 開關載入過，量到的實際佔用。
    Measured,
    /// 由同 repo 量過的另一個量化，加減權重檔的大小差推算。
    Derived,
    /// 依檔案大小估算（[`required`]）。
    Estimate,
}

/// 一個模型（或還沒下載的量化）載入要的記憶體；預估時 `vram` 是估算總量、`ram` 為 0。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    pub vram: u64,
    pub ram: u64,
    pub all_on_gpu: bool,
    pub basis: Basis,
}

impl Need {
    pub fn estimate(required: u64) -> Self {
        Self {
            vram: required,
            ram: 0,
            all_on_gpu: true,
            basis: Basis::Estimate,
        }
    }

    /// 不是實測（推算的也算），畫面要標「預估」。
    pub fn estimated(&self) -> bool {
        self.basis != Basis::Measured
    }

    pub fn total(&self) -> u64 {
        self.vram.saturating_add(self.ram)
    }
}

/// 需求與相容等級；VRAM 還沒量過時等級為 `None`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rating {
    pub need: Need,
    pub fit: Option<Fit>,
}

/// llama.cpp `--fit` 預設在每個裝置上留下的可用量（free memory target，1024 MiB）。
///
/// 部分卸載時 `--fit` 以整層擺放、把 VRAM 用到「可用量 − 這個值」為止，所以實測的 VRAM
/// 本來就比當時可用量少這麼多，再加上最多一層。
const FIT_TARGET: u64 = 1024 << 20;

/// 要算需求的模型或量化。
#[derive(Clone, Copy, Debug)]
pub struct Target<'a> {
    pub repo: &'a str,
    pub id: &'a str,
    /// 主模型分檔加上要用的 MTP 檔。
    pub weights: u64,
    pub mtp: bool,
    /// 要用的 MTP 檔（repo 路徑）；內建 MTP 或不開為 `None`。
    pub mtp_file: Option<&'a str>,
    /// 依檔案大小估的需求（[`required`]）。
    pub estimate: u64,
    /// 一層大約多少位元組；層數不明為 0。
    pub layer: u64,
}

/// `target` 載入時要的記憶體。
///
/// 自己照同樣的 MTP（同一個 MTP 檔）量過就用實測；否則從同 repo 量過的模型（含自己）推算
/// （先挑 MTP 一樣、全放 GPU 的，再挑權重最接近的）；都沒有就是預估。
/// 量的那次沒全放 GPU、而現在可用 VRAM 比它用的多出 [`FIT_TARGET`] 加一層以上時，看不出現在會不會流暢，也退回預估。
pub fn need(models: &[Installed], target: &Target, hw: &Hardware) -> Need {
    let room = |n: &Need| {
        n.vram
            .saturating_add(FIT_TARGET)
            .saturating_add(target.layer)
    };
    match measured_need(models, target) {
        Some(n) if n.all_on_gpu || effective(hw).0 <= room(&n) => n,
        _ => Need::estimate(target.estimate),
    }
}

/// 自己的實測或同 repo 量過的模型推算的需求。
fn measured_need(models: &[Installed], t: &Target) -> Option<Need> {
    let measured = |m: &Installed| {
        let x = m.manifest.measured.clone()?;
        Some((m.manifest.weights(x.mtp), x))
    };
    let own = models.iter().find(|m| m.id == t.id);
    let same_mtp_file =
        |m: &Installed| m.manifest.mtp.as_ref().map(|f| f.path.as_str()) == t.mtp_file;
    let usable = own
        .filter(|m| !t.mtp || same_mtp_file(m))
        .and_then(measured);
    if let Some((_, x)) = usable.filter(|(_, x)| x.mtp == t.mtp) {
        return Some(Need {
            vram: x.vram,
            ram: x.ram,
            all_on_gpu: x.all_on_gpu,
            basis: Basis::Measured,
        });
    }
    let sibling = models
        .iter()
        .filter(|m| m.manifest.repo == t.repo)
        .filter_map(measured)
        .min_by_key(|(base, x)| (x.mtp != t.mtp, !x.all_on_gpu, base.abs_diff(t.weights)));
    sibling.map(|(base, x)| Need {
        vram: x.vram.saturating_add(t.weights).saturating_sub(base),
        ram: x.ram,
        all_on_gpu: x.all_on_gpu,
        basis: Basis::Derived,
    })
}

/// 已下載的模型載入時要的記憶體；`mtp_on` 是 runtime 實際會不會開 MTP（設定開著，且沒有退回不帶 MTP）。
pub fn installed_need(
    models: &[Installed],
    model: &Installed,
    mtp_on: bool,
    hw: &Hardware,
) -> Need {
    let m = &model.manifest;
    let mtp = mtp_on && m.has_mtp();
    let target = Target {
        repo: &m.repo,
        id: &model.id,
        weights: m.weights(mtp),
        mtp,
        mtp_file: m.mtp.as_ref().map(|f| f.path.as_str()),
        estimate: m.required,
        layer: m.layer_bytes(),
    };
    need(models, &target, hw)
}

/// 依需求判斷等級：預估照 [`fit`]；實測（含推算）要每層都在 GPU、VRAM 與 RAM 各自放得下才 `Smooth`，合計放得下為 `Slow`／`Offload`。
///
/// RAM 與預估一樣先扣掉 `RAM_RESERVE`。
pub fn rate(need: &Need, moe: bool, hw: &Hardware) -> Fit {
    if need.basis == Basis::Estimate {
        return fit(need.total(), moe, hw);
    }
    let (vram, ram) = effective(hw);
    let ram = ram.saturating_sub(RAM_RESERVE);
    // 內顯（VRAM 為 0）與預估一樣永不 `Smooth`
    if need.all_on_gpu && vram > 0 && need.vram <= vram && need.ram <= ram {
        Fit::Smooth
    } else if need.total() <= vram.saturating_add(ram) {
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

/// MTP 候選的名次：在主模型路徑上、路徑上的資料夾深度、量化同名、位元數差（越小越好）。
type MtpRank = (bool, usize, bool, Reverse<i32>);

/// 有 MTP 就帶，照 llama.cpp `-hf` 找 MTP 檔的規則挑（`common/download.cpp` 的 `find_best_sibling`）。
///
/// 先看主模型資料夾或其上層裡的，取資料夾最深的，再取檔名含 `-{量化}.` 的，再取位元數最接近的；
/// 同分取路徑在前的。主模型路徑上一個都沒有時（例如 MTP 獨立放在 `MTP/`，llama.cpp 找不到），
/// 改從其他資料夾用同樣的量化與位元數規則挑。
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
    // 只有嚴格更好才換：同分留路徑在前的
    let mut best: Option<(usize, MtpRank)> = None;
    for (i, m) in cat.mtp.iter().enumerate() {
        let dir = dirs(&m.file.path);
        let on_path = model_dir.starts_with(&dir);
        let exact = m.file.path.to_ascii_uppercase().contains(&tag);
        let diff = (bits(m.quant.bits) - bits(main.quant.bits)).abs();
        let rank = (
            on_path,
            if on_path { dir.len() } else { 0 },
            exact,
            Reverse(diff),
        );
        if best.as_ref().is_none_or(|(_, b)| rank > *b) {
            best = Some((i, rank));
        }
    }
    best.map(|b| b.0)
}

/// 多個模型同時載入：需求加總（有一個是預估就照預估判），任一為 MoE 則以 MoE 判。
pub fn combined_fit(needs: &[(Need, bool)], hw: &Hardware) -> Fit {
    let mut sum = Need {
        basis: Basis::Measured,
        ..Need::estimate(0)
    };
    for (n, _) in needs {
        sum.vram = sum.vram.saturating_add(n.vram);
        sum.ram = sum.ram.saturating_add(n.ram);
        sum.all_on_gpu &= n.all_on_gpu;
        if n.basis == Basis::Estimate {
            sum.basis = Basis::Estimate;
        }
    }
    rate(&sum, needs.iter().any(|n| n.1), hw)
}

#[cfg(test)]
mod tests {
    use super::super::hardware::Gpu;
    use super::super::hf::{MtpChoice, Quant, RepoFile};
    use super::super::store::{Manifest, ManifestFile, Measured};
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
            own: Default::default(),
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
            own: Default::default(),
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

    /// 同 llama.cpp：主模型資料夾或上層的 MTP 優先、資料夾深的優先，再來同名量化，再來位元數接近；路徑上沒有才看別的資料夾。
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
        c.mtp = vec![
            mtp("MTP/mtp-m-Q4_K_M.gguf", 4),
            mtp("Q4_K_M/mtp-m-Q8_0.gguf", 8),
        ];
        assert_eq!(
            auto_pick(&c, &m, &h).unwrap().mtp,
            Some(1),
            "model path first"
        );
        c.mtp = vec![mtp("MTP/mtp-m-Q8_0.gguf", 8), mtp("MTP/mtp-m-Q4_0.gguf", 4)];
        assert_eq!(
            auto_pick(&c, &m, &h).unwrap().mtp,
            Some(1),
            "other folders as a fallback"
        );
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

    fn measured(vram: u64, ram: u64, all_on_gpu: bool) -> Need {
        Need {
            vram,
            ram,
            all_on_gpu,
            basis: Basis::Measured,
        }
    }

    #[test]
    fn combined_sums_requirements() {
        let five = Need::estimate(5 * G);
        assert_eq!(
            combined_fit(&[(five, false), (five, false)], &hw(8 * G, 3 * G)),
            Fit::NoFit
        );
        assert_eq!(combined_fit(&[], &hw(G, 3 * G)), Fit::Smooth);
        let both = [
            (measured(3 * G, G, true), false),
            (measured(4 * G, G, true), false),
        ];
        assert_eq!(combined_fit(&both, &hw(8 * G, 5 * G)), Fit::Smooth);
        let mixed = [
            (measured(3 * G, G, true), false),
            (Need::estimate(5 * G), false),
        ];
        assert_eq!(
            combined_fit(&mixed, &hw(8 * G, 5 * G)),
            Fit::Slow,
            "an estimate in the sum rates it as an estimate"
        );
    }

    /// 實測：每層都在 GPU 且 VRAM、RAM（扣掉保留量）各自放得下才流暢，合計放得下為會慢（MoE 為可用），否則放不下。
    #[test]
    fn measured_needs_are_rated_by_vram_and_ram_separately() {
        let h = hw(8 * G, 16 * G);
        assert_eq!(rate(&measured(4 * G, 3 * G, true), false, &h), Fit::Smooth);
        assert_eq!(
            rate(&measured(4 * G, 3 * G, false), false, &h),
            Fit::Slow,
            "partial offload"
        );
        assert_eq!(rate(&measured(4 * G, 3 * G, false), true, &h), Fit::Offload);
        assert_eq!(rate(&measured(9 * G, 3 * G, true), false, &h), Fit::Slow);
        assert_eq!(
            rate(&measured(4 * G, 15 * G, true), false, &h),
            Fit::Slow,
            "RAM reserve applies as in the estimate"
        );
        assert_eq!(rate(&measured(4 * G, 17 * G, true), false, &h), Fit::Slow);
        assert_eq!(rate(&measured(9 * G, 14 * G, false), false, &h), Fit::NoFit);
        assert_eq!(
            rate(&measured(0, 3 * G, true), false, &integrated(16 * G)),
            Fit::Slow,
            "integrated never smooth"
        );
        assert_eq!(
            rate(&Need::estimate(5 * G), false, &h),
            fit(5 * G, false, &h)
        );
    }

    fn installed(
        id: &str,
        repo: &str,
        main: u64,
        mtp: Option<u64>,
        measured: Option<Measured>,
    ) -> Installed {
        let file = |path: String, size| ManifestFile {
            path,
            size,
            sha256: None,
        };
        Installed {
            id: id.into(),
            manifest: Manifest {
                repo: repo.into(),
                quant: id.into(),
                files: vec![file(format!("{id}.gguf"), main)],
                mtp: mtp.map(|size| file(format!("mtp-{id}.gguf"), size)),
                moe: false,
                has_builtin_mtp: false,
                required: 99 * G,
                measured,
                layers: 4,
            },
            complete: true,
        }
    }

    fn run(vram: u64, ram: u64, mtp: bool) -> Option<Measured> {
        Some(Measured {
            vram,
            ram,
            all_on_gpu: true,
            mtp,
            engine: String::new(),
        })
    }

    fn partial(vram: u64, ram: u64, mtp: bool) -> Option<Measured> {
        run(vram, ram, mtp).map(|m| Measured {
            all_on_gpu: false,
            ..m
        })
    }

    fn target<'a>(id: &'a str, weights: u64, mtp_file: Option<&'a str>) -> Target<'a> {
        Target {
            repo: "r",
            id,
            weights,
            mtp: mtp_file.is_some(),
            mtp_file,
            estimate: 98 * G,
            layer: 0,
        }
    }

    /// 自己照同樣 MTP 開關、同一個 MTP 檔量過才用實測；MTP 開關或檔案不同時由自己的實測推算。
    #[test]
    fn own_measurement_counts_only_with_the_same_mtp() {
        let models = [installed("a", "r", 3 * G, Some(G), run(5 * G, 4 * G, true))];
        let h = hw(8 * G, 16 * G);
        assert_eq!(
            installed_need(&models, &models[0], true, &h),
            measured(5 * G, 4 * G, true)
        );
        let derived = |vram| Need {
            vram,
            ram: 4 * G,
            all_on_gpu: true,
            basis: Basis::Derived,
        };
        assert_eq!(
            installed_need(&models, &models[0], false, &h),
            derived(4 * G),
            "measured with MTP, now off"
        );
        let same = target("a", 4 * G, Some("mtp-a.gguf"));
        assert_eq!(need(&models, &same, &h).basis, Basis::Measured);
        let other = target("a", 5 * G, Some("mtp-other.gguf"));
        assert_eq!(
            need(&models, &other, &h),
            derived(6 * G),
            "another MTP file"
        );
    }

    /// 沒載入過的量化由同 repo 量過的推算：VRAM 加減權重差，RAM 照用；別的 repo 不算。
    #[test]
    fn unloaded_quants_are_derived_from_a_measured_sibling() {
        let models = [
            installed("q8", "r", 8 * G, Some(G), run(10 * G, 3 * G, true)),
            installed("q6", "r", 6 * G, None, run(7 * G, 2 * G, false)),
            installed("x", "other", 4 * G, None, run(G, G, false)),
        ];
        let h = hw(8 * G, 16 * G);
        let derived = need(&models, &target("q4", 4 * G, None), &h);
        assert_eq!(
            derived,
            Need {
                vram: 5 * G,
                ram: 2 * G,
                all_on_gpu: true,
                basis: Basis::Derived
            }
        );
        assert!(derived.estimated());
        let with_mtp = need(&models, &target("q9", 10 * G, Some("m.gguf")), &h);
        assert_eq!((with_mtp.vram, with_mtp.ram), (11 * G, 3 * G));
        let elsewhere = Target {
            repo: "none",
            ..target("q4", 4 * G, None)
        };
        assert_eq!(need(&models, &elsewhere, &h), Need::estimate(98 * G));
    }

    /// 推算先挑 MTP 一樣、全放 GPU 的量化，再挑權重最接近的。
    #[test]
    fn siblings_with_the_same_mode_come_first() {
        let models = [
            installed("near_mtp", "r", 4 * G, Some(G / 2), run(6 * G, G, true)),
            installed(
                "near_partial",
                "r",
                5 * G,
                None,
                partial(4 * G, 3 * G, false),
            ),
            installed("far", "r", G, None, run(2 * G, G, false)),
        ];
        let derived = need(&models, &target("q4", 4 * G, None), &hw(8 * G, 16 * G));
        assert_eq!(
            (derived.vram, derived.all_on_gpu),
            (5 * G, true),
            "from far"
        );
    }

    /// 量的那次沒全放 GPU：現在可用 VRAM 比它用的多出 `--fit` 的保留量以上就看不出會不會流暢，退回預估；否則照實測判（不會流暢）。
    #[test]
    fn partial_offload_measurements_fall_back_when_there_is_more_vram_now() {
        let models = [installed(
            "a",
            "r",
            8 * G,
            None,
            partial(6 * G, 5 * G, false),
        )];
        assert_eq!(
            installed_need(&models, &models[0], true, &hw(12 * G, 16 * G)),
            Need::estimate(99 * G),
            "more room than that run had"
        );
        // 8 GB 分 4 層，一層 2 GB：可用量最多到 6 + 1 + 2 = 9 GB 都還是 `--fit` 留下的
        let fit_left = hw(17 * G / 2, 16 * G);
        let kept = installed_need(&models, &models[0], true, &fit_left);
        assert_eq!(
            kept,
            measured(6 * G, 5 * G, false),
            "free = used + fit target + a layer"
        );
        assert_eq!(rate(&kept, false, &fit_left), Fit::Slow);
        assert_eq!(
            installed_need(&models, &models[0], true, &hw(19 * G / 2, 16 * G)),
            Need::estimate(99 * G),
            "more than a layer beyond the fit target"
        );
        let mut unknown = models[0].clone();
        unknown.manifest.layers = 0;
        assert_eq!(
            installed_need(&[unknown.clone()], &unknown, true, &fit_left),
            Need::estimate(99 * G),
            "layer count unknown: fit target only"
        );
        let b = target("b", 4 * G, None);
        assert_eq!(
            need(&models, &b, &hw(12 * G, 16 * G)),
            Need::estimate(98 * G),
            "derived from a partial run too"
        );
        assert_eq!(need(&models, &b, &hw(3 * G, 16 * G)).basis, Basis::Derived);
    }
}
