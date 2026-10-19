//! overlay 的兩條橫幅：錯誤（網址可點）與更新提示（整列可點、✕ 關閉），排在清單下方、比清單先分到高度。
//!
//! 存的是文案 key 與參數、`Release`，每幀才翻譯，換語言後自然跟著變。

use eframe::egui::{
    CursorIcon, FontId, OpenUrl, Rect, Role, Sense, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};

use crate::i18n::t;
use crate::logs::log;
use crate::reader::status::Banner;
pub use crate::runtime::updater::Release;
use crate::runtime::updater::is_newer;
use crate::translation::errors::localized;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::overlay::frame::{GRIP_SIZE, edge_under};
use crate::ui::palette::{BG_UPDATE, FG_ERROR, FG_UPDATE};
use crate::ui::richtext;

const BANNER_PT: f32 = 10.0;
const CLOSE_PT: f32 = 9.0;
const PAD_Y: f32 = 2.0;
const CLOSE_PAD: (f32, f32) = (4.0, 6.0);

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
        Some(localized(key, args))
    }

    /// 自動檢查用的入口：使用者按 ✕ 關掉過的版本（或更舊的）不再跳出；手動檢查直接走 `set_update`。
    pub fn offer_update(&mut self, release: Release) {
        if let Some(dismissed) = self
            .dismissed
            .as_ref()
            .filter(|d| !is_newer(&release.version, d))
        {
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
    /// `ui` 是整個視窗：縮放帶依它的 `max_rect` 判斷。
    pub fn show(&mut self, ui: &mut Ui, rect: Rect) -> Rect {
        let clip = rect.intersect(ui.clip_rect());
        let on_band = over_resize_band(ui);
        let mut bottom = rect.max.y;
        if let Some(release) = self.update.clone() {
            bottom = self.update_row(ui, rect, clip, on_band, &release);
        }
        if let Some(text) = self.error_text() {
            // 換行後的高度要排完才知道：先用上一輪量到的高度定位，高度變了就重跑這一輪
            let top = bottom - PAD_Y - self.error_height;
            let area = Rect::from_min_max(pos2(rect.min.x, top), pos2(rect.max.x, bottom - PAD_Y));
            let mut child = ui.new_child(UiBuilder::new().max_rect(area));
            child.set_clip_rect(clip);
            child.style_mut().override_font_id = Some(FontId::proportional(tk_pt(BANNER_PT)));
            child.visuals_mut().hyperlink_color = FG_UPDATE;
            if on_band {
                child.visuals_mut().disabled_alpha = 1.0;
                child.disable();
            }
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

    /// 貼著 `rect` 底部畫，回傳這列的上緣。
    fn update_row(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        clip: Rect,
        on_band: bool,
        release: &Release,
    ) -> f32 {
        let text = &self.update_text().unwrap_or_default();
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
            pos2(rect.min.x, rect.max.y - PAD_Y - height),
            vec2(width, height),
        );
        // 只在看得到的部分接滑鼠：視窗太矮時被裁掉的上半截不可蓋住分頁列與標題列的按鈕
        let row = ui.interact(
            row_rect.intersect(clip),
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
        let dismiss = ui.interact(
            close_rect.intersect(clip),
            row.id.with("close"),
            Sense::click(),
        );
        dismiss.widget_info(|| WidgetInfo::labeled(Role::Button, true, "✕"));
        painter.rect_filled(row_rect, 0.0, BG_UPDATE);
        let text_top = row_rect.min.y + (height - galley.size().y) / 2.0;
        painter.galley(pos2(row_rect.min.x, text_top), galley, FG_UPDATE);
        painter.galley(close_rect.min, close, FG_UPDATE);

        if on_band {
            return row_rect.min.y - PAD_Y;
        }
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

/// 游標或按壓起點壓在外框的縮放帶上：橫幅讓給縮放，不換手指游標、不開連結。
fn over_resize_band(ui: &Ui) -> bool {
    let window = ui.max_rect();
    ui.input(|i| {
        [i.pointer.latest_pos(), i.pointer.press_origin()]
            .into_iter()
            .flatten()
            .any(|p| edge_under(window, p).is_some())
    })
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
        /// 內容區頂端距視窗頂端的距離（模擬標題列與分頁列）。
        top: f32,
        area: Rect,
        rest: Rect,
    }

    fn harness(banners: Banners) -> Harness<'static, State> {
        sized(banners, vec2(460.0, 300.0), 0.0)
    }

    fn sized(banners: Banners, size: egui::Vec2, top: f32) -> Harness<'static, State> {
        let mut harness = Harness::builder()
            .with_size(size)
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui_state(
                |ui, s: &mut State| {
                    s.area = ui.max_rect();
                    let mut rect = s.area;
                    rect.min.y += s.top;
                    s.rest = s.banners.show(ui, rect);
                },
                State {
                    banners,
                    top,
                    area: Rect::NOTHING,
                    rest: Rect::NOTHING,
                },
            );
        harness.step();
        harness
    }

    fn click_at(harness: &mut Harness<State>, pos: egui::Pos2) {
        harness.hover_at(pos);
        harness.step();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        harness.step();
    }

    fn cursor(harness: &mut Harness<State>, pos: egui::Pos2) -> egui::CursorIcon {
        harness.hover_at(pos);
        harness.step();
        harness.output().platform_output.cursor_icon
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
    fn offer_update_skips_versions_not_newer_than_dismissed() {
        let mut banners = Banners {
            dismissed: Some("0.2.0".into()),
            ..Default::default()
        };
        for version in ["0.2.0", "0.1.9"] {
            banners.offer_update(release(version));
            assert_eq!(banners.update_text(), None, "{version} must stay hidden");
        }
        banners.offer_update(release("0.2.1"));
        assert_eq!(
            banners.update_text(),
            Some(t("update.available", &[("version", "0.2.1")]))
        );
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
    fn update_row_yields_to_the_resize_bands() {
        let mut banners = Banners::default();
        banners.set_update(release("0.2.0"));
        let mut harness = harness(banners);
        let label = t("update.available", &[("version", "0.2.0")]);
        let row = harness.get_by_label(&label).rect();
        let band = egui::pos2(row.min.x + 2.0, row.center().y);
        assert_ne!(cursor(&mut harness, band), CursorIcon::PointingHand);
        click_at(&mut harness, band);
        assert!(
            opened_urls(&harness).is_empty(),
            "a press on the band resizes"
        );
        assert_eq!(cursor(&mut harness, row.center()), CursorIcon::PointingHand);
    }

    #[test]
    fn error_banner_link_yields_to_the_left_resize_band() {
        let mut banners = Banners::default();
        let url = format!("https://a.example/{}", "x".repeat(30));
        banners.set_error("notice.offline_detail", vec![("message", url.clone())]);
        let mut harness = sized(banners, vec2(240.0, 200.0), 0.0);
        let link = harness.get_by_label(&url).rect();
        let area = harness.state().area;
        assert!(
            link.min.x < area.min.x + 6.0,
            "link {link:?} must reach the left band"
        );
        let band = egui::pos2(area.min.x + 2.0, link.max.y - 4.0);
        assert_ne!(cursor(&mut harness, band), CursorIcon::PointingHand);
        click_at(&mut harness, band);
        assert!(
            opened_urls(&harness).is_empty(),
            "a press on the band resizes"
        );
        click_at(
            &mut harness,
            egui::pos2(area.min.x + 30.0, link.max.y - 4.0),
        );
        assert_eq!(opened_urls(&harness), [url]);
    }

    #[test]
    fn update_row_hit_area_stays_inside_the_visible_strip() {
        let mut banners = Banners::default();
        banners.set_update(release(&format!("0.2.0-{}", "rc.".repeat(20))));
        let mut harness = sized(banners, vec2(200.0, 90.0), 60.0);
        let label = harness.state().banners.update_text().unwrap();
        let (area, row) = (harness.state().area, harness.get_by_label(&label).rect());
        assert!(row.min.y >= area.min.y + 60.0, "{row:?}");
        click_at(&mut harness, egui::pos2(area.center().x, area.min.y + 30.0));
        assert!(
            opened_urls(&harness).is_empty(),
            "the clipped part must not take clicks"
        );
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
