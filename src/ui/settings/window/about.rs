//! 關於分頁：版本與手動檢查更新、專案與開發者連結、譯者、紀錄檔位置與譯文快取。

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    Align, Color32, FontId, Frame, Hyperlink, Id, Label, Rect, RichText, Role, Sense, Ui, Vec2,
    WidgetInfo, pos2,
};

use super::{SettingsAction, SettingsWindow};
use crate::config::{app_dir, app_name};
use crate::i18n::{current_language, t, translators};
use crate::logs::log;
use crate::runtime::updater::{AUTHOR_URL, CROWDIN_URL, ISSUES_URL, PROJECT_URL, Release};
use crate::ui::icon::app_icon_texture;
use crate::ui::palette;
use crate::ui::settings::env::{Round, SettingsEnv};
use crate::ui::settings::theme::{self, icon};
use crate::ui::settings::widgets::{
    card, centered_column, outcome, rich, right_aligned, right_aligned_width, secondary_button,
    secondary_icon_button, setting_row, setting_row_with,
};

/// 關於分頁自己的狀態：檢查更新的輪次與結果與清除快取的結果。
pub(super) struct AboutState {
    update: Round,
    /// 最近一次檢查更新的結果：`Ok(None)` 已是最新、`Ok(Some)` 有新版、`Err` 失敗原因。
    result: Option<Result<Option<Release>, String>>,
    /// 最近一次清除快取的結果：`Some(None)` 是刪檔失敗。
    cache_cleared: Option<Option<usize>>,
}

impl Default for AboutState {
    fn default() -> Self {
        Self {
            update: Round::new("update check"),
            result: None,
            cache_cleared: None,
        }
    }
}

/// 頭卡圖示的邊長。
const HERO_ICON: f32 = 48.0;
/// 頭卡圖示與文字欄的間距。
const HERO_GAP: f32 = 16.0;

/// 外部連結：文字後面接一個外部連結圖示；點下去以系統瀏覽器開啟。
fn link(ui: &mut Ui, text: &str, url: &str) {
    // 圖示排在同一段文字裡：網址太長換行時圖示跟著最後一行，不會被擠出卡片
    let format = |font_id| TextFormat {
        font_id,
        color: theme::LINK,
        valign: Align::Center,
        ..TextFormat::default()
    };
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        format(FontId::proportional(theme::CONTROL_TEXT_SIZE)),
    );
    job.append(
        &icon::EXTERNAL_LINK.to_string(),
        6.0,
        format(theme::icons(11.0)),
    );
    let response = ui.add(Hyperlink::from_label_and_url(job, url));
    // 無障礙名稱不含圖示字元
    response.widget_info(|| WidgetInfo::labeled(Role::Link, true, text));
    if response.clicked() {
        log(format!("[ui] opening link from settings: {url}"));
    }
}

/// 網址去掉 `https://` 當連結文字。
fn bare(url: &str) -> &str {
    url.trim_start_matches("https://")
}

impl SettingsWindow {
    /// 手動檢查更新的結果；查到新版時另外送出 `UpdateFound`，讓 overlay 也掛橫幅。
    pub(super) fn apply_update(&mut self, round: u64, result: &Result<Option<Release>, String>) {
        if !self.about.update.finish(round) {
            return;
        }
        match result {
            Ok(None) => log("[update] manual check: already up to date"),
            Ok(Some(release)) => {
                log(format!(
                    "[update] manual check: {} available",
                    release.version
                ));
                self.pending
                    .push(SettingsAction::UpdateFound(release.clone()));
            }
            // 失敗原因已由 env 記過
            Err(_) => {}
        }
        self.about.result = Some(result.clone());
    }

    pub(super) fn about_page(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        self.hero_card(ui, env);
        card(ui, |ui| {
            setting_row(ui, &t("about.project", &[]), None, |ui| {
                link(ui, bare(PROJECT_URL), PROJECT_URL);
            });
            // 開發者名稱是識別碼不是文案，不進語言檔
            setting_row(ui, &t("about.author", &[]), None, |ui| {
                link(ui, "GoneTone", AUTHOR_URL);
            });
            let credit = translators(&current_language());
            if !credit.is_empty() {
                setting_row(ui, &t("credit.translators", &[]), None, |ui| {
                    ui.vertical(|ui| rich(ui, &credit, theme::CONTROL_TEXT_SIZE, theme::TEXT));
                });
            }
            setting_row(ui, &t("about.help_translate", &[]), None, |ui| {
                link(ui, bare(CROWDIN_URL), CROWDIN_URL);
            });
            setting_row(ui, &t("about.issues", &[]), None, |ui| {
                link(ui, bare(ISSUES_URL), ISSUES_URL);
            });
        });
        card(ui, |ui| {
            let path = app_dir().display().to_string();
            let details = |ui: &mut Ui| {
                let font = FontId::monospace(theme::HINT_SIZE);
                ui.add(Label::new(RichText::new(&path).font(font).color(palette::FG_BAR)).wrap());
                rich(
                    ui,
                    &t("about.logs_hint", &[]),
                    theme::HINT_SIZE,
                    theme::TEXT_SECONDARY,
                );
            };
            setting_row_with(ui, &t("about.logs", &[]), details, |ui| {
                let text = t("button.open_folder", &[]);
                if secondary_icon_button(ui, icon::OPEN_FOLDER, &text).clicked() {
                    env.open_folder(&app_dir());
                }
            });
            if env.has_cache() {
                let hint = t("about.cache_hint", &[]);
                setting_row(ui, &t("about.cache", &[]), Some(&hint), |ui| {
                    if let Some(cleared) = self.about.cache_cleared {
                        match cleared {
                            Some(count) => {
                                let count = count.to_string();
                                outcome(ui, true, &t("about.cache_cleared", &[("count", &count)]));
                            }
                            None => outcome(ui, false, &t("about.cache_clear_failed", &[])),
                        }
                        ui.add_space(8.0);
                    }
                    // 不加確認：快取會自動重建，誤按的代價只是下一則同樣的系統訊息重翻一次
                    if secondary_button(ui, &t("button.clear_cache", &[])).clicked() {
                        self.about.cache_cleared = Some(env.clear_cache());
                    }
                });
            }
        });
    }

    /// 頭卡：圖示、名稱與版本，〔檢查更新〕靠右；檢查結果另起一列、對齊名稱，名稱與版本不因結果出現而移動。
    fn hero_card(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        let texture = app_icon_texture(ui.ctx(), 48);
        let buttons = Id::unique("about_hero_buttons");
        let mut check = false;
        card(ui, |ui| {
            Frame::new().inner_margin(16).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = HERO_GAP;
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(HERO_ICON), Sense::hover());
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    ui.painter().image(texture, rect, uv, Color32::WHITE);
                    let room = ui.available_width() - right_aligned_width(ui, buttons) - HERO_GAP;
                    let column = Id::unique("about_hero_text");
                    centered_column(ui, column, room.max(80.0), |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        self.hero_text(ui);
                    });
                    check = right_aligned(ui, buttons, |ui| {
                        let busy = self.about.update.busy();
                        let text = if busy {
                            t("button.checking", &[])
                        } else {
                            t("button.check_update", &[])
                        };
                        ui.add_enabled_ui(!busy, |ui| {
                            secondary_icon_button(ui, icon::CHECK_UPDATE, &text)
                        })
                        .inner
                        .clicked()
                    });
                });
                if self.about.result.is_some() {
                    ui.add_space(10.0);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(HERO_ICON + HERO_GAP);
                        ui.vertical(|ui| self.update_result(ui));
                    });
                }
            });
        });
        if check {
            self.about.result = None;
            let round = self.about.update.start();
            env.check_update(round);
        }
    }

    fn hero_text(&self, ui: &mut Ui) {
        ui.add(Label::new(
            RichText::new(app_name()).font(theme::bold(17.0)),
        ));
        let version = format!("{} v{}", t("about.version", &[]), env!("CARGO_PKG_VERSION"));
        ui.add(Label::new(
            RichText::new(version)
                .size(theme::HINT_SIZE)
                .color(theme::TEXT_SECONDARY),
        ));
    }

    fn update_result(&self, ui: &mut Ui) {
        match &self.about.result {
            None => {}
            Some(Ok(None)) => outcome(ui, true, &t("update.latest", &[])),
            Some(Err(error)) => outcome(ui, false, &t("update.failed", &[("error", error)])),
            Some(Ok(Some(release))) => {
                // 有新版：整行文字是連結，用連結藍、不加成功／失敗圖示
                let text = t("update.available", &[("version", &release.version)]);
                let text = RichText::new(text)
                    .size(theme::CONTROL_TEXT_SIZE)
                    .color(theme::LINK);
                if ui
                    .add(Hyperlink::from_label_and_url(text, &release.url))
                    .clicked()
                {
                    log(format!("[update] opening release page: {}", release.url));
                }
            }
        }
    }
}
