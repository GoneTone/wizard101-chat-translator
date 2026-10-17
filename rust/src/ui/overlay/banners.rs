//! overlay 的兩條橫幅：錯誤（網址可點）與更新提示（整列可點、✕ 關閉），排在清單下方、比清單先分到高度。
//!
//! 存的是文案 key 與參數、`Release`，每幀才翻譯，換語言後自然跟著變。

use eframe::egui::{
    CursorIcon, FontId, OpenUrl, Rect, Role, Sense, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};

use crate::i18n::t;
use crate::logs::log;
use crate::reader::status::Banner;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::overlay::frame::GRIP_SIZE;
use crate::ui::palette::{BG_UPDATE, FG_ERROR, FG_UPDATE};
use crate::ui::richtext;

const BANNER_PT: f32 = 10.0;
const CLOSE_PT: f32 = 9.0;
const PAD_Y: f32 = 2.0;
const CLOSE_PAD: (f32, f32) = (4.0, 6.0);

/// GitHub 上的一個 release：版本號（已去 `v` 前綴）與該 release 的網頁網址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// 錯誤橫幅與更新橫幅的狀態，以及使用者這次執行內關掉的版本。
#[derive(Default)]
pub struct Banners {
    error: Option<Banner>,
    update: Option<Release>,
    dismissed: Option<String>,
    error_height: f32,
}

impl Banners {
    /// 顯示錯誤橫幅（文案 key 與 format 參數，顯示時才翻譯）。
    pub fn set_error(&mut self, key: &'static str, args: Vec<(&'static str, String)>) {
        self.error = Some((key, args));
    }

    /// 收起錯誤橫幅。
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// 錯誤橫幅目前的文字；沒有橫幅回 `None`。
    pub fn error_text(&self) -> Option<String> {
        let (key, args) = self.error.as_ref()?;
        let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        Some(t(key, &args))
    }

    /// 自動檢查用的入口：使用者按 ✕ 關掉過的版本不再跳出；手動檢查直接走 `set_update`。
    pub fn offer_update(&mut self, release: Release) {
        // shortcut: 只擋同一版；第 4 段移植 SemVer 比較後改成「不比關掉的版本新就略過」
        if let Some(dismissed) = self.dismissed.as_ref().filter(|d| **d == release.version) {
            log(format!(
                "[update] banner skipped: {} not newer than dismissed {dismissed}",
                release.version
            ));
            return;
        }
        self.set_update(release);
    }

    /// 顯示更新橫幅：整列可點（開瀏覽器到下載頁），右側 ✕ 關掉並記下版本。
    pub fn set_update(&mut self, release: Release) {
        log(format!("[update] banner shown for {}", release.version));
        self.update = Some(release);
    }

    /// 收起更新橫幅；不算使用者關掉，之後同版仍會再提醒。
    pub fn clear_update(&mut self) {
        self.update = None;
    }

    /// 更新橫幅目前的文字；沒有橫幅回 `None`。
    pub fn update_text(&self) -> Option<String> {
        let release = self.update.as_ref()?;
        Some(t("update.available", &[("version", &release.version)]))
    }

    /// 在 `rect` 底部由下往上畫出更新與錯誤橫幅，回傳留給清單的上半部。
    ///
    /// 橫幅先分到高度：視窗再小也看得到（「遊戲未就緒」正是最該看到的訊息），超出的部分被裁掉。
    pub fn show(&mut self, ui: &mut Ui, rect: Rect) -> Rect {
        let clip = rect.intersect(ui.clip_rect());
        let mut bottom = rect.max.y;
        if let (Some(text), Some(release)) = (self.update_text(), self.update.clone()) {
            bottom = self.update_row(ui, rect, bottom, clip, &text, &release);
        }
        if let Some(text) = self.error_text() {
            // 換行後的高度要排完才知道：先用上一輪量到的高度定位，高度變了就重跑這一輪
            let top = bottom - PAD_Y - self.error_height;
            let area = Rect::from_min_max(pos2(rect.min.x, top), pos2(rect.max.x, bottom - PAD_Y));
            let mut child = ui.new_child(UiBuilder::new().max_rect(area));
            child.set_clip_rect(clip);
            child.style_mut().override_font_id = Some(FontId::proportional(tk_pt(BANNER_PT)));
            child.visuals_mut().hyperlink_color = FG_UPDATE;
            richtext::show(&mut child, &text, FG_ERROR);
            let height = child.min_rect().height();
            if height != self.error_height {
                self.error_height = height;
                ui.ctx().request_discard("error banner height changed");
                ui.ctx().request_repaint();
            }
            bottom = top - PAD_Y;
        }
        Rect::from_min_max(rect.min, pos2(rect.max.x, bottom.max(rect.min.y)))
    }

    /// 畫在 `bottom` 之上，回傳這列的上緣。
    fn update_row(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        bottom: f32,
        clip: Rect,
        text: &str,
        release: &Release,
    ) -> f32 {
        let painter = ui.painter_at(clip);
        let close =
            painter.layout_no_wrap("✕".into(), FontId::proportional(tk_pt(CLOSE_PT)), FG_UPDATE);
        // 右側讓出縮放把手：把手疊在不透明橫幅上會蓋掉半個 ✕
        let width = rect.width() - GRIP_SIZE;
        let close_w = CLOSE_PAD.0 + close.size().x + CLOSE_PAD.1;
        let galley = painter.layout(
            text.to_owned(),
            FontId::proportional(tk_pt(BANNER_PT)),
            FG_UPDATE,
            (width - close_w).max(1.0),
        );
        let height = galley.size().y.max(close.size().y);
        let row_rect = Rect::from_min_size(
            pos2(rect.min.x, bottom - PAD_Y - height),
            vec2(width, height),
        );
        let row = ui.interact(
            row_rect,
            ui.scope_id().with("update_banner"),
            Sense::click(),
        );
        row.widget_info(|| WidgetInfo::labeled(Role::Button, true, text));
        let close_rect = Rect::from_min_size(
            pos2(
                row_rect.max.x - CLOSE_PAD.1 - close.size().x,
                row_rect.center().y - close.size().y / 2.0,
            ),
            close.size(),
        );
        // 晚於整列登記：重疊處由 ✕ 接到點擊
        let dismiss = ui.interact(close_rect, row.id.with("close"), Sense::click());
        dismiss.widget_info(|| WidgetInfo::labeled(Role::Button, true, "✕"));
        painter.rect_filled(row_rect, 0.0, BG_UPDATE);
        let text_top = row_rect.min.y + (height - galley.size().y) / 2.0;
        painter.galley(pos2(row_rect.min.x, text_top), galley, FG_UPDATE);
        painter.galley(close_rect.min, close, FG_UPDATE);

        if dismiss.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            log(format!(
                "[update] banner dismissed version={}",
                release.version
            ));
            self.dismissed = Some(release.version.clone());
            self.update = None;
        } else if row.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            log(format!(
                "[update] banner clicked version={}",
                release.version
            ));
            ui.ctx().open_url(OpenUrl::new_tab(&release.url));
        }
        row_rect.min.y - PAD_Y
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::fonts_egui;

    struct State {
        banners: Banners,
        area: Rect,
        rest: Rect,
    }

    fn harness(banners: Banners) -> Harness<'static, State> {
        let mut harness = Harness::builder()
            .with_size(vec2(460.0, 300.0))
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui_state(
                |ui, s: &mut State| {
                    s.area = ui.max_rect();
                    s.rest = s.banners.show(ui, s.area);
                },
                State {
                    banners,
                    area: Rect::NOTHING,
                    rest: Rect::NOTHING,
                },
            );
        harness.step();
        harness
    }

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            url: "https://example.invalid/rel".into(),
        }
    }

    fn opened_urls(harness: &Harness<State>) -> Vec<String> {
        harness
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn dismissed_update_version_not_reoffered() {
        let mut banners = Banners::default();
        banners.set_update(release("0.2.0"));
        let mut harness = harness(banners);
        harness.get_by_label("✕").click();
        harness.step();
        let banners = &mut harness.state_mut().banners;
        assert_eq!(banners.update_text(), None);
        assert!(opened_urls(&harness).is_empty(), "✕ must not open the page");

        let banners = &mut harness.state_mut().banners;
        banners.offer_update(release("0.2.0"));
        assert_eq!(banners.update_text(), None);
        banners.offer_update(release("0.3.0"));
        assert_eq!(
            banners.update_text(),
            Some(t("update.available", &[("version", "0.3.0")]))
        );
        // 手動檢查走 set_update：關掉過的版本也要再顯示
        banners.dismissed = Some("0.3.0".into());
        banners.set_update(release("0.3.0"));
        assert!(banners.update_text().is_some());
    }

    #[test]
    fn clear_update_does_not_count_as_dismissing() {
        let mut banners = Banners::default();
        banners.set_update(release("0.2.0"));
        banners.clear_update();
        banners.offer_update(release("0.2.0"));
        assert_eq!(
            banners.update_text(),
            Some(t("update.available", &[("version", "0.2.0")]))
        );
    }

    #[test]
    fn clicking_the_update_row_opens_the_release_page() {
        let mut banners = Banners::default();
        banners.set_update(release("0.2.0"));
        let mut harness = harness(banners);
        let label = t("update.available", &[("version", "0.2.0")]);
        harness.get_by_label(&label).click();
        harness.step();
        assert_eq!(opened_urls(&harness), ["https://example.invalid/rel"]);
        assert!(harness.state().banners.update_text().is_some());
    }

    #[test]
    fn update_row_leaves_room_for_the_resize_grip() {
        let mut banners = Banners::default();
        banners.set_update(release("0.2.0"));
        let harness = harness(banners);
        let label = t("update.available", &[("version", "0.2.0")]);
        let row = harness.get_by_label(&label).rect();
        assert_eq!(row.max.x, harness.state().area.max.x - GRIP_SIZE);
        assert!(harness.get_by_label("✕").rect().max.x <= row.max.x);
    }

    #[test]
    fn error_and_update_banners_coexist_above_the_list() {
        let mut banners = Banners::default();
        banners.set_error("notice.offline", Vec::new());
        banners.set_update(release("0.2.0"));
        let mut harness = harness(banners);
        let error = harness.get_by_label(&t("notice.offline", &[])).rect();
        let update = harness
            .get_by_label(&t("update.available", &[("version", "0.2.0")]))
            .rect();
        let (area, rest) = (harness.state().area, harness.state().rest);
        assert!(rest.max.y <= error.min.y.min(update.min.y), "{rest:?}");
        assert!(update.max.y <= area.max.y && error.max.y <= update.min.y);

        harness.state_mut().banners.clear_update();
        harness.step();
        assert_eq!(
            harness.state().banners.error_text(),
            Some(t("notice.offline", &[]))
        );
        harness.state_mut().banners.clear_error();
        harness.step();
        assert_eq!(harness.state().banners.error_text(), None);
        assert_eq!(harness.state().rest, harness.state().area);
    }

    #[test]
    fn error_banner_formats_args_and_links_urls() {
        let mut banners = Banners::default();
        banners.set_error(
            "notice.config_error_detail",
            vec![
                ("status", "401".into()),
                ("message", "see https://a.example/keys for a key".into()),
            ],
        );
        let text = banners.error_text().unwrap();
        assert!(
            text.contains("see https://a.example/keys for a key"),
            "{text}"
        );
        let harness = harness(banners);
        harness.get_by_label("https://a.example/keys");
    }
}
