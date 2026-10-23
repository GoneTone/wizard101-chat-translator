//! 依模型清單透露的思考能力，決定思考開關的狀態與深度選單的項目。

use crate::config::services::{EFFORT_AUTO, Provider};
use crate::translation::clients::ThinkingCaps;

/// 思考開關的狀態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkingSwitch {
    /// 模型不支援思考：鎖定關閉。
    Unsupported,
    /// 模型一律思考：鎖定開啟。
    Always,
    /// 使用者可切換（含清單沒說的情況）。
    Toggle,
}

/// 依能力決定開關狀態；`None` 是目前模型不在清單裡。
pub fn switch_state(caps: Option<&ThinkingCaps>) -> ThinkingSwitch {
    match caps {
        Some(c) if c.supported == Some(false) => ThinkingSwitch::Unsupported,
        Some(c) if c.can_disable == Some(false) => ThinkingSwitch::Always,
        _ => ThinkingSwitch::Toggle,
    }
}

/// 深度選單的項目：自動在最前，接著是該家等級；清單列了模型支援的等級時只留那些。
pub fn effort_options(provider: &Provider, caps: Option<&ThinkingCaps>) -> Vec<&'static str> {
    let allowed = caps.and_then(|c| c.efforts.as_ref());
    std::iter::once(EFFORT_AUTO)
        .chain(
            provider
                .efforts
                .iter()
                .copied()
                .filter(|level| allowed.is_none_or(|a| a.iter().any(|x| x == level))),
        )
        .collect()
}

/// 畫面上與存檔時實際生效的開關值：鎖定時忽略使用者的選擇。
pub fn effective_thinking(switch: ThinkingSwitch, chosen: bool) -> bool {
    match switch {
        ThinkingSwitch::Unsupported => false,
        ThinkingSwitch::Always => true,
        ThinkingSwitch::Toggle => chosen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::services::provider;

    fn caps(supported: Option<bool>, can_disable: Option<bool>) -> ThinkingCaps {
        ThinkingCaps {
            supported,
            can_disable,
            efforts: None,
        }
    }

    fn levels(items: &[&str]) -> ThinkingCaps {
        ThinkingCaps {
            efforts: Some(items.iter().map(|s| (*s).to_owned()).collect()),
            ..ThinkingCaps::default()
        }
    }

    #[test]
    fn unsupported_wins_over_can_disable() {
        let c = caps(Some(false), Some(true));
        assert_eq!(switch_state(Some(&c)), ThinkingSwitch::Unsupported);
    }

    #[test]
    fn cannot_disable_is_always() {
        let c = caps(Some(true), Some(false));
        assert_eq!(switch_state(Some(&c)), ThinkingSwitch::Always);
    }

    #[test]
    fn unknown_is_toggle() {
        assert_eq!(switch_state(None), ThinkingSwitch::Toggle);
        let c = ThinkingCaps::default();
        assert_eq!(switch_state(Some(&c)), ThinkingSwitch::Toggle);
    }

    #[test]
    fn options_start_with_auto_and_follow_caps() {
        let claude = provider("claude").unwrap();
        let gemini = provider("gemini").unwrap();
        assert_eq!(
            effort_options(claude, Some(&levels(&["low", "high"]))),
            ["auto", "low", "high"]
        );
        assert_eq!(effort_options(claude, Some(&levels(&[]))), ["auto"]);
        assert_eq!(
            effort_options(gemini, None),
            ["auto", "minimal", "low", "medium", "high"]
        );
    }

    #[test]
    fn caps_level_unknown_to_provider_is_ignored() {
        let gemini = provider("gemini").unwrap();
        assert_eq!(effort_options(gemini, Some(&levels(&["xhigh"]))), ["auto"]);
    }

    #[test]
    fn effective_thinking_follows_the_lock() {
        assert!(!effective_thinking(ThinkingSwitch::Unsupported, true));
        assert!(effective_thinking(ThinkingSwitch::Always, false));
        assert!(effective_thinking(ThinkingSwitch::Toggle, true));
        assert!(!effective_thinking(ThinkingSwitch::Toggle, false));
    }
}
