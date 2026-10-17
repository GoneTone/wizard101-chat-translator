//! overlay 換介面語言後的重繪；本檔是唯一切換全域語言的 UI 測試，獨立成一個測試執行檔免得與其他測試互相干擾。

use eframe::egui::{Event, Modifiers, PointerButton, Pos2, Shape, ViewportCommand, ViewportId};
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
    harness.state_mut().set_region_hotkey("ctrl+shift+space");
    harness.state_mut().banners_mut().offer_update(Release {
        version: "0.2.0".into(),
        url: "https://example.invalid/rel".into(),
    });
    harness.step();
    let zh_status = t("status.listening", &[]);
    // 標題列小字與空清單的大字
    assert_eq!(harness.query_all_by_label(&zh_status).count(), 2);
    let zh_copy = t("menu.copy", &[]);
    let zh_tooltip = t("tooltip.region", &[("hotkey", "ctrl+shift+space")]);
    assert_eq!(tooltip_after_pause(&mut harness), zh_tooltip);

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
    assert_eq!(
        tooltip_after_pause(&mut harness),
        "Translate a screen area (ctrl+shift+space)"
    );

    for _ in 0..60 {
        poster.post(UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: "original line".into(),
            translated: "translated line".into(),
            msg_id: None,
            pending: false,
            color: None,
        });
    }
    harness.step();
    assert!(
        copy_menu_opens(&mut harness),
        "no copy menu after selecting"
    );
    assert!(shows_text(&harness, "Copy"));
    assert!(!shows_text(&harness, &zh_copy));
}

/// 懸停 ⛶ 超過提示延遲後讀出提示框文字，之後把游標移開。
fn tooltip_after_pause(harness: &mut Harness<App>) -> String {
    let button = harness.get_by_label("⛶").rect();
    harness.hover_at(button.center());
    for _ in 0..40 {
        harness.step();
    }
    let text = harness
        .get_by(|n| n.value().is_some_and(|v| v.contains("ctrl+shift+space")))
        .value()
        .unwrap();
    harness.hover_at(Pos2::new(button.center().x, 200.0));
    harness.step();
    text
}

/// 在清單中段拖曳選取一段字再按右鍵；訊息列之間有空隙，換幾個高度試。
fn copy_menu_opens(harness: &mut Harness<App>) -> bool {
    [300.0, 306.0, 312.0, 318.0].into_iter().any(|y| {
        let (from, to) = (Pos2::new(20.0, y), Pos2::new(400.0, y));
        harness.hover_at(from);
        harness.drag_at(from);
        harness.step();
        harness.hover_at(to);
        harness.step();
        harness.drop_at(to);
        harness.step();
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos: to,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        harness.step();
        harness.step();
        shows_text(harness, "Copy")
    })
}

fn shows_text(harness: &Harness<App>, text: &str) -> bool {
    harness
        .output()
        .shapes
        .iter()
        .any(|c| matches!(&c.shape, Shape::Text(s) if s.galley.text() == text))
}
