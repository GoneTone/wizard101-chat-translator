//! 畫 immediate viewport 前先確定它的原生視窗已建好：還沒有視窗的先登記一幀，下一幀才畫。
//!
//! 根視窗隱藏（縮成泡泡）時 eframe 從沒有事件迴圈的路徑重繪，immediate viewport 在那裡建不了視窗、egui 直接 panic；
//! 登記過的 viewport 由 eframe 在下一幀開頭建好視窗，這條路徑也一樣。

use eframe::egui::{self, Id, Ui, ViewportBuilder, ViewportClass, ViewportId};

use crate::logs::log;

/// 畫出 immediate viewport 並回傳 `content` 的結果；視窗還沒建好時這一幀只登記（不畫內容）、要求重繪並回 `None`。
///
/// 同一幀只登記一個：一次只誕生一個視窗，宿主拿 `before`（建立前的執行緒視窗清單）比對才認得出自己的 HWND。
/// 視窗在兩幀之間誕生，所以建好那一幀 `before` 會換成登記那一幀拍的。
pub fn show_immediate<T>(
    ctx: &egui::Context,
    id: ViewportId,
    name: &str,
    builder: ViewportBuilder,
    before: &mut Vec<isize>,
    content: impl FnMut(&mut Ui, ViewportClass) -> T,
) -> Option<T> {
    let stash = Id::unique(("viewport windows before", id));
    let (registered, has_window) = ctx.input(|i| {
        let info = i.raw.viewports.get(&id);
        (
            info.is_some(),
            // eframe 只在建好視窗時填這一欄
            info.is_some_and(|v| v.native_pixels_per_point.is_some()),
        )
    });
    if has_window || ctx.embed_viewports() {
        if let Some(earlier) = ctx.data_mut(|d| d.remove_temp::<Vec<isize>>(stash)) {
            *before = earlier;
        }
        return Some(ctx.show_viewport_immediate(id, builder, content));
    }
    let frame = ctx.cumulative_frame_nr();
    let slot = Id::unique("viewport registered this frame");
    let taken = ctx.data(|d| d.get_temp::<(u64, ViewportId)>(slot));
    if taken.is_none_or(|(at, other)| at != frame || other == id) {
        if !registered && taken != Some((frame, id)) {
            log(format!(
                "[ui] {name} window not created yet; registered its viewport for the next frame"
            ));
        }
        ctx.data_mut(|d| d.insert_temp(slot, (frame, id)));
        ctx.show_viewport_deferred(id, builder, |_, _| {});
    }
    ctx.data_mut(|d| d.insert_temp(stash, std::mem::take(before)));
    ctx.request_repaint();
    None
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Context, RawInput, ViewportInfo};

    use super::*;

    /// 同 eframe：不嵌入、immediate viewport 在自己的 `run_ui` 裡跑。
    fn native_like_context() -> Context {
        let ctx = Context::default();
        ctx.set_embed_viewports(false);
        Context::set_immediate_viewport_renderer(|ctx, mut viewport| {
            let mut input = RawInput {
                viewport_id: viewport.ids.this,
                ..Default::default()
            };
            input
                .viewports
                .insert(viewport.ids.this, ViewportInfo::default());
            ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui))
                .textures_delta
                .clear();
        });
        ctx
    }

    /// 根 viewport 的輸入：`windows` 裡的 viewport 已有原生視窗。
    fn input_with_windows(windows: &[ViewportId]) -> RawInput {
        let mut input = RawInput::default();
        input
            .viewports
            .insert(ViewportId::ROOT, ViewportInfo::default());
        for &id in windows {
            let info = ViewportInfo {
                native_pixels_per_point: Some(1.0),
                ..Default::default()
            };
            input.viewports.insert(id, info);
        }
        input
    }

    /// 跑一幀根 viewport，依序對 `ids` 呼叫 [`show_immediate`]；回傳各自的結果、`before` 與輸出。
    fn frame(
        ctx: &Context,
        input: RawInput,
        ids: &[ViewportId],
        snapshot: isize,
    ) -> (Vec<Option<ViewportId>>, Vec<Vec<isize>>, egui::FullOutput) {
        let (mut shown, mut befores) = (Vec::new(), Vec::new());
        let mut output = ctx.run_ui(input, |ui| {
            for &id in ids {
                let mut before = vec![snapshot];
                shown.push(show_immediate(
                    ui.ctx(),
                    id,
                    "test",
                    ViewportBuilder::default(),
                    &mut before,
                    |_, _| id,
                ));
                befores.push(before);
            }
        });
        output.textures_delta.clear();
        (shown, befores, output)
    }

    #[test]
    fn a_viewport_without_a_window_is_registered_then_shown_on_the_next_frame() {
        let ctx = native_like_context();
        let id = ViewportId::from_hash_of("a");
        let (shown, _, out) = frame(&ctx, RawInput::default(), &[id], 1);
        assert_eq!(shown, [None]);
        let registered = &out.viewport_output[&id];
        assert!(registered.class == ViewportClass::Deferred);
        assert!(
            registered.viewport_ui_cb.is_some(),
            "registered as deferred"
        );
        assert!(
            out.viewport_output[&ViewportId::ROOT]
                .repaint_delay
                .is_zero()
        );

        let (shown, befores, out) = frame(&ctx, input_with_windows(&[id]), &[id], 2);
        assert_eq!(shown, [Some(id)]);
        assert_eq!(
            befores,
            [vec![1]],
            "the snapshot from the registering frame"
        );
        assert!(
            out.viewport_output[&id].viewport_ui_cb.is_none(),
            "immediate"
        );

        let (_, befores, _) = frame(&ctx, input_with_windows(&[id]), &[id], 3);
        assert_eq!(befores, [vec![3]], "later frames keep their own snapshot");
    }

    #[test]
    fn only_one_viewport_is_registered_per_frame() {
        let ctx = native_like_context();
        let (a, b) = (ViewportId::from_hash_of("a"), ViewportId::from_hash_of("b"));
        let (shown, _, out) = frame(&ctx, RawInput::default(), &[a, b], 1);
        assert_eq!(shown, [None, None]);
        assert!(out.viewport_output.contains_key(&a));
        assert!(
            !out.viewport_output.contains_key(&b),
            "waits for the next frame"
        );

        let (shown, befores, out) = frame(&ctx, input_with_windows(&[a]), &[a, b], 2);
        assert_eq!(shown, [Some(a), None]);
        assert_eq!(befores[0], [1]);
        assert!(out.viewport_output[&b].viewport_ui_cb.is_some());

        let (shown, befores, _) = frame(&ctx, input_with_windows(&[a, b]), &[a, b], 3);
        assert_eq!(shown, [Some(a), Some(b)]);
        assert_eq!(befores, [vec![3], vec![2]]);
    }

    #[test]
    fn a_discarded_pass_registers_the_same_viewport_again() {
        let ctx = native_like_context();
        let (a, b) = (ViewportId::from_hash_of("a"), ViewportId::from_hash_of("b"));
        let mut passes = 0;
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            passes += 1;
            for id in [a, b] {
                show_immediate(
                    ui.ctx(),
                    id,
                    "test",
                    ViewportBuilder::default(),
                    &mut Vec::new(),
                    |_, _| (),
                );
            }
            if passes == 1 {
                ui.ctx().request_discard("test relayout");
            }
        });
        output.textures_delta.clear();
        assert_eq!(passes, 2);
        assert!(output.viewport_output.contains_key(&a));
        assert!(!output.viewport_output.contains_key(&b));
    }

    #[test]
    fn embedded_viewports_are_shown_on_the_first_frame() {
        let ctx = Context::default();
        let id = ViewportId::from_hash_of("a");
        let (shown, befores, _) = frame(&ctx, RawInput::default(), &[id], 1);
        assert_eq!(shown, [Some(id)]);
        assert_eq!(befores, [vec![1]]);
    }
}
