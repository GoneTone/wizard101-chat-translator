//! 框選用的全螢幕選取畫面：蓋滿遊戲所在的螢幕，顯示事先凍結的遊戲畫面，拖曳畫矩形、放開回報螢幕座標。
//!
//! 這裡只有狀態與 egui 內容；視窗由 App 托管。整層不透明：整顆螢幕截圖上疊遊戲畫面，兩者繪製時都以 `DIM_TINT` 著色變暗，
//! 拖曳中的範圍內用原亮度的遊戲畫面聚光（只作用在遊戲畫面範圍內）。點一下沒拖動、Esc、右鍵都算取消。

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, ColorImage, CursorIcon, Event, FontId, Key, Margin, PointerButton,
    RichText, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};
use image::RgbImage;

use crate::i18n::t;
use crate::region::capture::Frame;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::{Rect, is_click};
use crate::ui::palette::{BAR, FG_TRANSLATED, FG_UPDATE, GRIP};

/// 底圖與遊戲畫面的暗化著色（≈ 0.45 倍）；繪製時乘上去，不另做暗化副本。
const DIM_TINT: Color32 = Color32::from_gray(115);
/// 框線寬度（DIP）。
pub const BAND_WIDTH: f32 = 2.0;
/// 提示文字與螢幕頂端的距離（DIP）。
pub const HINT_Y: f32 = 40.0;

const FULL_UV: egui::Rect = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// `content` 這一幀的結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectorAction {
    /// 放開滑鼠：框選的螢幕矩形（實體像素）。
    Selected(Rect),
    /// 點一下沒拖動、Esc 或右鍵。
    Cancelled,
}

struct Textures {
    backdrop: TextureHandle,
    frame: TextureHandle,
}

/// 選取畫面的狀態；每次框選建一個新的。
///
/// 兩張圖在 `new` 時就轉好，第一次 `content` 才上傳成貼圖（需要 egui Context），之後每幀沿用。
pub struct Selector {
    monitor: Rect,
    frame: Arc<Frame>,
    frame_pos: (i32, i32),
    /// 待上傳的 backdrop 與遊戲畫面（皆原亮度）；上傳後清空。
    images: Option<[ColorImage; 2]>,
    textures: Option<Textures>,
    /// 按下的位置（相對選取畫面左上角的 DIP）。
    start: Option<Vec2>,
}

impl Selector {
    /// `monitor` 是遊戲所在螢幕的矩形（實體像素），`images` 是 [`images`] 轉好的 `[backdrop, 遊戲畫面]`。
    pub fn new(monitor: Rect, frame: Arc<Frame>, images: [ColorImage; 2]) -> Self {
        let frame_pos = (
            frame.client_origin.0 - monitor.0,
            frame.client_origin.1 - monitor.1,
        );
        Self {
            monitor,
            frame,
            frame_pos,
            images: Some(images),
            textures: None,
            start: None,
        }
    }

    /// 選取畫面蓋住的螢幕矩形（實體像素）。
    pub fn monitor(&self) -> Rect {
        self.monitor
    }

    /// 選取畫面顯示的凍結畫面；框選完從同一張裁切。
    pub fn frame(&self) -> &Arc<Frame> {
        &self.frame
    }

    /// 還沒上傳成貼圖的 backdrop 大小（測試用）。
    #[cfg(test)]
    pub(crate) fn backdrop_size(&self) -> Option<[usize; 2]> {
        self.images.as_ref().map(|[backdrop, _]| backdrop.size)
    }

    /// 畫出選取畫面並處理滑鼠與 Esc；選取畫面要佔滿 `ui` 的範圍、與螢幕左上角對齊。
    pub fn content(&mut self, ui: &mut egui::Ui) -> Option<SelectorAction> {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let area = ui.max_rect();
        // 起訖點取按鍵事件自帶的位置：eframe 會把兩次重繪間的事件併成一幀，interact_pos 只剩最後一個
        let (buttons, secondary, escape, pointer) = ctx.input(|i| {
            let buttons: Vec<(Vec2, bool)> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        ..
                    } => Some((*pos - area.min, *pressed)),
                    _ => None,
                })
                .collect();
            (
                buttons,
                i.pointer.secondary_pressed(),
                i.key_pressed(Key::Escape),
                i.pointer.latest_pos().map(|p| p - area.min),
            )
        });
        ctx.set_cursor_icon(CursorIcon::Crosshair);

        let mut action = None;
        if escape || secondary {
            self.start = None;
            action = Some(SelectorAction::Cancelled);
        } else {
            for (pos, pressed) in buttons {
                if pressed {
                    self.start = Some(pos);
                } else if let Some(start) = self.start.take() {
                    action = Some(self.finish(start, pos, ppp));
                    break;
                }
            }
        }
        self.paint(ui, area, ppp, pointer);
        hint(ui);
        action
    }

    fn finish(&self, start: Vec2, end: Vec2, ppp: f32) -> SelectorAction {
        let (a, b) = (to_physical(start, ppp), to_physical(end, ppp));
        if is_click(b.0 - a.0, b.1 - a.1) {
            return SelectorAction::Cancelled;
        }
        let (x, y, w, h) = drag_rect(a, b);
        if w == 0 || h == 0 {
            return SelectorAction::Cancelled;
        }
        SelectorAction::Selected((self.monitor.0 + x, self.monitor.1 + y, w, h))
    }

    fn paint(&mut self, ui: &mut egui::Ui, area: egui::Rect, ppp: f32, pointer: Option<Vec2>) {
        let to_points = |(x, y, w, h): Rect| {
            egui::Rect::from_min_size(
                area.min + vec2(x as f32, y as f32) / ppp,
                vec2(w as f32, h as f32) / ppp,
            )
        };
        let (fx, fy) = self.frame_pos;
        let textures = self.textures(ui.ctx());
        let [bw, bh] = textures.backdrop.size();
        let [iw, ih] = textures.frame.size();
        let (backdrop, frame) = (textures.backdrop.id(), textures.frame.id());

        let painter = ui.painter().clone();
        painter.rect_filled(area, 0.0, Color32::BLACK);
        painter.image(
            backdrop,
            to_points((0, 0, bw as i32, bh as i32)),
            FULL_UV,
            DIM_TINT,
        );
        painter.image(
            frame,
            to_points((fx, fy, iw as i32, ih as i32)),
            FULL_UV,
            DIM_TINT,
        );

        let (Some(start), Some(end)) = (self.start, pointer) else {
            return;
        };
        let (a, b) = (to_physical(start, ppp), to_physical(end, ppp));
        let drag = drag_rect(a, b);
        if let Some(spot) = spotlight_rect(drag, self.frame_pos, self.frame.client_size) {
            let (x, y, w, h) = spot;
            let uv = egui::Rect::from_min_size(
                pos2((x - fx) as f32 / iw as f32, (y - fy) as f32 / ih as f32),
                vec2(w as f32 / iw as f32, h as f32 / ih as f32),
            );
            painter.image(frame, to_points(spot), uv, Color32::WHITE);
        }
        let band = to_points(drag);
        painter.rect_stroke(
            band,
            0.0,
            Stroke::new(BAND_WIDTH, FG_UPDATE),
            StrokeKind::Middle,
        );
        if a != b {
            let galley = painter.layout_no_wrap(
                format!("{}×{}", drag.2, drag.3),
                FontId::proportional(tk_pt(9.0)),
                FG_UPDATE,
            );
            let size = galley.size();
            let at = pos2(band.min.x, band.min.y - 2.0 - size.y);
            ui.put(
                egui::Rect::from_min_size(at, size),
                egui::Label::new(galley).selectable(false),
            );
        }
    }

    fn textures(&mut self, ctx: &egui::Context) -> &Textures {
        if let Some([backdrop, frame]) = self.images.take() {
            let load = |name: &str, image| ctx.load_texture(name, image, TextureOptions::NEAREST);
            self.textures = Some(Textures {
                backdrop: load("region_backdrop", backdrop),
                frame: load("region_frame", frame),
            });
        }
        self.textures
            .as_ref()
            .expect("images are uploaded on the first frame")
    }
}

/// 把同一顆螢幕的整張截圖與遊戲畫面轉成 `[backdrop, 遊戲畫面]`，供 [`Selector::new`] 使用。
pub fn images(frame: &Frame, backdrop: &RgbImage) -> [ColorImage; 2] {
    [color_image(backdrop), color_image(&frame.image)]
}

fn color_image(image: &RgbImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    ColorImage::from_rgb(size, image.as_raw())
}

/// 提示文字：頂端置中、不透明底，不接滑鼠（拖曳經過時照樣框得到底下）。
fn hint(ui: &egui::Ui) {
    egui::Area::new(egui::Id::unique("region_hint"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, HINT_Y))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(BAR)
                .stroke(Stroke::new(1.0, GRIP))
                .inner_margin(Margin::symmetric(16, 8))
                .show(ui, |ui| {
                    let text = RichText::new(t("region.hint", &[]))
                        .font(FontId::proportional(tk_pt(13.0)))
                        .color(FG_TRANSLATED);
                    ui.add(egui::Label::new(text).extend().selectable(false));
                });
        });
}

/// 選取畫面內的 DIP 座標換成實體像素（螢幕內座標）。
pub(crate) fn to_physical(v: Vec2, ppp: f32) -> (i32, i32) {
    ((v.x * ppp).round() as i32, (v.y * ppp).round() as i32)
}

/// 兩個角點圍出的正規化矩形（左上角加寬高）。
fn drag_rect(a: (i32, i32), b: (i32, i32)) -> Rect {
    (
        a.0.min(b.0),
        a.1.min(b.1),
        (b.0 - a.0).abs(),
        (b.1 - a.1).abs(),
    )
}

/// 拖曳矩形與遊戲畫面範圍的交集（螢幕內座標、實體像素）；沒有交集時回 `None`。
pub fn spotlight_rect(drag: Rect, frame_pos: (i32, i32), frame_size: (i32, i32)) -> Option<Rect> {
    let left = drag.0.max(frame_pos.0);
    let top = drag.1.max(frame_pos.1);
    let right = (drag.0 + drag.2).min(frame_pos.0 + frame_size.0);
    let bottom = (drag.1 + drag.3).min(frame_pos.1 + frame_size.1);
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Event, Key, Modifiers, PointerButton, Shape, TextureId, Vec2, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use image::Rgb;

    use super::*;
    use crate::i18n::t;
    use crate::ui::fonts_egui;

    // 第二顆螢幕：矩形要加上螢幕原點
    const MONITOR: Rect = (1920, 0, 800, 600);

    /// 遊戲 client 區左上角在螢幕 (1920 + 100, 50)，換成這顆螢幕內的座標是 (100, 50)。
    fn frame() -> Arc<Frame> {
        Arc::new(Frame {
            image: RgbImage::from_pixel(400, 300, Rgb([255, 255, 255])),
            client_origin: (1920 + 100, 50),
            client_size: (400, 300),
        })
    }

    struct State {
        selector: Selector,
        actions: Vec<SelectorAction>,
        area: egui::Rect,
    }

    fn harness(ppp: f32) -> Harness<'static, State> {
        let frame = frame();
        let images = super::images(&frame, &RgbImage::new(800, 600));
        let state = State {
            selector: Selector::new(MONITOR, frame, images),
            actions: Vec::new(),
            area: egui::Rect::NOTHING,
        };
        let mut harness = Harness::builder()
            .with_size(vec2(800.0 / ppp + 16.0, 600.0 / ppp + 16.0))
            .with_pixels_per_point(ppp)
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui_state(
                |ui, s: &mut State| {
                    s.area = ui.max_rect();
                    s.actions.extend(s.selector.content(ui));
                },
                state,
            );
        harness.step();
        harness
    }

    /// 在 `from`（相對選取畫面左上角的 DIP）按下並移到 `to`，不放開。
    fn press_and_move(h: &mut Harness<State>, from: Vec2, to: Vec2) {
        let origin = h.state().area.min;
        h.hover_at(origin + from);
        h.drag_at(origin + from);
        h.step();
        h.hover_at(origin + to);
        h.step();
    }

    fn drag(h: &mut Harness<State>, from: Vec2, to: Vec2) {
        press_and_move(h, from, to);
        h.drop_at(h.state().area.min + to);
        h.step();
    }

    /// 這幀畫出的貼圖：（貼圖、位置、UV 範圍、頂點著色），依疊放順序。
    fn images(h: &Harness<State>) -> Vec<(TextureId, egui::Rect, egui::Rect, Color32)> {
        h.output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => {
                    let uv = mesh.vertices.iter().fold(egui::Rect::NOTHING, |r, v| {
                        r.union(egui::Rect::from_pos(v.uv))
                    });
                    Some((
                        mesh.texture_id,
                        mesh.calc_bounds(),
                        uv,
                        mesh.vertices[0].color,
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn texture_ids(h: &Harness<State>) -> (TextureId, TextureId) {
        let t = h.state().selector.textures.as_ref().unwrap();
        (t.backdrop.id(), t.frame.id())
    }

    #[test]
    fn drag_reports_the_screen_rect() {
        let mut h = harness(1.0);
        // 從右下拖到左上也要正規化
        drag(&mut h, vec2(400.0, 300.0), vec2(100.0, 150.0));
        assert_eq!(
            h.state().actions,
            [SelectorAction::Selected((1920 + 100, 150, 300, 150))]
        );
    }

    #[test]
    fn drag_reports_physical_pixels_on_scaled_displays() {
        let mut h = harness(2.0);
        drag(&mut h, vec2(200.0, 150.0), vec2(50.0, 75.0));
        assert_eq!(
            h.state().actions,
            [SelectorAction::Selected((1920 + 100, 150, 300, 150))]
        );
    }

    #[test]
    fn press_move_and_release_in_one_frame_use_the_button_positions() {
        let mut h = harness(1.0);
        let origin = h.state().area.min;
        let button = |at: Vec2, pressed| Event::PointerButton {
            pos: origin + at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        // kittest 的 step 一幀只送一個排隊事件；直接塞進 RawInput 才能模擬 eframe 併成一幀
        h.input_mut().events.extend([
            button(vec2(100.0, 150.0), true),
            Event::PointerMoved(origin + vec2(300.0, 250.0)),
            button(vec2(400.0, 300.0), false),
            Event::PointerMoved(origin + vec2(500.0, 500.0)),
        ]);
        h.step();
        assert_eq!(
            h.state().actions,
            [SelectorAction::Selected((1920 + 100, 150, 300, 150))]
        );
    }

    #[test]
    fn rubber_band_is_drawn_while_dragging() {
        let mut h = harness(1.0);
        let origin = h.state().area.min;
        press_and_move(&mut h, vec2(10.0, 20.0), vec2(110.0, 70.0));
        let band = egui::Rect::from_min_max(origin + vec2(10.0, 20.0), origin + vec2(110.0, 70.0));
        let drawn = h.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, Shape::Rect(r)
                if r.rect == band && r.stroke == Stroke::new(BAND_WIDTH, FG_UPDATE))
        });
        assert!(drawn);
    }

    #[test]
    fn points_convert_to_physical_pixels() {
        assert_eq!(to_physical(vec2(10.0, 20.0), 1.0), (10, 20));
        assert_eq!(to_physical(vec2(10.0, 20.0), 1.25), (13, 25));
        assert_eq!(to_physical(vec2(10.4, 0.2), 1.5), (16, 0));
    }

    #[test]
    fn a_click_without_drag_cancels() {
        let mut h = harness(1.0);
        drag(&mut h, vec2(100.0, 100.0), vec2(102.0, 101.0));
        assert_eq!(h.state().actions, [SelectorAction::Cancelled]);
    }

    #[test]
    fn a_zero_width_or_height_drag_cancels() {
        for to in [vec2(300.0, 100.0), vec2(100.0, 300.0)] {
            let mut h = harness(1.0);
            drag(&mut h, vec2(100.0, 100.0), to);
            assert_eq!(h.state().actions, [SelectorAction::Cancelled]);
        }
    }

    #[test]
    fn escape_cancels() {
        let mut h = harness(1.0);
        press_and_move(&mut h, vec2(10.0, 20.0), vec2(130.0, 60.0));
        h.key_press(Key::Escape);
        h.step();
        assert_eq!(h.state().actions, [SelectorAction::Cancelled]);
    }

    #[test]
    fn right_click_cancels() {
        let mut h = harness(1.0);
        h.event(Event::PointerButton {
            pos: h.state().area.min + vec2(50.0, 50.0),
            button: PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        assert_eq!(h.state().actions, [SelectorAction::Cancelled]);
    }

    #[test]
    fn spotlight_is_clipped_to_the_game_frame() {
        let (pos, size) = ((100, 50), (400, 300));
        assert_eq!(
            spotlight_rect((120, 80, 100, 70), pos, size),
            Some((120, 80, 100, 70))
        );
        assert_eq!(
            spotlight_rect((50, 20, 200, 100), pos, size),
            Some((100, 50, 150, 70))
        );
        assert_eq!(
            spotlight_rect((450, 300, 200, 100), pos, size),
            Some((450, 300, 50, 50))
        );
        assert_eq!(spotlight_rect((0, 0, 100, 50), pos, size), None);
        assert_eq!(spotlight_rect((600, 400, 50, 50), pos, size), None);
    }

    #[test]
    fn dimmed_backdrop_and_frame_are_placed_in_order() {
        let h = harness(1.0);
        let origin = h.state().area.min;
        let (backdrop, frame) = texture_ids(&h);
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        assert_eq!(
            images(&h),
            [
                (
                    backdrop,
                    egui::Rect::from_min_size(origin, vec2(800.0, 600.0)),
                    full,
                    DIM_TINT
                ),
                (
                    frame,
                    egui::Rect::from_min_size(origin + vec2(100.0, 50.0), vec2(400.0, 300.0)),
                    full,
                    DIM_TINT
                ),
            ]
        );
    }

    #[test]
    fn dimmed_frame_is_placed_in_points_on_scaled_displays() {
        let h = harness(2.0);
        let origin = h.state().area.min;
        let placed = images(&h)[1].1;
        assert_eq!(
            placed,
            egui::Rect::from_min_size(origin + vec2(50.0, 25.0), vec2(200.0, 150.0))
        );
    }

    #[test]
    fn spotlight_shows_the_dragged_area_and_hides_on_the_next_press() {
        let mut h = harness(1.0);
        let origin = h.state().area.min;
        let (_, full_frame) = texture_ids(&h);
        // 拖出遊戲畫面左上角外的部分不聚光
        press_and_move(&mut h, vec2(50.0, 20.0), vec2(220.0, 150.0));
        let spot = images(&h)[2];
        assert_eq!(spot.0, full_frame);
        assert_eq!(spot.3, Color32::WHITE);
        assert_eq!(
            spot.1,
            egui::Rect::from_min_size(origin + vec2(100.0, 50.0), vec2(120.0, 100.0))
        );
        assert_eq!(
            spot.2,
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(120.0 / 400.0, 100.0 / 300.0))
        );
        h.drop_at(origin + vec2(220.0, 150.0));
        h.step();
        h.drag_at(origin + vec2(130.0, 90.0));
        h.step();
        assert_eq!(images(&h).len(), 2);
    }

    #[test]
    fn size_label_shows_w_x_h_while_dragging() {
        let mut h = harness(1.0);
        press_and_move(&mut h, vec2(10.0, 20.0), vec2(130.0, 60.0));
        h.get_by_label("120×40");
    }

    #[test]
    fn hint_text_is_shown() {
        let h = harness(1.0);
        h.get_by_label(&t("region.hint", &[]));
    }
}
