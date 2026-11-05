//! 版本說明裡的表格：egui_commonmark 把表格包進水平捲動區，太寬時右側欄被切掉、捲軸還會浮到可見範圍底部；這裡改成自己排版，整張塞進可用寬度。

use std::iter::once;

use eframe::egui::{
    Align, CornerRadius, Frame, Layout, Margin, Rect, Shape, Stroke, TextStyle, Ui, Vec2, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::theme;

/// 儲存格內距。
const CELL_PAD: Margin = Margin::symmetric(10, 6);
/// 量欄寬用純文字，實際排版的粗體、行內程式碼會寬一點；多留一點免得短短的數字欄被擠到換行。
const CELL_SLACK: f32 = 4.0;
/// 表格外框圓角。
const RADIUS: u8 = 6;
/// 第一欄（說明文字）縮窄時至少保留的比例。
const FIRST_COLUMN_MIN: f32 = 0.4;

/// 內文的一段：一般 Markdown，或一張表格。
#[derive(Debug, PartialEq)]
pub enum Block<'a> {
    Markdown(&'a str),
    Table(Table),
}

/// 一張表格：標題列與資料列。
#[derive(Debug, Default, PartialEq)]
pub struct Table {
    pub header: Vec<Cell>,
    pub rows: Vec<Vec<Cell>>,
}

/// 一個儲存格：原始 Markdown（照樣交給 egui_commonmark 畫，保留樣式與連結）與量欄寬用的純文字。
#[derive(Debug, Default, PartialEq)]
pub struct Cell {
    pub source: String,
    pub text: String,
}

/// 把內文切成 Markdown 段落與表格，順序不變；空白的段落略過。
pub fn split_tables(body: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut markdown_start = 0;
    let mut table: Option<Table> = None;
    let (mut row, mut cell) = (Vec::new(), Cell::default());
    for (event, range) in Parser::new_ext(body, Options::ENABLE_TABLES).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(_)) => {
                let text = &body[markdown_start..range.start];
                if !text.trim().is_empty() {
                    blocks.push(Block::Markdown(text));
                }
                table = Some(Table::default());
            }
            Event::End(TagEnd::Table) => {
                blocks.extend(table.take().map(Block::Table));
                markdown_start = range.end;
            }
            Event::Start(Tag::TableCell) => {
                cell = Cell {
                    source: body[range].trim().to_owned(),
                    text: String::new(),
                };
            }
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut cell)),
            Event::End(TagEnd::TableHead) => {
                if let Some(table) = &mut table {
                    table.header = std::mem::take(&mut row);
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(table) = &mut table {
                    table.rows.push(std::mem::take(&mut row));
                }
            }
            Event::Text(text) | Event::Code(text) if table.is_some() => cell.text.push_str(&text),
            Event::SoftBreak | Event::HardBreak if table.is_some() => cell.text.push(' '),
            _ => {}
        }
    }
    let text = &body[markdown_start..];
    if !text.trim().is_empty() {
        blocks.push(Block::Markdown(text));
    }
    blocks
}

/// 各欄的文字寬：放得下就照內容寬、多出來的給第一欄；放不下先縮第一欄（至少留四成），其餘欄再按比例縮。
pub fn column_widths(natural: &[f32], available: f32) -> Vec<f32> {
    let Some((&first, rest)) = natural.split_first() else {
        return Vec::new();
    };
    let rest_total: f32 = rest.iter().sum();
    if first + rest_total <= available {
        return once(available - rest_total)
            .chain(rest.iter().copied())
            .collect();
    }
    let first = (available - rest_total).max(available * FIRST_COLUMN_MIN);
    let scale = ((available - first) / rest_total).min(1.0);
    once(first).chain(rest.iter().map(|w| w * scale)).collect()
}

/// 畫一張表格：佔滿可用寬度，儲存格依欄寬換行；標題列粗體、資料列隔列加底色。
pub fn show(ui: &mut Ui, table: &Table, cache: &mut CommonMarkCache) {
    let columns = table
        .rows
        .iter()
        .map(Vec::len)
        .chain(once(table.header.len()))
        .max()
        .unwrap_or(0);
    if columns == 0 {
        return;
    }
    let body = TextStyle::Body.resolve(ui.style());
    let lines = once((&table.header, true)).chain(table.rows.iter().map(|row| (row, false)));
    Frame::new()
        .stroke(Stroke::new(1.0, theme::CARD_BORDER))
        .corner_radius(RADIUS)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let (left, width) = (ui.cursor().left(), ui.available_width());
            let natural: Vec<f32> = (0..columns)
                .map(|c| {
                    let widest = lines
                        .clone()
                        .filter_map(|(row, header)| Some((row.get(c)?, header)))
                        .map(|(cell, header)| {
                            let font = if header {
                                theme::bold(body.size)
                            } else {
                                body.clone()
                            };
                            ui.fonts_mut(|f| {
                                f.layout_no_wrap(cell.text.clone(), font, theme::TEXT)
                                    .size()
                                    .x
                            })
                        })
                        .fold(0.0, f32::max);
                    widest + CELL_SLACK
                })
                .collect();
            let padding = CELL_PAD.sum().x * columns as f32;
            let widths = column_widths(&natural, (width - padding).max(0.0));
            let last = table.rows.len();
            for (i, (row, header)) in lines.enumerate() {
                let background = ui.painter().add(Shape::Noop);
                let cells = ui.horizontal_top(|ui| {
                    for (c, &text_width) in widths.iter().enumerate() {
                        let size = vec2(text_width + CELL_PAD.sum().x, 0.0);
                        ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                            Frame::new().inner_margin(CELL_PAD).show(ui, |ui| {
                                ui.set_width(text_width);
                                if header {
                                    // Markdown 的粗體只是加亮，標題列要真的粗體字型
                                    ui.style_mut()
                                        .text_styles
                                        .insert(TextStyle::Body, theme::bold(body.size));
                                }
                                if let Some(cell) =
                                    row.get(c).filter(|cell| !cell.source.is_empty())
                                {
                                    CommonMarkViewer::new().show(ui, cache, &cell.source);
                                }
                            });
                        });
                    }
                });
                let rect =
                    Rect::from_x_y_ranges(left..=left + width, cells.response.rect.y_range());
                if header {
                    let line = Stroke::new(1.0, theme::CARD_BORDER);
                    ui.painter().hline(rect.x_range(), rect.bottom(), line);
                } else if i % 2 == 0 {
                    // 最後一列的底色要跟著外框圓角，不然方角會凸出框線
                    let corner = if i == last { RADIUS } else { 0 };
                    let radius = CornerRadius {
                        sw: corner,
                        se: corner,
                        ..CornerRadius::ZERO
                    };
                    let fill = Shape::rect_filled(rect, radius, theme::MODAL_FOOT);
                    ui.painter().set(background, fill);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(source: &str, text: &str) -> Cell {
        Cell {
            source: source.into(),
            text: text.into(),
        }
    }

    /// 表格前後的段落原樣保留；儲存格留下原始 Markdown 與純文字。
    #[test]
    fn splits_markdown_around_a_table() {
        let body = "Intro\n\n| | **a** | b |\n|---|---|---|\n| [x](https://e.com) | 1 | 2 |\n| y \\| z | `3` | 4 |\n\nOutro\n";
        let blocks = split_tables(body);
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert_eq!(blocks[0], Block::Markdown("Intro\n\n"));
        assert_eq!(
            blocks[1],
            Block::Table(Table {
                header: vec![cell("", ""), cell("**a**", "a"), cell("b", "b")],
                rows: vec![
                    vec![
                        cell("[x](https://e.com)", "x"),
                        cell("1", "1"),
                        cell("2", "2")
                    ],
                    vec![cell("y \\| z", "y | z"), cell("`3`", "3"), cell("4", "4")],
                ],
            })
        );
        assert!(matches!(blocks[2], Block::Markdown(text) if text.trim() == "Outro"));
    }

    /// 程式碼區塊裡長得像表格的文字不當成表格。
    #[test]
    fn ignores_tables_inside_code_blocks() {
        let body = "```\n| a | b |\n|---|---|\n```\n";
        assert_eq!(split_tables(body), vec![Block::Markdown(body)]);
    }

    /// 放得下時多出的寬度給第一欄；放不下時第一欄先縮，其餘欄不足時按比例縮，總寬不超過可用寬。
    #[test]
    fn column_widths_fit_the_available_width() {
        assert_eq!(
            column_widths(&[100.0, 50.0, 50.0], 300.0),
            vec![200.0, 50.0, 50.0]
        );
        assert_eq!(
            column_widths(&[300.0, 50.0, 50.0], 300.0),
            vec![200.0, 50.0, 50.0]
        );
        assert_eq!(
            column_widths(&[300.0, 150.0, 150.0], 300.0),
            vec![120.0, 90.0, 90.0]
        );
    }
}
