//! 內建推薦模型清單與精靈的自動推薦。

use super::fit::Fit;

/// 推薦的 Hugging Face 模型；`wizard` 為 `false` 者不讓精靈自動挑。
#[derive(Debug, PartialEq)]
pub struct Recommended {
    pub repo: &'static str,
    pub name: &'static str,
    pub wizard: bool,
}

/// 推薦清單，順序即定位由低到高。
pub const RECOMMENDED: &[Recommended] = &[
    Recommended {
        repo: "unsloth/Qwen3.5-4B-GGUF",
        name: "Qwen3.5-4B",
        wizard: true,
    },
    Recommended {
        repo: "unsloth/gemma-4-E4B-it-GGUF",
        name: "Gemma 4 E4B",
        wizard: true,
    },
    Recommended {
        repo: "unsloth/Qwen3.5-9B-GGUF",
        name: "Qwen3.5-9B",
        wizard: true,
    },
    Recommended {
        repo: "google/gemma-4-12B-it-qat-q4_0-gguf",
        name: "Gemma 4 12B QAT",
        wizard: true,
    },
    Recommended {
        repo: "unsloth/gemma-4-26B-A4B-it-GGUF",
        name: "Gemma 4 26B-A4B",
        wizard: false,
    },
    Recommended {
        repo: "unsloth/Qwen3.8-27B-GGUF",
        name: "Qwen3.8-27B",
        wizard: false,
    },
];

/// 精靈用：只看 `wizard` 項目，挑 `Fit` 最好、同等級取順序最後者。
pub fn wizard_pick<'a>(fits: &[(&'a Recommended, Fit)]) -> Option<&'a Recommended> {
    fits.iter()
        .filter(|(r, _)| r.wizard)
        .max_by_key(|(_, f)| *f)
        .map(|(r, _)| *r)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 六個推薦都 Smooth 時，精靈跳過高階、回 Gemma 4 12B QAT。
    #[test]
    fn wizard_skips_high_end_models() {
        let all: Vec<_> = RECOMMENDED.iter().map(|r| (r, Fit::Smooth)).collect();
        assert_eq!(wizard_pick(&all).unwrap().name, "Gemma 4 12B QAT");
    }

    /// 較大的模型跑不動時退到較小且等級較好者。
    #[test]
    fn wizard_prefers_better_fit() {
        let fits: Vec<_> = RECOMMENDED
            .iter()
            .enumerate()
            .map(|(i, r)| (r, if i < 2 { Fit::Smooth } else { Fit::Slow }))
            .collect();
        assert_eq!(wizard_pick(&fits).unwrap().name, "Gemma 4 E4B");
    }

    /// Smooth 的較小模型勝過 Offload 的較大模型。
    #[test]
    fn wizard_prefers_smooth_over_offload() {
        let fits = [
            (&RECOMMENDED[0], Fit::Smooth),
            (&RECOMMENDED[3], Fit::Offload),
        ];
        assert_eq!(wizard_pick(&fits).unwrap().name, "Qwen3.5-4B");
    }
}
