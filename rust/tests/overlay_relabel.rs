//! overlay 換介面語言後的重繪；本檔是唯一切換全域語言的 UI 測試，獨立成一個測試執行檔免得與其他測試互相干擾。

use eframe::egui::{ViewportCommand, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::ui::app::{App, channel};
use wizard101_chat_translator::ui::events::UiEvent;
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::overlay::banners::Release;
use wizard101_chat_translator::ui::overlay::frame::Frame;

#[test]
fn status_key_relabels_on_language_change() {
    i18n::set_language("zh-TW");
    let (poster, inbox) = channel();
    poster.post(UiEvent::SetStatus("listening"));
    poster.post(UiEvent::SetError {
        key: "notice.config_error_detail",
        args: vec![
            ("status", "401".into()),
            ("message", "Incorrect API key".into()),
        ],
    });
    let mut harness = Harness::new_eframe(|cc| {
        fonts_egui::install(&cc.egui_ctx, "zh-TW");
        App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
    });
    harness.state_mut().banners_mut().offer_update(Release {
        version: "0.2.0".into(),
        url: "https://example.invalid/rel".into(),
    });
    harness.step();
    let zh_status = t("status.listening", &[]);
    // 標題列小字與空清單的大字
    assert_eq!(harness.query_all_by_label(&zh_status).count(), 2);

    i18n::set_language("en-US");
    let ctx = harness.ctx.clone();
    harness.state_mut().relabel(&ctx);
    harness.step();
    let title = ViewportCommand::Title("Wizard101 Chat Translator".into());
    assert!(
        harness.output().viewport_output[&ViewportId::ROOT]
            .commands
            .contains(&title)
    );
    assert_eq!(harness.query_all_by_label("●  Listening").count(), 2);
    assert!(harness.query_by_label(&zh_status).is_none());
    harness.get_by_label(
        "⚠  API responded with HTTP 401: Incorrect API key — open settings (⚙) to check",
    );
    harness.get_by_label("⬆  Version 0.2.0 is available — click to download");
}
