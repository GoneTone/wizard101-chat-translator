//! 「下載模型」對話框：推薦與自訂分頁、放不下的確認、精靈用的選擇模式。

use crate::common;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Event, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use common::kittest::{click, click_in_dialog, has_label, painted_texts, run, themed_harness, tr};
use common::settings_env::FakeEnv;
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::local::catalog::RECOMMENDED;
use wizard101_chat_translator::local::fit::{Fit, Pick};
use wizard101_chat_translator::local::gguf::ModelMeta;
use wizard101_chat_translator::local::hardware::{Gpu, GpuKind, Hardware};
use wizard101_chat_translator::local::hf::{Catalog, Choice, MtpChoice, Quant, RepoFile};
use wizard101_chat_translator::local::store::{Installed, Manifest, choice_id};
use wizard101_chat_translator::local::{Command, RepoInfo, RepoState, Snapshot};
use wizard101_chat_translator::ui::settings::env::SettingsEnv;
use wizard101_chat_translator::ui::settings::local_dialog::{
    DialogMode, DialogOutcome, DownloadDialog, Selection, Tab,
};

const LANGUAGE: &str = "en-US";
const G: u64 = 1 << 30;

struct State {
    dialog: DownloadDialog,
    env: FakeEnv,
    outcome: DialogOutcome,
}

fn choice(label: &str, bits: u8, size: u64) -> Choice {
    Choice {
        quant: Quant {
            label: label.into(),
            bits: Some(bits),
        },
        files: vec![RepoFile {
            path: format!("{label}.gguf"),
            size,
            sha256: None,
        }],
    }
}

/// 兩個量化（或一個）的 repo；自動挑第一個，等級為 `fit`。
fn info(fit: Fit, moe: bool, choices: usize, required: u64) -> RepoInfo {
    let all = [choice("Q4_K_M", 4, 2 * G), choice("Q8_0", 8, 4 * G)];
    RepoInfo {
        catalog: Catalog {
            choices: all[..choices].to_vec(),
            mtp: vec![],
        },
        meta: ModelMeta {
            expert_count: if moe { 8 } else { 0 },
            ..Default::default()
        },
        fits: vec![(required, Some(fit)); choices],
        mtp_required: vec![],
        auto: Some(Pick {
            choice: 0,
            mtp: None,
            fit,
            required,
        }),
    }
}

/// 推薦清單全部載入中，`ready` 內的 repo 換成查好的結果。
fn snapshot(ready: &[(usize, RepoInfo)]) -> Snapshot {
    let mut s = Snapshot {
        hardware: Hardware {
            gpu: Some(Gpu {
                name: "t".into(),
                vendor: 0,
                kind: GpuKind::Discrete,
            }),
            vram_total: 8 * G,
            vram_free: 8 * G,
            ram_total: 16 * G,
            ram_free: 16 * G,
            measured: true,
        },
        ..Default::default()
    };
    for r in RECOMMENDED {
        s.repos.insert(r.repo.into(), RepoState::Loading);
    }
    for (i, info) in ready {
        s.repos
            .insert(RECOMMENDED[*i].repo.into(), RepoState::Ready(info.clone()));
    }
    s
}

fn harness(mode: DialogMode, tab: Tab, snap: Snapshot) -> Harness<'static, State> {
    i18n::set_language(LANGUAGE);
    let env = FakeEnv::default();
    *env.snapshot.borrow_mut() = snap;
    let state = State {
        dialog: DownloadDialog::new(mode, tab),
        env,
        outcome: DialogOutcome::Open,
    };
    themed_harness(
        egui::vec2(760.0, 700.0),
        LANGUAGE,
        |ui: &mut Ui, s: &mut State| {
            let snap = s.env.local_snapshot();
            let outcome = s.dialog.ui(ui, &s.env, &snap);
            if outcome != DialogOutcome::Open {
                s.outcome = outcome;
            }
        },
        state,
    )
}

fn size_arg(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / G as f64)
}

#[test]
fn recommended_rows_show_memory_kind_not_tier() {
    let snap = snapshot(&[
        (0, info(Fit::Smooth, false, 2, 3 * G)),
        (4, info(Fit::Offload, true, 2, 18 * G)),
    ]);
    let h = harness(DialogMode::Download, Tab::Recommended, snap);
    let texts = painted_texts(&h);
    let vram = t("local.memory.vram", &[("size", &size_arg(3 * G))]);
    let both = t("local.memory.vram_ram", &[("size", &size_arg(18 * G))]);
    assert!(texts.contains(&vram), "{texts:?}");
    assert!(texts.contains(&both), "{texts:?}");
    assert!(!include_str!("../../src/i18n/locales/en-US.json").contains("local.tier."));
    let queried = h.state().env.commands.borrow().len();
    assert_eq!(
        queried, 0,
        "every recommended repo is already in the snapshot"
    );
}

#[test]
fn opening_queries_only_repos_missing_from_the_snapshot() {
    let mut snap = snapshot(&[]);
    snap.repos.remove(RECOMMENDED[0].repo);
    let h = harness(DialogMode::Download, Tab::Recommended, snap);
    let commands = h.state().env.commands.borrow();
    assert!(
        matches!(&commands[..], [Command::QueryRepo(r)] if r == RECOMMENDED[0].repo),
        "{commands:?}"
    );
}

#[test]
fn single_quant_repo_disables_choose_version() {
    let snap = snapshot(&[(0, info(Fit::Smooth, false, 1, 3 * G))]);
    let h = harness(DialogMode::Download, Tab::Recommended, snap);
    let label = tr("local.dialog.choose_version");
    let choose = h.get_by_role_and_label(Role::Button, &label);
    assert!(choose.accesskit_node().is_disabled());
}

#[test]
fn custom_tab_rejects_non_hf_input() {
    let mut h = harness(DialogMode::Download, Tab::Custom, snapshot(&[]));
    h.get_by_role_and_label(Role::TextInput, &tr("local.dialog.repo_hint"))
        .click();
    h.event(Event::Text("https://example.com/a/b".into()));
    run(&mut h);
    click(&mut h, Role::Button, &tr("local.dialog.query"));
    assert!(has_label(&h, &tr("local.dialog.bad_repo")));
    assert!(h.state().env.commands.borrow().is_empty());
}

#[test]
fn nofit_download_asks_first() {
    let snap = snapshot(&[(0, info(Fit::NoFit, false, 2, 40 * G))]);
    let mut h = harness(DialogMode::Download, Tab::Recommended, snap);
    click(&mut h, Role::Button, &tr("local.dialog.download"));
    assert!(has_label(&h, &tr("local.dialog.nofit_confirm")));
    assert!(h.state().env.commands.borrow().is_empty());
    click_in_dialog(
        &mut h,
        &tr("dialog.confirm_title"),
        &tr("local.dialog.download"),
    );
    let commands = h.state().env.commands.borrow();
    assert!(
        matches!(&commands[..], [Command::Download { repo, choice: 0, mtp: None, auto: true }] if repo == RECOMMENDED[0].repo),
        "{commands:?}"
    );
    assert_eq!(h.state().outcome, DialogOutcome::Closed);
}

#[test]
fn select_mode_returns_the_choice() {
    let snap = snapshot(&[(0, info(Fit::Smooth, false, 2, 3 * G))]);
    let mut h = harness(DialogMode::Select, Tab::Recommended, snap);
    click(&mut h, Role::Button, &tr("local.dialog.select"));
    assert_eq!(
        h.state().outcome,
        DialogOutcome::Selected(Selection {
            repo: RECOMMENDED[0].repo.into(),
            choice: 0,
            mtp: None,
            auto: true,
        })
    );
    assert!(h.state().env.commands.borrow().is_empty());
}

#[test]
fn picking_a_quant_chip_changes_the_selection() {
    let snap = snapshot(&[(0, info(Fit::Smooth, false, 2, 3 * G))]);
    let mut h = harness(DialogMode::Select, Tab::Recommended, snap);
    click(&mut h, Role::Button, &tr("local.dialog.choose_version"));
    click(&mut h, Role::RadioButton, "Q8_0");
    click(&mut h, Role::Button, &tr("local.dialog.select"));
    assert!(matches!(
        &h.state().outcome,
        DialogOutcome::Selected(Selection {
            choice: 1,
            auto: false,
            ..
        })
    ));
}

fn with_mtp(mut info: RepoInfo, auto_mtp: Option<usize>, extra: u64) -> RepoInfo {
    info.catalog.mtp = vec![MtpChoice {
        quant: Quant {
            label: "Q4_0".into(),
            bits: Some(4),
        },
        file: RepoFile {
            path: "mtp.gguf".into(),
            size: G / 2,
            sha256: None,
        },
        shared: false,
    }];
    info.mtp_required = vec![extra];
    info.auto.as_mut().unwrap().mtp = auto_mtp;
    info
}

fn downloads(h: &Harness<State>) -> Vec<Command> {
    h.state().env.commands.borrow().clone()
}

#[test]
fn custom_tab_queries_the_parsed_repo_then_downloads_the_selection() {
    let mut snap = snapshot(&[]);
    snap.repos.insert(
        "org/repo".into(),
        RepoState::Ready(info(Fit::Smooth, false, 2, 3 * G)),
    );
    let mut h = harness(DialogMode::Download, Tab::Custom, snap);
    h.get_by_role_and_label(Role::TextInput, &tr("local.dialog.repo_hint"))
        .click();
    h.event(Event::Text(
        "https://huggingface.co/org/repo/tree/main".into(),
    ));
    run(&mut h);
    click(&mut h, Role::Button, &tr("local.dialog.query"));
    assert!(matches!(&downloads(&h)[..], [Command::QueryRepo(r)] if r == "org/repo"));
    click(&mut h, Role::Button, &tr("local.dialog.download_selected"));
    assert!(matches!(
        &downloads(&h)[..],
        [Command::QueryRepo(_), Command::Download { repo, choice: 0, mtp: None, auto: true }] if repo == "org/repo"
    ));
}

#[test]
fn mtp_chips_change_the_download() {
    let base = with_mtp(info(Fit::Smooth, false, 2, 3 * G), Some(0), G);
    let mut h = harness(
        DialogMode::Download,
        Tab::Recommended,
        snapshot(&[(0, base)]),
    );
    click(&mut h, Role::Button, &tr("local.dialog.choose_version"));
    click(&mut h, Role::RadioButton, &tr("local.quant.mtp_none"));
    click(&mut h, Role::Button, &tr("local.dialog.download"));
    assert!(matches!(
        &downloads(&h)[..],
        [Command::Download { mtp: None, .. }]
    ));
    let base = with_mtp(info(Fit::Smooth, false, 2, 3 * G), None, G);
    let mut h = harness(
        DialogMode::Download,
        Tab::Recommended,
        snapshot(&[(0, base)]),
    );
    click(&mut h, Role::Button, &tr("local.dialog.choose_version"));
    click(&mut h, Role::RadioButton, "MTP Q4_0");
    click(&mut h, Role::Button, &tr("local.dialog.download"));
    assert!(matches!(
        &downloads(&h)[..],
        [Command::Download { mtp: Some(0), .. }]
    ));
}

#[test]
fn an_mtp_that_overflows_memory_asks_before_downloading() {
    let base = with_mtp(info(Fit::Smooth, false, 2, 3 * G), Some(0), 40 * G);
    let mut h = harness(
        DialogMode::Download,
        Tab::Recommended,
        snapshot(&[(0, base)]),
    );
    click(&mut h, Role::Button, &tr("local.dialog.download"));
    assert!(has_label(&h, &tr("local.dialog.nofit_confirm")));
    assert!(downloads(&h).is_empty());
}

#[test]
fn failed_rows_can_retry() {
    let mut snap = snapshot(&[]);
    snap.repos.insert(
        RECOMMENDED[0].repo.into(),
        RepoState::Failed("HTTP 500".into()),
    );
    let mut h = harness(DialogMode::Download, Tab::Recommended, snap);
    assert!(has_label(&h, "HTTP 500"));
    click(&mut h, Role::Button, &tr("local.dialog.retry"));
    assert!(matches!(&downloads(&h)[..], [Command::QueryRepo(r)] if r == RECOMMENDED[0].repo));
}

#[test]
fn only_complete_models_count_as_downloaded() {
    let mut snap = snapshot(&[(0, info(Fit::Smooth, false, 2, 3 * G))]);
    snap.models.push(Installed {
        id: choice_id(RECOMMENDED[0].repo, &choice("Q4_K_M", 4, 2 * G)),
        manifest: Manifest {
            repo: RECOMMENDED[0].repo.into(),
            quant: "Q4_K_M".into(),
            files: vec![],
            mtp: None,
            moe: false,
            has_builtin_mtp: false,
            required: 0,
        },
        complete: false,
    });
    let mut h = harness(DialogMode::Download, Tab::Recommended, snap);
    let label = tr("local.in_progress");
    assert!(
        h.get_by_role_and_label(Role::Button, &label)
            .accesskit_node()
            .is_disabled()
    );
    h.state().env.snapshot.borrow_mut().models[0].complete = true;
    run(&mut h);
    let label = tr("local.downloaded");
    let done = h.get_by_role_and_label(Role::Button, &label);
    assert!(done.accesskit_node().is_disabled());
}

#[test]
fn the_basis_line_shows_available_memory() {
    let h = harness(
        DialogMode::Download,
        Tab::Recommended,
        snapshot(&[(0, info(Fit::Smooth, false, 2, 3 * G))]),
    );
    let line = t(
        "local.dialog.basis",
        &[("vram", &size_arg(8 * G)), ("ram", &size_arg(16 * G))],
    );
    assert!(painted_texts(&h).contains(&line));
}

/// 同名量化放在不同資料夾時，只有真的下載過的那一個算已下載（依模型 id 比對）。
#[test]
fn same_label_quants_in_different_folders_are_told_apart() {
    let mut info = info(Fit::Smooth, false, 2, 3 * G);
    for (c, dir) in info.catalog.choices.iter_mut().zip(["a", "b"]) {
        c.quant.label = "Q4_K_M".into();
        c.files[0].path = format!("{dir}/model-Q4_K_M.gguf");
    }
    let other = choice_id(RECOMMENDED[0].repo, &info.catalog.choices[1]);
    let mut snap = snapshot(&[(0, info)]);
    snap.models.push(Installed {
        id: other,
        manifest: Manifest {
            repo: RECOMMENDED[0].repo.into(),
            quant: "Q4_K_M".into(),
            files: vec![],
            mtp: None,
            moe: false,
            has_builtin_mtp: false,
            required: 0,
        },
        complete: true,
    });
    let h = harness(DialogMode::Download, Tab::Recommended, snap);
    assert!(
        h.query_by_role_and_label(Role::Button, &tr("local.downloaded"))
            .is_none()
    );
    assert!(
        !h.get_by_role_and_label(Role::Button, &tr("local.dialog.download"))
            .accesskit_node()
            .is_disabled()
    );
}

/// 動過量化表的選擇以手動選擇送出（hub 不會改挑）。
#[test]
fn a_manually_picked_quant_downloads_as_a_manual_choice() {
    let snap = snapshot(&[(0, info(Fit::Smooth, false, 2, 3 * G))]);
    let mut h = harness(DialogMode::Download, Tab::Recommended, snap);
    click(&mut h, Role::Button, &tr("local.dialog.choose_version"));
    click(&mut h, Role::RadioButton, "Q8_0");
    click(&mut h, Role::Button, &tr("local.dialog.download"));
    assert!(matches!(
        &downloads(&h)[..],
        [Command::Download {
            choice: 1,
            auto: false,
            ..
        }]
    ));
}

/// 只有 CPU（VRAM 量到 0）依 RAM 判斷；還沒量過才是「無法判斷」。
#[test]
fn the_basis_line_follows_whether_vram_was_measured() {
    let cpu_only = |measured: bool| {
        let mut snap = snapshot(&[(0, info(Fit::Slow, false, 2, 3 * G))]);
        snap.hardware = Hardware {
            ram_total: 16 * G,
            ram_free: 16 * G,
            measured,
            ..Hardware::default()
        };
        harness(DialogMode::Download, Tab::Recommended, snap)
    };
    let ram = t("local.dialog.basis_ram", &[("ram", &size_arg(16 * G))]);
    assert!(painted_texts(&cpu_only(true)).contains(&ram));
    let unknown = tr("local.dialog.basis_unknown");
    assert!(painted_texts(&cpu_only(false)).contains(&unknown));
}

mod page {
    //! 設定視窗的「本地模型」分頁：卡片順序、草稿寫入、檔案操作的立即生效與刪除確認。

    use crate::common;

    use eframe::egui::accesskit::{Role, Toggled};
    use eframe::egui::{self, Ui};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    use common::kittest::{click, click_in_dialog, has_label, run, themed_harness, tr};
    use common::settings_env::FakeEnv;
    use common::settings_window::{State as WindowState, custom_cfg, harness as window_harness};
    use wizard101_chat_translator::config::services::new_service;
    use wizard101_chat_translator::i18n::{self, t};
    use wizard101_chat_translator::local::engine::{Build, EngineInstall, Variant};
    use wizard101_chat_translator::local::hardware::{Gpu, GpuKind, Hardware};
    use wizard101_chat_translator::local::store::{Installed, Manifest, ManifestFile};
    use wizard101_chat_translator::local::{Command, DownloadState, EngineState, Snapshot};
    use wizard101_chat_translator::ui::settings::env::SettingsEnv;
    use wizard101_chat_translator::ui::settings::local_pane::LocalPane;
    use wizard101_chat_translator::ui::settings::window::SettingsAction;

    const LANGUAGE: &str = "en-US";
    const MODEL: &str = "org__repo__Q4_K_M";

    fn model(id: &str, complete: bool, mtp: bool) -> Installed {
        Installed {
            id: id.into(),
            manifest: Manifest {
                repo: "org/repo".into(),
                quant: "Q4_K_M".into(),
                files: vec![],
                mtp: None,
                moe: false,
                has_builtin_mtp: mtp,
                required: 0,
            },
            complete,
        }
    }

    fn snapshot(models: Vec<Installed>) -> Snapshot {
        Snapshot {
            models,
            ..Default::default()
        }
    }

    struct State {
        pane: LocalPane,
        env: FakeEnv,
        services: Vec<Value>,
        now: Instant,
    }

    fn pane_harness(snap: Snapshot, services: Vec<Value>) -> Harness<'static, State> {
        i18n::set_language(LANGUAGE);
        let env = FakeEnv::default();
        *env.snapshot.borrow_mut() = snap;
        let state = State {
            pane: LocalPane::new(&custom_cfg()),
            env,
            services,
            now: Instant::now(),
        };
        themed_harness(
            egui::vec2(760.0, 1000.0),
            LANGUAGE,
            |ui: &mut Ui, s: &mut State| {
                let snap = s.env.local_snapshot();
                s.pane.ui_at(ui, &s.env, &snap, &s.services, s.now);
            },
            state,
        )
    }

    fn window(snap: Snapshot) -> Harness<'static, WindowState> {
        i18n::set_language(LANGUAGE);
        let env = FakeEnv::default();
        *env.snapshot.borrow_mut() = snap;
        let mut h = window_harness(&custom_cfg(), env, egui::vec2(840.0, 1400.0), LANGUAGE);
        click(&mut h, Role::Tab, &tr("settings.tab.local"));
        h
    }

    fn commands<S>(env: impl Fn(&S) -> &FakeEnv, h: &Harness<S>) -> Vec<Command> {
        env(h.state()).commands.borrow().clone()
    }

    fn deletes(commands: &[Command]) -> usize {
        commands
            .iter()
            .filter(|c| matches!(c, Command::Delete(_)))
            .count()
    }

    #[test]
    fn page_order_puts_load_settings_above_installed_models() {
        let h = window(snapshot(vec![model(MODEL, true, false)]));
        let top = |label: &str| h.get_by_label(label).rect().top();
        let engine = top(&tr("local.engine.title"));
        let preload = h
            .get_by_role_and_label(Role::Switch, &tr("local.load.preload"))
            .rect()
            .top();
        let models = top(&tr("local.models.title"));
        assert!(
            engine < preload && preload < models,
            "{engine} {preload} {models}"
        );
    }

    #[test]
    fn preload_and_idle_go_into_the_saved_config() {
        let mut h = window(Snapshot::default());
        click(&mut h, Role::Switch, &tr("local.load.preload"));
        click(&mut h, Role::ComboBox, &tr("local.load.idle"));
        let thirty = t("local.load.idle_minutes", &[("minutes", "30")]);
        click(&mut h, Role::Button, &thirty);
        click(&mut h, Role::Button, &tr("button.save"));
        let saved = h.state().actions.iter().find_map(|a| match a {
            SettingsAction::Save(cfg) => Some(cfg.clone()),
            _ => None,
        });
        assert_eq!(
            saved.unwrap()["local_models"],
            json!({"preload": false, "idle_minutes": 30, "mtp": {}})
        );
    }

    #[test]
    fn disk_actions_apply_immediately_even_if_cancelled() {
        let mut h = window(snapshot(vec![model(MODEL, true, false)]));
        click(&mut h, Role::Button, &tr("button.delete"));
        let sent = commands(|s: &WindowState| &s.env, &h);
        assert!(matches!(&sent[..], [Command::ListBuilds, Command::Delete(id)] if id == MODEL));
        click(&mut h, Role::Button, &tr("button.cancel"));
        assert_eq!(commands(|s: &WindowState| &s.env, &h).len(), 2);
        assert!(h.state().actions.contains(&SettingsAction::Cancel));
    }

    #[test]
    fn deleting_a_model_in_use_asks_first() {
        let mut service = new_service("local", &[]);
        service["model"] = json!(MODEL);
        let name = service["name"].as_str().unwrap().to_owned();
        let snap = snapshot(vec![model(MODEL, true, false)]);
        let mut h = pane_harness(snap, vec![service]);
        h.state().env.snapshot.borrow_mut().models[0].manifest.files = vec![ManifestFile {
            path: "m.gguf".into(),
            size: 3 << 30,
            sha256: None,
        }];
        run(&mut h);
        click(&mut h, Role::Button, &tr("button.delete"));
        assert_eq!(deletes(&commands(|s: &State| &s.env, &h)), 0);
        let size = format!("{:.1}", (3u64 << 30) as f64 / (1u64 << 30) as f64);
        let question = t(
            "local.models.confirm_delete",
            &[("services", &name), ("size", &size)],
        );
        assert!(has_label(&h, &question));
        click_in_dialog(&mut h, &tr("dialog.confirm_title"), &tr("button.delete"));
        assert_eq!(deletes(&commands(|s: &State| &s.env, &h)), 1);
    }

    #[test]
    fn paused_download_offers_continue() {
        let mut snap = snapshot(vec![model(MODEL, false, false)]);
        snap.downloads
            .insert(MODEL.into(), DownloadState::Paused { done: 1, total: 2 });
        let mut h = pane_harness(snap, vec![]);
        click(&mut h, Role::Button, &tr("local.continue_download"));
        let sent = commands(|s: &State| &s.env, &h);
        assert!(matches!(sent.last(), Some(Command::Resume(id)) if id == MODEL));
    }

    #[test]
    fn a_newer_build_offers_update() {
        let build = |tag: &str| Build {
            tag: tag.into(),
            published: "2026-10-26T00:00:00Z".into(),
            variants: vec![(Variant::Vulkan, vec![])],
        };
        let mut snap = Snapshot {
            engine: EngineState::Ready(EngineInstall {
                build: "b11871".into(),
                variant: Variant::Vulkan,
            }),
            builds: Some(Ok(vec![build("b11880"), build("b11871")])),
            ..Default::default()
        };
        snap.hardware.gpu = None;
        let mut h = pane_harness(snap, vec![]);
        assert!(has_label(
            &h,
            &t("local.engine.new_build", &[("tag", "b11880")])
        ));
        click(&mut h, Role::Button, &tr("local.engine.update"));
        let sent = commands(|s: &State| &s.env, &h);
        assert!(matches!(
            sent.last(),
            Some(Command::InstallEngine { build: Some(b), variant: Some(Variant::Vulkan) }) if b == "b11880"
        ));
    }

    #[test]
    fn the_empty_state_link_opens_the_download_dialog() {
        let mut h = pane_harness(Snapshot::default(), vec![]);
        click(&mut h, Role::Link, &tr("local.models.empty_link"));
        run(&mut h);
        assert!(has_label(&h, &tr("local.dialog.title")));
    }

    fn ready_engine() -> Snapshot {
        let build = |tag: &str| Build {
            tag: tag.into(),
            published: "2026-10-26T00:00:00Z".into(),
            variants: vec![(Variant::Vulkan, vec![]), (Variant::Cpu, vec![])],
        };
        Snapshot {
            engine: EngineState::Ready(EngineInstall {
                build: "b11871".into(),
                variant: Variant::Vulkan,
            }),
            builds: Some(Ok(vec![build("b11880"), build("b11871")])),
            ..Default::default()
        }
    }

    fn last(h: &Harness<State>) -> Option<Command> {
        h.state().env.commands.borrow().last().cloned()
    }

    #[test]
    fn the_mtp_toggle_defaults_on_and_is_saved_per_model() {
        let models = vec![model(MODEL, true, true), model("plain", true, false)];
        let mut h = window(snapshot(models));
        let mtp = h
            .query_all_by_role(Role::Switch)
            .filter(|n| {
                n.accesskit_node()
                    .label()
                    .is_some_and(|l| l.starts_with("MTP"))
            })
            .count();
        assert_eq!(mtp, 1, "only the model with MTP gets a toggle");
        let label = "MTP repo · Q4_K_M";
        let on = h.get_by_role_and_label(Role::Switch, label);
        assert_eq!(on.accesskit_node().toggled(), Some(Toggled::True));
        click(&mut h, Role::Switch, label);
        click(&mut h, Role::Button, &tr("button.save"));
        let saved = h.state().actions.iter().find_map(|a| match a {
            SettingsAction::Save(cfg) => Some(cfg.clone()),
            _ => None,
        });
        assert_eq!(
            saved.unwrap()["local_models"]["mtp"],
            json!({ MODEL: false })
        );
    }

    #[test]
    fn a_changed_variant_installs_the_selected_version() {
        let mut h = pane_harness(ready_engine(), vec![]);
        let install = tr("local.engine.install_selected");
        assert!(h.query_by_role_and_label(Role::Button, &install).is_none());
        click(&mut h, Role::ComboBox, &tr("local.engine.variant"));
        click(&mut h, Role::Button, "CPU");
        click(&mut h, Role::Button, &install);
        assert!(matches!(
            last(&h),
            Some(Command::InstallEngine { build: Some(b), variant: Some(Variant::Cpu) }) if b == "b11871"
        ));
    }

    #[test]
    fn an_update_without_the_installed_variant_falls_back_to_auto() {
        let mut snap = ready_engine();
        snap.engine = EngineState::Ready(EngineInstall {
            build: "b11871".into(),
            variant: Variant::Cpu,
        });
        snap.builds = Some(Ok(vec![
            Build {
                tag: "b11880".into(),
                published: "2026-10-26T00:00:00Z".into(),
                variants: vec![(Variant::Vulkan, vec![])],
            },
            Build {
                tag: "b11871".into(),
                published: "2026-10-25T00:00:00Z".into(),
                variants: vec![(Variant::Cpu, vec![])],
            },
        ]));
        snap.hardware.gpu = Some(Gpu {
            name: "t".into(),
            vendor: 0,
            kind: GpuKind::Discrete,
        });
        let mut h = pane_harness(snap, vec![]);
        click(&mut h, Role::Button, &tr("local.engine.update"));
        assert!(matches!(
            last(&h),
            Some(Command::InstallEngine { build: Some(b), variant: Some(Variant::Vulkan) }) if b == "b11880"
        ));
    }

    /// 版本清單已列過就不再向 GitHub 要（匿名額度有限）；還沒有時列一次。
    #[test]
    fn builds_are_listed_only_when_missing() {
        let listed = |h: &Harness<State>| {
            commands(|s: &State| &s.env, h)
                .iter()
                .filter(|c| matches!(c, Command::ListBuilds))
                .count()
        };
        let mut h = pane_harness(ready_engine(), vec![]);
        run(&mut h);
        assert_eq!(listed(&h), 0);
        let mut h = pane_harness(Snapshot::default(), vec![]);
        run(&mut h);
        assert_eq!(listed(&h), 1);
    }

    /// 移除引擎途中狀態是「移除中」，不給安裝或移除鈕。
    #[test]
    fn a_removing_engine_shows_its_state_without_actions() {
        let mut snap = ready_engine();
        snap.engine = EngineState::Removing;
        let h = pane_harness(snap, vec![]);
        assert!(has_label(&h, &tr("local.engine.status_removing")));
        for key in [
            "local.engine.install",
            "local.engine.install_selected",
            "local.engine.update",
            "local.engine.remove",
        ] {
            assert!(
                h.query_by_role_and_label(Role::Button, &tr(key)).is_none(),
                "{key}"
            );
        }
    }

    /// 已下載模型的相容等級與對話框同一個「量過沒」判準：只有 CPU 量過就有等級。
    #[test]
    fn downloaded_models_are_rated_once_vram_was_measured() {
        let card = |measured: bool| {
            let mut m = model(MODEL, true, false);
            m.manifest.required = 1 << 30;
            let mut snap = snapshot(vec![m]);
            snap.hardware = Hardware {
                ram_total: 16 << 30,
                ram_free: 16 << 30,
                measured,
                ..Hardware::default()
            };
            pane_harness(snap, vec![])
        };
        assert!(has_label(&card(true), &tr("local.fit.slow")));
        assert!(has_label(&card(false), &tr("local.fit.unknown")));
    }

    #[test]
    fn removing_the_engine_confirms_first() {
        let mut h = pane_harness(ready_engine(), vec![]);
        click(&mut h, Role::Button, &tr("local.engine.remove"));
        let removed = |h: &Harness<State>| {
            h.state()
                .env
                .commands
                .borrow()
                .iter()
                .any(|c| matches!(c, Command::RemoveEngine))
        };
        assert!(!removed(&h));
        click_in_dialog(&mut h, &tr("dialog.confirm_title"), &tr("button.delete"));
        assert!(removed(&h));
    }

    #[test]
    fn redetect_refreshes_the_hardware() {
        let mut h = pane_harness(Snapshot::default(), vec![]);
        click(&mut h, Role::Link, &tr("local.hw.redetect"));
        assert!(matches!(last(&h), Some(Command::RefreshHardware)));
    }

    #[test]
    fn failed_rows_show_the_original_error_and_can_continue() {
        let mut snap = snapshot(vec![model(MODEL, false, false)]);
        snap.downloads
            .insert(MODEL.into(), DownloadState::Failed("HTTP 503".into()));
        let mut h = pane_harness(snap, vec![]);
        assert!(has_label(&h, "HTTP 503"));
        click(&mut h, Role::Button, &tr("local.continue_download"));
        assert!(matches!(last(&h), Some(Command::Resume(id)) if id == MODEL));
    }

    #[test]
    fn running_rows_show_speed_and_eta_from_successive_snapshots() {
        const MIB: u64 = 1 << 20;
        let running = |done: u64| {
            let mut snap = snapshot(vec![model(MODEL, false, false)]);
            let total = 3072 * MIB;
            snap.downloads
                .insert(MODEL.into(), DownloadState::Running { done, total });
            snap
        };
        let line = t(
            "local.models.downloading",
            &[("done", "1.0"), ("total", "3.0")],
        );
        let mut h = pane_harness(running(1024 * MIB), vec![]);
        assert!(has_label(&h, &line));
        *h.state().env.snapshot.borrow_mut() = running(1042 * MIB);
        h.state_mut().now += Duration::from_secs(1);
        run(&mut h);
        let eta = t("local.models.eta_minutes", &[("n", "2")]);
        let tail = t(
            "local.models.speed_eta",
            &[("speed", "18.0 MB/s"), ("eta", &eta)],
        );
        assert!(has_label(&h, &format!("{line} · {tail}")));
        click(&mut h, Role::Button, &tr("button.cancel"));
        assert!(matches!(last(&h), Some(Command::CancelDownload(id)) if id == MODEL));
    }
}
