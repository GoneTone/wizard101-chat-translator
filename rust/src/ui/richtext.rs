//! 文字中帶可點連結的顯示與其純邏輯。
//!
//! `[文字](網址)` 是語言檔用的行內連結語法（`parse_link_markup`）；API 回傳的錯誤訊息常夾著裸網址，
//! `linkify` 先把它們轉成同一種語法，顯示端只需認一種格式。

use std::sync::LazyLock;

use eframe::egui::{self, Color32, Hyperlink, Label, RichText, Ui};
use regex::{Captures, Regex};

use crate::logs::log;

/// 淺色設定視窗上的連結藍（overlay 的深色底另用 `palette::FG_UPDATE`）。
pub const LINK_COLOR: Color32 = Color32::from_rgb(0x4a, 0x7d, 0xdc);

static LINK_MARKUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]\n]+)\]\(([^)\s]+)\)").unwrap());
// 只收 RFC 3986 的 ASCII 字元：網址常緊接中文或全形標點，用「非空白」當邊界會把整句吃進連結
static BARE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)https?://[A-Za-z0-9\-._~:/?#@!$&'*+,;=%]+").unwrap());
const SAFE_SCHEMES: [&str; 2] = ["http://", "https://"];
const URL_TRAILING_PUNCT: &str = ".,;:!?'";

/// 把 `[文字](網址)` 切成（顯示文字，網址或 `None`）的段落；其餘字元原樣留在文字段。
///
/// 網址只放行 http／https：語言檔可由外部貢獻者提供，其他 scheme（`file:`、`javascript:`）降級成純文字。
pub fn parse_link_markup(text: &str) -> Vec<(String, Option<String>)> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    for m in LINK_MARKUP.captures_iter(text) {
        let whole = m.get(0).unwrap();
        if whole.start() > cursor {
            segments.push((text[cursor..whole.start()].to_owned(), None));
        }
        let (label, url) = (m[1].to_owned(), &m[2]);
        let lower = url.to_lowercase();
        if SAFE_SCHEMES.iter().any(|s| lower.starts_with(s)) {
            segments.push((label, Some(url.to_owned())));
        } else {
            let scheme: String = url
                .split(':')
                .next()
                .unwrap_or("")
                .chars()
                .take(16)
                .collect();
            log(format!("[ui] link markup rejected: scheme={scheme}"));
            segments.push((label, None));
        }
        cursor = whole.end();
    }
    if cursor < text.len() {
        segments.push((text[cursor..].to_owned(), None));
    }
    segments
}

/// 把裸 http／https 網址包成 `[網址](網址)`；已是連結語法的不重複包，句尾標點不算網址。
pub fn linkify(text: &str) -> String {
    BARE_URL
        .replace_all(text, |m: &Captures| {
            let whole = m.get(0).unwrap();
            let before = &text[..whole.start()];
            if before.ends_with("](") || before.ends_with('[') {
                return whole.as_str().to_owned();
            }
            let url = whole
                .as_str()
                .trim_end_matches(|c| URL_TRAILING_PUNCT.contains(c));
            format!("[{url}]({url}){}", &whole.as_str()[url.len()..])
        })
        .into_owned()
}

#[derive(Default)]
struct Segmenter;

impl egui::cache::ComputerMut<&str, Vec<(String, Option<String>)>> for Segmenter {
    fn compute(&mut self, text: &str) -> Vec<(String, Option<String>)> {
        parse_link_markup(&linkify(text))
    }
}

/// 依寬度換行顯示 `text`，網址段是可點的 `Hyperlink`，以系統瀏覽器開啟。
///
/// 字型取 `style.override_font_id`、連結色取 `visuals.hyperlink_color`，由呼叫端設定。
pub fn show(ui: &mut Ui, text: &str, color: Color32) {
    // 每幀重繪都切段會讓被拒的連結每幀記一次 log：切段結果留到文字不再顯示為止
    let segments = ui.memory_mut(|m| {
        m.caches
            .cache::<egui::cache::FrameCache<_, Segmenter>>()
            .get(text)
            .clone()
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (segment, url) in segments {
            match url {
                None => {
                    ui.add(Label::new(RichText::new(segment).color(color)).selectable(false));
                }
                Some(url) => {
                    if ui
                        .add(Hyperlink::from_label_and_url(segment, &url))
                        .clicked()
                    {
                        log(format!("[ui] opening link from message: {url}"));
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::fonts_egui;

    #[test]
    fn javascript_scheme_is_plain_text() {
        assert_eq!(
            parse_link_markup("[bad](javascript:alert(1)) [ok](https://a.example)"),
            [
                ("bad".to_owned(), None),
                (") ".to_owned(), None),
                ("ok".to_owned(), Some("https://a.example".to_owned())),
            ]
        );
        assert_eq!(
            parse_link_markup("[f](file:///C:/x)"),
            [("f".to_owned(), None)]
        );
    }

    fn harness(text: &'static str) -> Harness<'static> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(240.0, 120.0))
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui(move |ui| show(ui, text, Color32::RED));
        harness.step();
        harness
    }

    #[test]
    fn plain_text_has_no_links() {
        let harness = harness("✗ HTTP 401: bad key");
        harness.get_by_label("✗ HTTP 401: bad key");
        assert!(harness.query_by_role(egui::accesskit::Role::Link).is_none());
    }

    #[test]
    fn clicking_a_bare_url_opens_it() {
        let mut harness = harness("see https://a.example/help now");
        harness.get_by_label("https://a.example/help").click();
        harness.step();
        let opened: Vec<_> = harness
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(opened, ["https://a.example/help"]);
    }

    #[test]
    fn long_text_wraps_within_the_width() {
        let harness = harness(
            "a fairly long error message that certainly needs more than one line at two hundred pixels wide",
        );
        let rect = harness
            .get_by(|n| n.value().is_some_and(|v| v.starts_with("a fairly")))
            .rect();
        assert!(rect.max.x <= 240.0 && rect.height() > 20.0, "{rect:?}");
    }

    #[test]
    fn rich_label_fits_a_long_unbreakable_word() {
        let text = format!(
            "see https://a.example/{}end，請開啟設定檢查",
            "verylongpath/".repeat(8)
        );
        let mut harness = Harness::builder()
            .with_size(egui::vec2(200.0, 400.0))
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui(move |ui| show(ui, &text, Color32::RED));
        harness.step();
        let bounds = harness.ctx.content_rect();
        let mut tail_seen = false;
        for clipped in &harness.output().shapes {
            if let egui::Shape::Text(shape) = &clipped.shape {
                let rect = shape.visual_bounding_rect();
                assert!(
                    bounds.contains_rect(rect.shrink(0.5)),
                    "{rect:?} {bounds:?}"
                );
                tail_seen |= shape.galley.text().contains("請開啟設定檢查");
            }
        }
        assert!(tail_seen, "the last word must be drawn inside the area");
    }
}
