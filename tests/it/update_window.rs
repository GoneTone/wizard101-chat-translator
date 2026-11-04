//! 更新內容視窗：卡片清單、各下載階段的底部列與按鈕、Markdown 內文。

use crate::common::kittest::{click, run, themed_harness, tr};
use eframe::egui::accesskit::Role;
use eframe::egui::{Color32, FontFamily, Shape, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::runtime::update_job::Phase;
use wizard101_chat_translator::runtime::updater::Release;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::update_window::{UpdateAction, UpdateView, can_close};

const MIB: u64 = 1 << 20;

struct Window {
    view: UpdateView,
    releases: Vec<Release>,
    phase: Phase,
    progress: (u64, u64),
    unsaved: bool,
    actions: Vec<UpdateAction>,
}

fn release(version: &str, body: &str) -> Release {
    Release {
        version: version.into(),
        url: format!("https://example.com/v{version}"),
        published_at: "2026-11-03T08:00:00Z".into(),
        body: body.into(),
        asset: None,
    }
}

fn window_with(
    releases: Vec<Release>,
    phase: Phase,
    progress: (u64, u64),
    unsaved: bool,
) -> Harness<'static, Window> {
    let state = Window {
        view: UpdateView::new(),
        releases,
        phase,
        progress,
        unsaved,
        actions: Vec::new(),
    };
    themed_harness(
        vec2(620.0, 600.0),
        "en-US",
        |ui, w: &mut Window| {
            let actions = w.view.ui(ui, &w.releases, &w.phase, w.progress, w.unsaved);
            w.actions.extend(actions);
        },
        state,
    )
}

fn window(phase: Phase, progress: (u64, u64), unsaved: bool) -> Harness<'static, Window> {
    let releases = vec![
        release("1.2.0", "Newest notes"),
        release("1.1.0", "Older notes"),
    ];
    window_with(releases, phase, progress, unsaved)
}

/// 點名稱是 `key` 譯文的按鈕，回傳這次點擊產生的動作。
fn press(h: &mut Harness<'static, Window>, key: &str) -> Vec<UpdateAction> {
    h.state_mut().actions.clear();
    click(h, Role::Button, &tr(key));
    std::mem::take(&mut h.state_mut().actions)
}

fn disabled(h: &Harness<'static, Window>, key: &str) -> bool {
    h.get_by_role_and_label(Role::Button, &tr(key))
        .accesskit_node()
        .is_disabled()
}

/// 卡片由新到舊，最新那張有「最新」標籤；〔立即更新〕開始下載、〔稍後〕關閉。
#[test]
fn idle_lists_every_release_newest_first() {
    let mut h = window(Phase::Idle, (0, 0), false);
    let newest = h.get_by_label("v1.2.0").rect().top();
    let older = h.get_by_label("v1.1.0").rect().top();
    assert!(newest < older);
    h.get_by_label(&tr("update.latest_tag"));
    assert_eq!(press(&mut h, "update.now"), [UpdateAction::Start]);
    assert_eq!(press(&mut h, "update.later"), [UpdateAction::Close]);
}

/// 捲到最後一張卡片時，標題與副標固定在原位，只有卡片跟著捲動。
#[test]
fn header_stays_put_while_the_cards_scroll() {
    let long = "A long paragraph of release notes.\n\n".repeat(80);
    let releases = vec![release("1.2.0", &long), release("1.1.0", "Older notes")];
    let mut h = window_with(releases, Phase::Idle, (0, 0), false);
    let heading = t("update.heading", &[("version", "1.2.0")]);
    let heading_top = h.get_by_label(&heading).rect().top();
    let newest_top = h.get_by_label("v1.2.0").rect().top();

    h.get_by_label("v1.1.0").scroll_to_me();
    run(&mut h);

    assert_eq!(h.get_by_label(&heading).rect().top(), heading_top);
    assert!(h.get_by_label("v1.2.0").rect().top() < newest_top - 100.0);
    assert!(h.get_by_label("v1.1.0").rect().top() > heading_top);
}

/// 下載中有進度條與 MB 進度文字，只剩〔取消下載〕可按。
#[test]
fn downloading_shows_progress_and_only_cancel() {
    let mut h = window(Phase::Downloading, (25 * MIB, 61 * MIB), false);
    h.get_by_role(Role::ProgressIndicator);
    let text = t("update.downloading", &[("done", "25.0"), ("total", "61.0")]);
    h.get_by_label(&text);
    assert!(h.query_by_label(&tr("update.now")).is_none());
    assert!(!disabled(&h, "update.cancel"));
    assert_eq!(press(&mut h, "update.cancel"), [UpdateAction::Cancel]);
}

/// 下載量到總量時是驗證中，不能取消。
#[test]
fn verifying_disables_cancel() {
    let h = window(Phase::Downloading, (61 * MIB, 61 * MIB), false);
    h.get_by_label(&tr("update.verifying"));
    assert!(disabled(&h, "update.cancel"));
}

/// 還沒拿到總量（0／0）時仍是下載中，可以取消。
#[test]
fn downloading_at_zero_bytes_is_not_verifying() {
    let h = window(Phase::Downloading, (0, 0), false);
    assert!(h.query_by_label(&tr("update.verifying")).is_none());
    assert!(!disabled(&h, "update.cancel"));
}

/// 重新啟動中不能取消。
#[test]
fn restarting_disables_cancel() {
    let h = window(Phase::Restarting, (61 * MIB, 61 * MIB), false);
    h.get_by_label(&tr("update.restarting"));
    assert!(disabled(&h, "update.cancel"));
}

/// 失敗時顯示原始錯誤，可重試或前往 GitHub。
#[test]
fn failed_offers_github_and_retry() {
    let phase = Phase::Failed {
        error: "boom".into(),
        replace: false,
    };
    let mut h = window(phase, (0, 0), false);
    h.get_by_label_contains("boom");
    assert_eq!(press(&mut h, "update.retry"), [UpdateAction::Retry]);
    assert_eq!(
        press(&mut h, "update.open_github"),
        [UpdateAction::OpenReleasePage]
    );
}

/// 取消後可關閉或重新下載。
#[test]
fn cancelled_offers_close_and_redownload() {
    let mut h = window(Phase::Cancelled, (0, 0), false);
    h.get_by_label(&tr("update.cancelled"));
    assert_eq!(press(&mut h, "update.close"), [UpdateAction::Close]);
    assert_eq!(press(&mut h, "update.redownload"), [UpdateAction::Start]);
}

/// 下載完成但設定有沒儲存的變更：警告並提供放棄變更重新啟動。
#[test]
fn ready_with_unsaved_settings_asks_to_discard() {
    let mut h = window(Phase::Ready, (61 * MIB, 61 * MIB), true);
    h.get_by_label(&tr("update.unsaved"));
    assert!(h.query_by_label(&tr("update.restart")).is_none());
    assert_eq!(
        press(&mut h, "update.discard_restart"),
        [UpdateAction::Restart]
    );
    assert_eq!(press(&mut h, "update.later"), [UpdateAction::Close]);
}

/// 下載完成且設定都存好了：直接提供重新啟動。
#[test]
fn ready_without_unsaved_settings_offers_restart() {
    let mut h = window(Phase::Ready, (61 * MIB, 61 * MIB), false);
    h.get_by_label(&tr("update.ready"));
    assert!(h.query_by_label(&tr("update.discard_restart")).is_none());
    assert_eq!(press(&mut h, "update.restart"), [UpdateAction::Restart]);
}

/// 內文以 Markdown 渲染，不露出原始標記。
#[test]
fn markdown_body_is_rendered() {
    let h = window_with(
        vec![release("1.2.0", "## What's New\n\n- item")],
        Phase::Idle,
        (0, 0),
        false,
    );
    h.get_by_label("What's New");
    h.get_by_label("item");
    assert!(h.query_by_label_contains("##").is_none());
}

/// 畫面上文字 `text` 的字族。
fn font_family(h: &Harness<'static, Window>, text: &str) -> FontFamily {
    h.output()
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.job.sections[0].format.font_id.family.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} not painted"))
}

/// 各級標題都用粗體；程式碼區塊裡的 `##` 不當標題。
#[test]
fn markdown_headings_are_bold() {
    let body = "## What's New

### Fixes

plain

```
## not a heading
```";
    let h = window_with(vec![release("1.2.0", body)], Phase::Idle, (0, 0), false);
    let bold = theme::bold(1.0).family;
    assert_eq!(font_family(&h, "What's New"), bold);
    assert_eq!(font_family(&h, "Fixes"), bold);
    assert_ne!(font_family(&h, "plain"), bold);
    let code = h.output().shapes.iter().any(
        |c| matches!(&c.shape, Shape::Text(t) if t.galley.text().contains("## not a heading")),
    );
    assert!(code);
}

/// 畫面上第一個含 `needle` 的文字：依各列首字的字型量出的字列上緣 y 與字高。
fn text_rows(h: &Harness<'static, Window>, needle: &str) -> Vec<(f32, f32)> {
    h.output()
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text().contains(needle) => Some(
                t.galley
                    .rows
                    .iter()
                    .filter_map(|r| {
                        let g = r.row.glyphs.first()?;
                        Some((t.pos.y + r.pos.y + g.pos.y - g.font_ascent, g.font_height))
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{needle:?} not painted"))
}

/// 清單項目裡的連結與前後文字同一條基線；項目符號落在第一行中央；項目內換行行距同段落。
#[test]
fn list_items_line_up_with_their_text() {
    let long = "word ".repeat(40);
    let body = format!(
        "* New Crowdin updates by [@GoneTone](https://github.com/GoneTone) in [#72](https://x.com/72)\n\
         * listed {long}\n\nplain {long}"
    );
    let h = window_with(vec![release("1.2.0", &body)], Phase::Idle, (0, 0), false);

    let text = text_rows(&h, "New Crowdin")[0];
    for link in ["@GoneTone", "#72"] {
        let row = text_rows(&h, link)[0];
        assert!(
            (row.0 - text.0).abs() <= 1.0 && (row.1 - text.1).abs() <= 1.0,
            "{link} is not on the item's first row: {row:?} vs {text:?}"
        );
    }

    let middle = text.0 + text.1 / 2.0;
    let bullet = h
        .output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Circle(c) if (text.0..=text.0 + text.1).contains(&c.center.y) => {
                Some(c.center.y)
            }
            _ => None,
        })
        .min_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()))
        .expect("bullet painted on the item's first row");
    assert!((bullet - middle).abs() <= 1.0, "{bullet} vs {middle}");

    let gap = |rows: &[(f32, f32)]| rows[1].0 - rows[0].0;
    let listed = text_rows(&h, "listed");
    let plain = text_rows(&h, "plain");
    assert!(
        (gap(&listed) - gap(&plain)).abs() <= 1.0,
        "{listed:?} vs {plain:?}"
    );
}

/// 發布日期換成本機時區的日期；解析不了就顯示原字串。
#[test]
fn published_date_is_local_or_raw() {
    let at = "2026-11-03T23:30:00+08:00";
    let mut odd = release("1.1.0", "");
    odd.published_at = "not-a-date".into();
    let mut newest = release("1.2.0", "");
    newest.published_at = at.into();
    let h = window_with(vec![newest, odd], Phase::Idle, (0, 0), false);
    let local = chrono::DateTime::parse_from_rfc3339(at)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d")
        .to_string();
    h.get_by_label(&t("update.published", &[("date", &local)]));
    h.get_by_label(&t("update.published", &[("date", "not-a-date")]));
}

/// 量到速度後，進度文字接上速度。
#[test]
fn downloading_appends_the_speed() {
    let mut h = window(Phase::Downloading, (10 * MIB, 100 * MIB), false);
    std::thread::sleep(std::time::Duration::from_millis(120));
    h.state_mut().progress = (20 * MIB, 100 * MIB);
    h.run_steps(2);
    let prefix = t(
        "update.downloading",
        &[("done", "20.0"), ("total", "100.0")],
    ) + " · ";
    let label = h
        .get_by_label_contains(&prefix)
        .accesskit_node()
        .value()
        .unwrap()
        .to_string();
    assert!(label.ends_with(" MB/s"), "{label}");
}

/// 只有下載中（含驗證）與重新啟動中不能關視窗。
#[test]
fn can_close_only_outside_download_and_restart() {
    assert!(!can_close(&Phase::Downloading));
    assert!(!can_close(&Phase::Restarting));
    for phase in [
        Phase::Idle,
        Phase::Ready,
        Phase::Cancelled,
        Phase::Failed {
            error: String::new(),
            replace: true,
        },
    ] {
        assert!(can_close(&phase), "{phase:?}");
    }
}

mod links {
    use super::*;
    use wizard101_chat_translator::ui::settings::release_links::linkify_github;
    use wizard101_chat_translator::ui::text_select::opened_urls;

    const PR: &str = "https://github.com/GoneTone/wizard101-chat-translator/pull/69";

    /// 本專案的 PR 網址改成 `#N` 連結。
    #[test]
    fn project_pr_url_becomes_numbered_link() {
        let line = format!("* fix(settings): require a service name by @GoneTone in {PR}");
        assert_eq!(
            linkify_github(&line),
            format!(
                "* fix(settings): require a service name by [@GoneTone](https://github.com/GoneTone) in [#69]({PR})"
            )
        );
    }

    /// issue、compare、commit 網址的連結文字。
    #[test]
    fn project_issue_compare_and_commit_urls_get_short_labels() {
        let repo = "https://github.com/GoneTone/wizard101-chat-translator";
        let compare = format!("**Full Changelog**: {repo}/compare/v1.1.0...v1.2.0");
        assert_eq!(
            linkify_github(&compare),
            format!("**Full Changelog**: [v1.1.0...v1.2.0]({repo}/compare/v1.1.0...v1.2.0)")
        );
        assert_eq!(
            linkify_github(&format!("see {repo}/issues/7.")),
            format!("see [#7]({repo}/issues/7).")
        );
        let sha = "4128e3f0fd086b9aaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(
            linkify_github(&format!("{repo}/commit/{sha}")),
            format!("[4128e3f]({repo}/commit/{sha})")
        );
    }

    /// 機器人帳號連到 `apps/`。
    #[test]
    fn bot_mention_links_to_apps() {
        assert_eq!(
            linkify_github("* chore(deps): bump x by @dependabot[bot] in"),
            "* chore(deps): bump x by [@dependabot[bot]](https://github.com/apps/dependabot) in"
        );
    }

    /// 已是連結、信箱、程式碼的內容不動。
    #[test]
    fn existing_links_emails_and_code_are_untouched() {
        for same in [
            format!("[#69]({PR})"),
            format!("<{PR}>"),
            "mail a@b.com now".to_owned(),
            "path /x/@user and [@u](https://github.com/u)".to_owned(),
            format!("`{PR} @GoneTone`"),
            format!("```\n{PR} @GoneTone\n```"),
            format!("    {PR}"),
        ] {
            assert_eq!(linkify_github(&same), same);
        }
    }

    /// 其他裸網址變成自動連結，尾端句讀留在外面。
    #[test]
    fn other_bare_urls_become_autolinks_without_trailing_punctuation() {
        assert_eq!(
            linkify_github("see https://example.com/a. And (https://example.com/b), ok。"),
            "see <https://example.com/a>. And (<https://example.com/b>), ok。"
        );
    }

    /// URL 只含 RFC 3986 字元：中文與全形標點、強調記號都不進網址。
    #[test]
    fn url_ends_at_cjk_text_and_emphasis_markers() {
        assert_eq!(
            linkify_github("詳見https://x.com/a，謝謝"),
            "詳見<https://x.com/a>，謝謝"
        );
        assert_eq!(
            linkify_github("**https://x.com/a**"),
            "**<https://x.com/a>**"
        );
        assert_eq!(linkify_github("https://x.com/a。"), "<https://x.com/a>。");
    }

    /// 成對括號留在網址裡，不成對的結尾括號才切掉。
    #[test]
    fn balanced_parentheses_stay_in_the_url() {
        assert_eq!(
            linkify_github("https://en.wikipedia.org/wiki/Foo_(bar)"),
            "<https://en.wikipedia.org/wiki/Foo_(bar)>"
        );
        assert_eq!(
            linkify_github("(see https://x.com/a)"),
            "(see <https://x.com/a>)"
        );
    }

    /// 行內程式碼依反引號數量配對。
    #[test]
    fn inline_code_pairs_backtick_runs() {
        for same in ["``a ` https://x.com/a``", "`a` and ```b https://x.com/a```"] {
            let out = linkify_github(same);
            assert!(!out.contains("<https://x.com/a>"), "{out}");
        }
        assert_eq!(
            linkify_github("`x` https://x.com/a"),
            "`x` <https://x.com/a>"
        );
        assert_eq!(
            linkify_github("`unclosed https://x.com/a"),
            "`unclosed <https://x.com/a>"
        );
    }

    fn body_window(body: &str) -> Harness<'static, Window> {
        window_with(vec![release("1.2.0", body)], Phase::Idle, (0, 0), false)
    }

    /// 點內文裡的 PR 連結：視窗輸出要求開啟完整網址。
    #[test]
    fn clicking_a_pr_link_opens_the_full_url() {
        let mut h = body_window(&format!("* fix thing in {PR}"));
        h.get_by_role_and_label(Role::Link, "#69").click();
        h.step();
        let opened = opened_urls(&h.output().platform_output);
        assert!(opened.contains(&PR.to_owned()), "{opened:?}");
    }

    /// 內文連結用設定視窗的連結色，不是內文白色。
    #[test]
    fn link_text_uses_the_link_color() {
        let h = body_window(&format!("* fix thing in {PR}"));
        let shape = h
            .output()
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == "#69" => Some(t.clone()),
                _ => None,
            })
            .expect("link painted");
        for section in &shape.galley.job.sections {
            let c = section.format.color;
            assert!(c == Color32::PLACEHOLDER || c == theme::LINK, "{c:?}");
        }
        let color = shape.override_text_color.unwrap_or(shape.fallback_color);
        assert_eq!(color, theme::LINK);
    }
}

/// 底部列左側狀態（有進度條時連同進度條）的垂直中心，與按鈕的垂直中心對齊。
#[test]
fn footer_status_is_vertically_centred_on_the_buttons() {
    let long_error = "boom ".repeat(60);
    let failed = |error: &str| Phase::Failed {
        error: error.into(),
        replace: false,
    };
    let cases = [
        (
            Phase::Downloading,
            (25 * MIB, 61 * MIB),
            false,
            "update.downloading",
            true,
            "update.cancel",
        ),
        (
            Phase::Downloading,
            (61 * MIB, 61 * MIB),
            false,
            "update.verifying",
            true,
            "update.cancel",
        ),
        (
            Phase::Restarting,
            (61 * MIB, 61 * MIB),
            false,
            "update.restarting",
            true,
            "update.cancel",
        ),
        (failed("boom"), (0, 0), false, "boom", false, "update.retry"),
        (
            failed(&long_error),
            (0, 0),
            false,
            "boom boom",
            false,
            "update.retry",
        ),
        (
            Phase::Cancelled,
            (0, 0),
            false,
            "update.cancelled",
            false,
            "update.redownload",
        ),
        (
            Phase::Ready,
            (61 * MIB, 61 * MIB),
            true,
            "update.unsaved",
            false,
            "update.discard_restart",
        ),
        (
            Phase::Ready,
            (61 * MIB, 61 * MIB),
            false,
            "update.ready",
            false,
            "update.restart",
        ),
    ];
    for (phase, progress, unsaved, status, bar, button) in cases {
        let label = match status {
            "update.downloading" => t(status, &[("done", "25.0"), ("total", "61.0")]),
            key if key.starts_with("update.") => tr(key),
            raw => raw.to_owned(),
        };
        let h = window(phase, progress, unsaved);
        let text = h.get_by_label_contains(&label).rect();
        let (top, bottom) = if bar {
            let b = h.get_by_role(Role::ProgressIndicator).rect();
            (b.top().min(text.top()), b.bottom().max(text.bottom()))
        } else {
            (text.top(), text.bottom())
        };
        let target = h.get_by_role_and_label(Role::Button, &tr(button)).rect();
        let (status_mid, button_mid) = ((top + bottom) / 2.0, target.center().y);
        assert!(
            (status_mid - button_mid).abs() <= 1.0,
            "{status}: status centre {status_mid} vs button centre {button_mid}"
        );
    }
}
