//! 本地模型頁的「llama.cpp 引擎」卡：狀態、版本與變體選擇、安裝／更新／移除，以及硬體概況。

use eframe::egui::{FontId, Id, Label, RichText, TextStyle, Ui};

use super::dialog::{Choice, confirm_delete};
use super::env::SettingsEnv;
use super::local_card::{padded, section};
use super::local_rate::{ENGINE, Rates, engine_text};
use super::quant_table::{dot, fraction, gb};
use super::theme;
use super::widgets::{
    combo_box, danger_secondary_button, focus_ring, full_text_tip, pill, primary_button,
    progress_bar, secondary_button, skeleton,
};
use crate::i18n::t;
use crate::local::engine::{Build, EngineInstall, Variant, auto_variant, build_number};
use crate::local::usage::Usage;
use crate::local::{Command, EngineState, Snapshot};
use crate::logs::log;

const COMBO_WIDTH: f32 = 170.0;

/// 引擎卡的選擇狀態；沒動過的版本與變體跟著已安裝的，沒裝時版本取最新、變體取自動。
#[derive(Default)]
pub(super) struct EngineCard {
    build: Option<String>,
    /// `Some(None)` 是明確選了「自動」。
    variant: Option<Option<Variant>>,
    confirm_remove: bool,
}

/// 下拉目前選定的版本與變體。
struct Picked {
    build: Option<String>,
    variant: Variant,
    /// 版本清單或硬體還在載入，選定的值之後可能會變。
    loading: bool,
    /// 變體是「自動」。
    auto: bool,
}

impl EngineCard {
    pub(super) fn ui(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        rates: &Rates,
    ) {
        let installed = match &snapshot.engine {
            EngineState::Ready(i) => Some(i),
            _ => None,
        };
        section(
            ui,
            &t("local.engine.title", &[]),
            |ui| status(ui, &snapshot.engine),
            |ui| {
                padded(ui, |ui| {
                    let picked = self.pickers(ui, env, snapshot, installed);
                    self.actions(ui, env, snapshot, installed, &picked, rates);
                    ui.add_space(4.0);
                    hardware(ui, env, snapshot);
                });
            },
        );
        self.confirm(ui, env);
    }

    fn pickers(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        installed: Option<&EngineInstall>,
    ) -> Picked {
        let builds: &[Build] = match &snapshot.builds {
            Some(Ok(builds)) => builds,
            _ => &[],
        };
        let build = self
            .build
            .clone()
            .or_else(|| installed.map(|i| i.build.clone()))
            .or_else(|| builds.first().map(|b| b.tag.clone()));
        let found = build
            .as_deref()
            .and_then(|tag| builds.iter().find(|b| b.tag == tag));
        let auto = auto_variant(snapshot.hardware.gpu.as_ref(), found);
        let (variant, shows_auto) = match (&self.variant, installed) {
            (Some(Some(v)), _) => (v.clone(), false),
            (None, Some(i)) => (i.variant.clone(), false),
            _ => (auto.clone(), true),
        };
        let build_loading = build.is_none() && snapshot.builds.is_none();
        let variant_loading = shows_auto && (snapshot.builds.is_none() || snapshot.detecting);
        let field_height = ui.spacing().interact_size.y;
        let variants: Vec<Variant> = found.map_or_else(
            || vec![Variant::Vulkan, Variant::Cpu],
            |b| b.variants.iter().map(|(v, _)| v.clone()).collect(),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.vertical(|ui| {
                let label = small_label(ui, &t("local.engine.version", &[]));
                if build_loading {
                    skeleton(ui, COMBO_WIDTH, field_height);
                    return;
                }
                let text = match &build {
                    Some(tag) => build_label(tag, found),
                    None => t("local.engine.builds_failed", &[]),
                };
                let combo = combo_box(ui, "engine_build")
                    .width(COMBO_WIDTH)
                    .selected_text(text.clone())
                    .show_ui(ui, |ui| {
                        for b in builds {
                            let on = build.as_deref() == Some(b.tag.as_str());
                            let text = build_label(&b.tag, Some(b));
                            if ui.selectable_label(on, text).clicked() {
                                self.pick_build(b, &variant);
                            }
                        }
                    });
                let response = combo.response.labelled_by(label);
                full_text_tip(ui, &response, &text, 0.0);
                focus_ring(ui, &response, theme::CONTROL_RADIUS);
            });
            ui.vertical(|ui| {
                let label = small_label(ui, &t("local.engine.variant", &[]));
                if variant_loading {
                    skeleton(ui, COMBO_WIDTH, field_height);
                    return;
                }
                let text = if shows_auto {
                    auto_label(&auto)
                } else {
                    variant_label(&variant)
                };
                let combo = combo_box(ui, "engine_variant")
                    .width(COMBO_WIDTH)
                    .selected_text(text.clone())
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(shows_auto, auto_label(&auto)).clicked() {
                            self.variant = Some(None);
                        }
                        for v in &variants {
                            let on = !shows_auto && *v == variant;
                            if ui.selectable_label(on, variant_label(v)).clicked() {
                                self.variant = Some(Some(v.clone()));
                            }
                        }
                    });
                let response = combo.response.labelled_by(label);
                full_text_tip(ui, &response, &text, 0.0);
                focus_ring(ui, &response, theme::CONTROL_RADIUS);
            });
            if let (Some(i), Some(latest)) = (installed, builds.first())
                && newer(&latest.tag, &i.build)
            {
                ui.vertical(|ui| {
                    small_label(ui, " ");
                    let text = t("local.engine.new_build", &[("tag", latest.tag.as_str())]);
                    pill(ui, &text);
                });
            }
        });
        if let Some(Err(error)) = &snapshot.builds {
            ui.horizontal(|ui| {
                ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
                if secondary_button(ui, &t("local.dialog.retry", &[])).clicked() {
                    log("[ui] retry listing llama.cpp builds");
                    env.local_command(Command::ListBuilds);
                }
            });
        }
        Picked {
            build,
            variant,
            loading: build_loading || variant_loading,
            auto: shows_auto,
        }
    }

    /// 換版本時，選定的變體在新 build 裡沒有就退回「自動」。
    fn pick_build(&mut self, build: &Build, variant: &Variant) {
        self.build = Some(build.tag.clone());
        if !build.variants.iter().any(|(v, _)| v == variant) {
            self.variant = Some(None);
        }
    }

    fn actions(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        installed: Option<&EngineInstall>,
        picked: &Picked,
        rates: &Rates,
    ) {
        let latest = match &snapshot.builds {
            Some(Ok(builds)) => builds.first(),
            _ => None,
        };
        let primary = match (&snapshot.engine, installed) {
            (EngineState::Installing { .. } | EngineState::Removing, _) => None,
            _ if picked.loading => None,
            (_, Some(i))
                if picked.build.as_deref() != Some(i.build.as_str())
                    || picked.variant != i.variant =>
            {
                Some(("local.engine.install_selected", picked.build.clone()))
            }
            (_, Some(i)) => latest
                .filter(|b| newer(&b.tag, &i.build))
                .map(|b| ("local.engine.update", Some(b.tag.clone()))),
            (_, None) => Some(("local.engine.install", picked.build.clone())),
        };
        match &snapshot.engine {
            EngineState::Installing {
                done,
                total,
                extracting,
                ..
            } => {
                progress_bar(ui, fraction(*done, *total), ui.available_width());
                let status = engine_text(*done, *total, *extracting, rates.speed(ENGINE));
                let status = RichText::new(status)
                    .size(theme::HINT_SIZE)
                    .color(theme::TEXT_SECONDARY);
                ui.add(Label::new(status).wrap());
            }
            EngineState::Failed(error) => {
                ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
            }
            _ => {}
        }
        ui.horizontal(|ui| {
            let busy = matches!(
                snapshot.engine,
                EngineState::Installing { .. } | EngineState::Removing
            );
            if picked.loading && !busy {
                skeleton(ui, 96.0, ui.spacing().interact_size.y);
            }
            if let Some((key, build)) = primary
                && primary_button(ui, &t(key, &[])).clicked()
            {
                // 「自動」交給 hub 依實際要裝的 build 決定，UI 這邊的版本清單可能還沒有或已過期
                let mut variant = (!picked.auto).then(|| picked.variant.clone());
                if key == "local.engine.update"
                    && let Some(b) = latest
                    && let Some(v) = &variant
                    && !b.variants.iter().any(|(bv, _)| bv == v)
                {
                    log(format!(
                        "[ui] build {} has no {v} variant, updating with the automatic variant",
                        b.tag
                    ));
                    variant = None;
                }
                log(format!(
                    "[ui] engine install requested: build={build:?} variant={variant:?}"
                ));
                env.local_command(Command::InstallEngine { build, variant });
            }
            if installed.is_some()
                && danger_secondary_button(ui, &t("local.engine.remove", &[])).clicked()
            {
                self.confirm_remove = true;
            }
        });
    }

    fn confirm(&mut self, ui: &Ui, env: &dyn SettingsEnv) {
        if !self.confirm_remove {
            return;
        }
        let question = t("local.engine.confirm_remove", &[]);
        match confirm_delete(ui, "confirm_remove_engine", question) {
            Choice::Open => {}
            Choice::Cancel => self.confirm_remove = false,
            Choice::Done(()) => {
                self.confirm_remove = false;
                log("[ui] engine removal requested");
                env.local_command(Command::RemoveEngine);
            }
        }
    }
}

fn newer(candidate: &str, installed: &str) -> bool {
    matches!((build_number(candidate), build_number(installed)), (Some(a), Some(b)) if a > b)
}

/// 下拉上方的小標籤；回傳它的 id，給下拉當無障礙名稱。
fn small_label(ui: &mut Ui, text: &str) -> Id {
    let label = RichText::new(text)
        .font(FontId::proportional(theme::HINT_SIZE))
        .color(theme::TEXT_SECONDARY);
    ui.add(Label::new(label)).id
}

fn build_label(tag: &str, build: Option<&Build>) -> String {
    match build.and_then(|b| b.published.get(..10)) {
        Some(date) => format!("{tag} ({date})"),
        None => tag.to_owned(),
    }
}

fn variant_label(v: &Variant) -> String {
    match v {
        Variant::Vulkan => "Vulkan".into(),
        Variant::Cpu => "CPU".into(),
        Variant::Cuda(version) => format!("CUDA {version}"),
    }
}

fn auto_label(auto: &Variant) -> String {
    t(
        "local.engine.variant_auto",
        &[("variant", &variant_label(auto))],
    )
}

/// 卡片標題右側的狀態徽章。
fn status(ui: &mut Ui, state: &EngineState) {
    let (key, color) = match state {
        EngineState::Missing => ("local.engine.status_missing", theme::TEXT_MUTED),
        EngineState::Installing { .. } => ("local.engine.status_installing", theme::WARN_TEXT),
        EngineState::Removing => ("local.engine.status_removing", theme::WARN_TEXT),
        EngineState::Ready(_) => ("local.engine.status_ready", theme::OK),
        EngineState::Failed(_) => ("local.engine.status_failed", theme::ERROR),
    };
    ui.label(RichText::new(t(key, &[])).color(color));
    dot(ui, color);
}

/// 顯示卡名稱、VRAM 與 RAM 可用量各一行，下面是估算說明與〔重新偵測〕。
fn hardware(ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) {
    let hw = &snapshot.hardware;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        if snapshot.detecting {
            let height = ui.text_style_height(&TextStyle::Body);
            for width in [220.0, 150.0, 150.0] {
                skeleton(ui, width, height);
            }
            return;
        }
        let line = |ui: &mut Ui, text: String| {
            ui.label(RichText::new(text).color(theme::TEXT_SECONDARY));
        };
        let name = hw
            .gpu
            .as_ref()
            .map_or_else(|| t("local.hw.none", &[]), |g| g.name.clone());
        line(ui, t("local.hw.gpu", &[("name", &name)]));
        if hw.vram_total > 0 {
            let (free, total) = (gb(hw.vram_free), gb(hw.vram_total));
            line(
                ui,
                t("local.hw.vram", &[("free", &free), ("total", &total)]),
            );
        } else if hw.gpu.is_some() && !hw.measured {
            let key = match snapshot.engine {
                EngineState::Ready(_) => "local.hw.vram_failed",
                _ => "local.hw.vram_pending",
            };
            line(ui, t(key, &[]));
        }
        let (free, total) = (gb(hw.ram_free), gb(hw.ram_total));
        line(ui, t("local.hw.ram", &[("free", &free), ("total", &total)]));
        if hw.own != Usage::default() {
            let (vram, ram) = (gb(hw.own.vram), gb(hw.own.ram));
            line(ui, t("local.hw.own", &[("vram", &vram), ("ram", &ram)]));
        }
    });
    ui.horizontal_wrapped(|ui| {
        let note = RichText::new(t("local.hw.note", &[]))
            .size(theme::HINT_SIZE)
            .color(theme::TEXT_MUTED);
        ui.add(Label::new(note).wrap());
        if snapshot.detecting {
            return;
        }
        let redetect = RichText::new(t("local.hw.redetect", &[]))
            .size(theme::HINT_SIZE)
            .color(theme::LINK);
        if ui.link(redetect).clicked() {
            log("[ui] hardware re-detect requested");
            env.local_command(Command::RefreshHardware { redetect_gpu: true });
        }
    });
}
