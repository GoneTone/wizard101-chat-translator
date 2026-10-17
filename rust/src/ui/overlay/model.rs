//! overlay 的訊息資料：每個（客戶端, 分頁）一份清單，決定顯示哪個客戶端，並維護上限、淡出與未讀。

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use eframe::egui::Color32;

use crate::logs::log;
use crate::reader::chat_tabs::TabState;
use crate::ui::palette::{DIM_FACTOR, FG_ERROR, FG_ORIGINAL, FG_PENDING, FG_TRANSLATED};

/// （客戶端編號, 遊戲分頁編號）。
pub type ViewKey = (u32, i32);

/// 把顏色各通道乘上係數調暗；原文用遊戲色的暗版，維持原文暗、譯文亮的層次。
pub fn dimmed(color: Color32, factor: f64) -> Color32 {
    let scale = |c: u8| (f64::from(c) * factor).round_ties_even() as u8;
    Color32::from_rgb(scale(color.r()), scale(color.g()), scale(color.b()))
}

/// 一段訊息的內容雜湊：訊息增刪或譯文回填都會讓它改變。
pub fn fingerprint(messages: &[Message]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for m in messages {
        (m.seq, &m.translated).hash(&mut hasher);
    }
    hasher.finish()
}

/// 列表中的一則訊息；`msg_id` 為 None 代表不需要就地回填。
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub original: String,
    pub translated: String,
    pub msg_id: Option<u64>,
    pub pending: bool,
    pub failed: bool,
    /// 該則在遊戲內的顯示色；None 退回預設配色。
    pub color: Option<Color32>,
    pub added_at: Instant,
    /// 加入順序的流水號（廣播進多個分頁的各份相同）；繪製端以它認出同一則，不受前方訊息被移除影響。
    pub seq: u64,
}

impl Message {
    /// 原文行的顏色：遊戲色的暗版，沒有遊戲色用預設原文色。
    pub fn original_color(&self) -> Color32 {
        self.color.map_or(FG_ORIGINAL, |c| dimmed(c, DIM_FACTOR))
    }

    /// 譯文行的顏色：佔位中最暗、失敗用錯誤色，其餘用遊戲色。
    pub fn translated_color(&self) -> Color32 {
        if self.pending {
            FG_PENDING
        } else if self.failed {
            FG_ERROR
        } else {
            self.color.unwrap_or(FG_TRANSLATED)
        }
    }
}

/// 一個（客戶端, 分頁）的訊息，舊到新。
#[derive(Debug, Default)]
pub struct View {
    pub messages: Vec<Message>,
}

impl View {
    fn trim(&mut self, max: usize) {
        let excess = self.messages.len().saturating_sub(max);
        self.messages.drain(..excess);
    }
}

/// overlay 的純資料模型（不依賴 egui 繪製）。
pub struct OverlayModel {
    views: HashMap<ViewKey, View>,
    tab_states: HashMap<u32, TabState>,
    client: Option<u32>,
    client_decided: bool,
    max_messages: usize,
    fade: Duration,
    minimized: bool,
    unread: u32,
    next_seq: u64,
}

impl OverlayModel {
    /// `fade_seconds` 為 0 代表不依時間清除。
    pub fn new(max_messages: usize, fade_seconds: u64) -> Self {
        Self {
            views: HashMap::new(),
            tab_states: HashMap::new(),
            client: None,
            client_decided: false,
            max_messages,
            fade: Duration::from_secs(fade_seconds),
            minimized: false,
            unread: 0,
            next_seq: 0,
        }
    }

    /// 顯示中的客戶端。
    pub fn client(&self) -> Option<u32> {
        self.client
    }

    /// 是否處於縮小（泡泡）狀態。
    pub fn minimized(&self) -> bool {
        self.minimized
    }

    /// 縮小期間累積的未讀訊息數。
    pub fn unread(&self) -> u32 {
        self.unread
    }

    /// 進出縮小狀態；真的切換時才把未讀歸零，重複呼叫不動。
    pub fn set_minimized(&mut self, minimized: bool) {
        if self.minimized == minimized {
            return;
        }
        self.minimized = minimized;
        self.unread = 0;
    }

    /// 把訊息加到該客戶端每個 `tabs` 分頁；第一則訊息決定要顯示哪個客戶端。
    ///
    /// 縮小期間只有顯示中客戶端的訊息計入未讀，廣播進多個分頁也只算一則。
    #[allow(clippy::too_many_arguments)]
    pub fn add_message(
        &mut self,
        slot: u32,
        tabs: &[i32],
        original: &str,
        translated: &str,
        msg_id: Option<u64>,
        pending: bool,
        color: Option<&str>,
        now: Instant,
    ) {
        if !self.client_decided {
            self.set_foreground_slot(Some(slot));
        }
        let color = color.and_then(|c| Color32::from_hex(c).ok());
        let seq = self.next_seq;
        self.next_seq += 1;
        for &tab in tabs {
            let view = self.views.entry((slot, tab)).or_default();
            view.messages.push(Message {
                original: original.to_owned(),
                translated: translated.to_owned(),
                msg_id,
                pending,
                failed: false,
                color,
                added_at: now,
                seq,
            });
            view.trim(self.max_messages);
        }
        if self.minimized && Some(slot) == self.client {
            self.unread += 1;
        }
    }

    /// 回填佔位訊息的譯文（廣播會落在多個分頁）；找不到 `msg_id`（已被擠掉）就忽略。
    pub fn update_message(&mut self, msg_id: u64, translated: &str, failed: bool) {
        for message in self
            .views
            .values_mut()
            .flat_map(|v| v.messages.iter_mut())
            .filter(|m| m.msg_id == Some(msg_id))
        {
            message.translated = translated.to_owned();
            message.pending = false;
            message.failed = failed;
        }
    }

    /// 記下某客戶端的分頁狀態（None＝讀不到）；還沒決定客戶端或就是顯示中的客戶端時一併換顯示。
    pub fn set_tabs(&mut self, slot: u32, state: Option<TabState>) {
        match state {
            Some(state) => self.tab_states.insert(slot, state),
            None => self.tab_states.remove(&slot),
        };
        if !self.client_decided || Some(slot) == self.client {
            self.set_foreground_slot(Some(slot));
        }
    }

    /// 改顯示該客戶端。
    pub fn set_foreground_slot(&mut self, slot: Option<u32>) {
        if !self.client_decided || slot != self.client {
            log(format!("[ui] overlay client switched: slot={slot:?}"));
        }
        self.client_decided = true;
        self.client = slot;
    }

    /// 套用新的訊息上限與淡出秒數；超出上限的最舊訊息立即移除。
    pub fn set_limits(&mut self, max_messages: usize, fade_seconds: u64) {
        self.max_messages = max_messages;
        self.fade = Duration::from_secs(fade_seconds);
        for view in self.views.values_mut() {
            view.trim(max_messages);
        }
    }

    /// 移除每份清單中已超過淡出秒數的訊息（淡出為 0 時不做）。
    pub fn prune(&mut self, now: Instant) {
        if self.fade.is_zero() {
            return;
        }
        for view in self.views.values_mut() {
            view.messages
                .retain(|m| now.saturating_duration_since(m.added_at) < self.fade);
        }
    }

    /// 顯示中那份清單的下一則訊息還要多久淡出；不淡出或清單是空的時回 `None`（不必排程重繪）。
    pub fn next_fade(&self, now: Instant) -> Option<Duration> {
        if self.fade.is_zero() {
            return None;
        }
        self.current_view()?
            .messages
            .iter()
            .map(|m| (m.added_at + self.fade).saturating_duration_since(now))
            .min()
    }

    /// 顯示中的（客戶端, 分頁）；還沒決定客戶端時為 None。
    pub fn current_key(&self) -> Option<ViewKey> {
        let slot = self.client?;
        Some((slot, self.tab_states.get(&slot).map_or(0, |s| s.current)))
    }

    /// 顯示中的（客戶端, 分頁）的訊息；還沒有任何訊息進過那份清單時為 None。
    pub fn current_view(&self) -> Option<&View> {
        self.views.get(&self.current_key()?)
    }

    /// 沒有訊息可畫時，由呼叫端改畫狀態佔位字。
    pub fn is_empty(&self) -> bool {
        self.current_view().is_none_or(|v| v.messages.is_empty())
    }

    /// 顯示中客戶端的分頁列內容（編號與按鈕文字）與目前分頁；沒有狀態時為空。
    pub fn current_tabs(&self) -> (&[(i32, String)], Option<i32>) {
        match self.client.and_then(|s| self.tab_states.get(&s)) {
            Some(state) => (&state.tabs, Some(state.current)),
            None => (&[], None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(current: i32) -> TabState {
        TabState {
            tabs: vec![(0, "Main".into()), (1, "Group".into()), (2, "Room".into())],
            current,
        }
    }

    fn texts(model: &OverlayModel) -> Vec<(String, String)> {
        model.current_view().map_or_else(Vec::new, |v| {
            v.messages
                .iter()
                .map(|m| (m.original.clone(), m.translated.clone()))
                .collect()
        })
    }

    #[test]
    fn next_fade_waits_for_the_oldest_shown_message() {
        let t0 = Instant::now();
        let mut model = OverlayModel::new(200, 30);
        assert_eq!(
            model.next_fade(t0),
            None,
            "nothing shown, nothing to schedule"
        );
        for (text, at) in [("a", t0), ("b", t0 + Duration::from_secs(5))] {
            model.add_message(1, &[0], text, text, None, false, None, at);
        }
        assert_eq!(
            model.next_fade(t0 + Duration::from_secs(10)),
            Some(Duration::from_secs(20))
        );
        assert_eq!(
            model.next_fade(t0 + Duration::from_secs(40)),
            Some(Duration::ZERO)
        );
        model.set_limits(200, 0);
        assert_eq!(model.next_fade(t0), None, "fading off");
    }

    fn add(model: &mut OverlayModel, slot: u32, tabs: &[i32], original: &str, translated: &str) {
        model.add_message(
            slot,
            tabs,
            original,
            translated,
            None,
            false,
            None,
            Instant::now(),
        );
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.into(), b.into())
    }

    #[test]
    fn messages_go_to_each_tab_view() {
        let mut model = OverlayModel::new(10, 0);
        model.set_tabs(1, Some(state(0)));
        add(&mut model, 1, &[0], "main", "主");
        add(&mut model, 1, &[1], "group", "隊");
        add(&mut model, 1, &[3], "friend", "友");
        assert_eq!(texts(&model), [pair("main", "主")]);
        model.set_tabs(1, Some(state(1)));
        assert_eq!(texts(&model), [pair("group", "隊")]);
        // 不在分頁列上的分頁也照樣保存
        model.set_tabs(1, Some(state(3)));
        assert_eq!(texts(&model), [pair("friend", "友")]);
    }

    #[test]
    fn broadcast_lands_in_every_listed_tab() {
        let mut model = OverlayModel::new(10, 0);
        add(&mut model, 1, &[0, 1, 2, 3], "news", "公告");
        for tab in 0..4 {
            model.set_tabs(
                1,
                Some(TabState {
                    tabs: vec![(tab, "x".into())],
                    current: tab,
                }),
            );
            assert_eq!(texts(&model), [pair("news", "公告")]);
        }
    }

    #[test]
    fn update_fills_in_place_keeps_order() {
        let mut model = OverlayModel::new(10, 0);
        let now = Instant::now();
        for (i, line) in ["[A] one", "[B] two", "[C] three"].iter().enumerate() {
            model.add_message(1, &[0], line, "...", Some(i as u64), true, None, now);
        }
        // 完成順序與讀取順序相反
        model.update_message(2, "丙", false);
        model.update_message(1, "乙", false);
        model.update_message(0, "甲", false);
        assert_eq!(
            texts(&model),
            [
                pair("[A] one", "甲"),
                pair("[B] two", "乙"),
                pair("[C] three", "丙")
            ]
        );
        assert!(
            model
                .current_view()
                .unwrap()
                .messages
                .iter()
                .all(|m| !m.pending)
        );
    }

    #[test]
    fn update_fills_hidden_view_and_ignores_unknown_id() {
        let mut model = OverlayModel::new(1, 0);
        model.set_tabs(1, Some(state(0)));
        let now = Instant::now();
        model.add_message(1, &[1], "group", "...", Some(5), true, None, now);
        model.update_message(5, "隊", false);
        model.add_message(1, &[0], "a", "...", Some(1), true, None, now);
        model.add_message(1, &[0], "b", "...", Some(2), true, None, now);
        model.update_message(1, "甲", false); // 已被擠掉
        assert_eq!(texts(&model), [pair("b", "...")]);
        model.set_tabs(1, Some(state(1)));
        assert_eq!(texts(&model), [pair("group", "隊")]);
    }

    #[test]
    fn colors_follow_pending_failed_and_game_color() {
        let mut model = OverlayModel::new(10, 0);
        let now = Instant::now();
        model.add_message(1, &[0], "o", "...", Some(1), true, Some("#80ff00"), now);
        model.add_message(1, &[0], "o", "t", None, false, None, now);
        let message = |m: &OverlayModel, i: usize| m.current_view().unwrap().messages[i].clone();
        let green = Color32::from_rgb(0x80, 0xff, 0);
        assert_eq!(message(&model, 0).translated_color(), FG_PENDING);
        assert_eq!(
            message(&model, 0).original_color(),
            dimmed(green, DIM_FACTOR)
        );
        model.update_message(1, "甲", false);
        assert_eq!(message(&model, 0).translated_color(), green);
        model.update_message(1, "⚠", true);
        assert_eq!(message(&model, 0).translated_color(), FG_ERROR);
        assert_eq!(message(&model, 1).translated_color(), FG_TRANSLATED);
        assert_eq!(message(&model, 1).original_color(), FG_ORIGINAL);
    }

    #[test]
    fn max_messages_drops_oldest() {
        let mut model = OverlayModel::new(3, 0);
        for i in 0..5 {
            add(
                &mut model,
                1,
                &[0],
                &format!("msg {i}"),
                &format!("訊息 {i}"),
            );
        }
        assert_eq!(
            texts(&model),
            [
                pair("msg 2", "訊息 2"),
                pair("msg 3", "訊息 3"),
                pair("msg 4", "訊息 4")
            ]
        );
        model.set_limits(1, 0);
        assert_eq!(texts(&model), [pair("msg 4", "訊息 4")]);
    }

    #[test]
    fn max_messages_applies_per_tab() {
        let mut model = OverlayModel::new(2, 0);
        model.set_tabs(1, Some(state(0)));
        add(&mut model, 1, &[1], "g", "隊");
        for i in 0..3 {
            add(&mut model, 1, &[0], &format!("m{i}"), &format!("主{i}"));
        }
        assert_eq!(texts(&model), [pair("m1", "主1"), pair("m2", "主2")]);
        model.set_tabs(1, Some(state(1)));
        assert_eq!(texts(&model), [pair("g", "隊")]);
    }

    #[test]
    fn fade_prunes_after_seconds_and_zero_disables() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut model = OverlayModel::new(10, 10);
        model.set_tabs(1, Some(state(0)));
        model.add_message(1, &[0, 1], "old", "舊", None, false, None, at(0));
        model.add_message(1, &[0, 1], "new", "新", None, false, None, at(5));
        model.prune(at(10)); // 剛好滿 10 秒就過期，與 Python 的 ts <= now - fade 一致
        assert_eq!(texts(&model), [pair("new", "新")]);
        model.set_tabs(1, Some(state(1)));
        assert_eq!(texts(&model), [pair("new", "新")]); // 隱藏分頁也清
        model.prune(at(100));
        assert!(model.is_empty());

        let mut forever = OverlayModel::new(10, 0);
        forever.add_message(1, &[0], "a", "甲", None, false, None, at(0));
        forever.prune(at(1_000_000));
        assert_eq!(texts(&forever), [pair("a", "甲")]);
    }

    #[test]
    fn unread_counts_once_per_broadcast_only_for_shown_client() {
        let mut model = OverlayModel::new(10, 0);
        model.set_tabs(1, Some(state(0)));
        add(&mut model, 1, &[0], "before", "x");
        model.set_minimized(true);
        add(&mut model, 2, &[0], "x", "x");
        add(&mut model, 2, &[0], "y", "y");
        add(&mut model, 1, &[0, 1, 2, 3], "news", "公告");
        assert_eq!(model.unread(), 1);
        model.set_minimized(false);
        assert_eq!(model.unread(), 0);
        assert_eq!(model.current_view().unwrap().messages.len(), 2); // 縮小期間的訊息沒遺失
        add(&mut model, 1, &[0], "z", "z");
        assert_eq!(model.unread(), 0); // 沒縮小就不累積
    }

    #[test]
    fn repeated_minimize_or_expand_keeps_unread() {
        let mut model = OverlayModel::new(10, 0);
        model.set_tabs(1, Some(state(0)));
        model.set_minimized(true);
        add(&mut model, 1, &[0], "a", "甲");
        model.set_minimized(true);
        assert_eq!(model.unread(), 1);
        model.set_minimized(false);
        model.set_minimized(false);
        assert_eq!(model.unread(), 0);
    }

    #[test]
    fn first_message_decides_client() {
        let mut model = OverlayModel::new(10, 0);
        add(&mut model, 2, &[0], "two", "二");
        assert_eq!(model.client(), Some(2));
        add(&mut model, 1, &[0], "one", "一");
        assert_eq!(model.client(), Some(2));
        assert_eq!(texts(&model), [pair("two", "二")]);
    }

    #[test]
    fn first_set_tabs_decides_client_and_other_clients_do_not_switch() {
        let mut model = OverlayModel::new(10, 0);
        model.set_tabs(1, Some(state(0)));
        assert_eq!(model.client(), Some(1));
        model.set_tabs(2, Some(state(1)));
        assert_eq!(model.client(), Some(1));
        assert_eq!(model.current_tabs().1, Some(0));
    }

    #[test]
    fn set_tabs_for_shown_client_switches_view() {
        let mut model = OverlayModel::new(10, 0);
        model.set_tabs(1, Some(state(0)));
        add(&mut model, 1, &[0], "main", "主");
        add(&mut model, 1, &[2], "room", "房");
        let (tabs, current) = model.current_tabs();
        assert_eq!((tabs.len(), current), (3, Some(0)));
        model.set_tabs(1, Some(state(2)));
        assert_eq!(texts(&model), [pair("room", "房")]);
        assert_eq!(model.current_tabs().1, Some(2));
        // 分頁不明：回到預設的 0 號分頁，分頁列隱藏
        model.set_tabs(1, None);
        assert_eq!(texts(&model), [pair("main", "主")]);
        assert_eq!(model.current_tabs(), (&[][..], None));
    }

    #[test]
    fn foreground_slot_switches_the_whole_view() {
        let mut model = OverlayModel::new(10, 0);
        model.set_tabs(1, Some(state(0)));
        model.set_tabs(
            2,
            Some(TabState {
                tabs: vec![(0, "Main".into()), (1, "Group".into())],
                current: 1,
            }),
        );
        add(&mut model, 1, &[0], "one", "一");
        add(&mut model, 2, &[1], "two", "二");
        assert_eq!(texts(&model), [pair("one", "一")]);
        model.set_foreground_slot(Some(2));
        assert_eq!(model.current_tabs().0.len(), 2);
        assert_eq!(texts(&model), [pair("two", "二")]);
        model.set_foreground_slot(None);
        assert!(model.current_view().is_none());
    }

    #[test]
    fn dimmed_matches_python() {
        // 對照 Python message_list.dimmed 的輸出
        let dim = |r, g, b| dimmed(Color32::from_rgb(r, g, b), DIM_FACTOR);
        assert_eq!(dim(0xff, 0xff, 0xff), Color32::from_rgb(0xb5, 0xb5, 0xb5));
        assert_eq!(dim(0x80, 0xff, 0x00), Color32::from_rgb(0x5b, 0xb5, 0x00));
        assert_eq!(dim(0, 0, 0), Color32::BLACK);
        // 落在進位邊界的值：Python 的 round 是銀行家進位（150 → 106）
        assert_eq!(dim(50, 150, 250), Color32::from_rgb(36, 106, 178));
    }
}
