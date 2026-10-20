//! 本機 OCR：rusto-rs（PP-OCR 模型的純 Rust 推論），PNG bytes → 文字；框選翻譯唯一的辨識路徑。
//!
//! 引擎延到第一次辨識才建立：載入模型要一段時間，程式啟動不該為可能用不到的功能先付這筆成本。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use rusto::{
    DetectTextResult, DetectionRunOptions, ImageSource, InitializeConfig, OcrRunOptions, RustO,
};

use crate::config::local_state_dir;
use crate::logs::log;
use crate::util::{lock, sha1_head};

const DET: &str = "PP-OCRv6_det_small.onnx";
const REC: &str = "PP-OCRv6_rec_small.onnx";
const CLS: &str = "ch_ppocr_mobile_v2.0_cls_mobile.onnx";
const KEYS: &str = "ppocrv6_keys.txt";
/// 偵測時把短邊放大到的長度（RapidOCR 的 `limit_side_len`）。
const DET_SHORT_SIDE: u32 = 736;
/// 偵測放大後長邊的上限：細長的聊天裁切照短邊放大會寬達好幾千像素，推論慢好幾倍。
const DET_LONG_SIDE_CAP: u32 = 2000;

/// 沒註冊模型時（測試、`cargo run --example`）直接讀 repo 內的模型檔。
const ASSETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ocr");

static REGISTERED: OnceLock<Models> = OnceLock::new();
static ENGINE: Mutex<Option<RustO>> = Mutex::new(None);

/// 編進 exe 的模型與字典；rusto-rs 只吃檔案路徑，第一次辨識時才解到磁碟。
pub struct Models {
    pub det: &'static [u8],
    pub rec: &'static [u8],
    pub cls: &'static [u8],
    pub keys: &'static [u8],
}

/// 本機 OCR 不可用：模型解不出來或引擎建立失敗；附上原因。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrUnavailable(pub String);

/// 辨識失敗：引擎起不來（`Unavailable`），或引擎已建好、辨識這張圖時出錯（`Failed`，附上原因）。
#[derive(Debug, Clone, PartialEq)]
pub enum OcrError {
    Unavailable(OcrUnavailable),
    Failed(String),
}

impl From<OcrUnavailable> for OcrError {
    fn from(e: OcrUnavailable) -> Self {
        Self::Unavailable(e)
    }
}

/// 一個辨識框：4 個角點（左上、右上、右下、左下）與框內文字。
pub struct OcrBox {
    pub points: [(f32, f32); 4],
    pub text: String,
}

/// 交出編進 exe 的模型；由 main 呼叫一次，之後再呼叫不會生效。
pub fn register_models(models: Models) {
    let _ = REGISTERED.set(models);
}

/// 把模型解到 `dir/<四個檔案合起來的 sha1 前 12 碼>/` 並回傳該目錄；內容相同的檔案不重寫，舊版模型的資料夾順手清掉。
pub fn ensure_extracted(dir: &Path, models: &Models) -> io::Result<PathBuf> {
    let files = [
        (DET, models.det),
        (REC, models.rec),
        (CLS, models.cls),
        (KEYS, models.keys),
    ];
    let target = dir.join(sha1_head(&files.map(|(_, bytes)| bytes)));
    fs::create_dir_all(&target)?;
    for (name, bytes) in files {
        let path = target.join(name);
        if fs::read(&path).ok().as_deref() != Some(bytes) {
            fs::write(&path, bytes)?;
        }
    }
    remove_stale_dirs(dir, &target);
    Ok(target)
}

/// 刪掉 `dir` 底下除了 `keep` 以外的資料夾（換模型後留下的舊解出目錄）；刪不掉只記 log。
fn remove_stale_dirs(dir: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        if path == keep || !path.is_dir() {
            continue;
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => log(format!("[ocr] removed stale model dir {}", path.display())),
            Err(error) => log(format!(
                "[ocr] could not remove stale model dir {}: {error}",
                path.display()
            )),
        }
    }
}

/// 把逐框辨識的結果併回可讀的行（同 Python `merge_lines`）。
///
/// 同一視覺行常被切成好幾個框：以框中心 y 分組，與該行平均 y 差距小於「框高度中位數的一半」
/// 就算同一行；行由上到下、行內由左到右排，以單一空白接起來。
pub fn merge_lines(items: &[OcrBox]) -> Vec<String> {
    if items.is_empty() {
        return Vec::new();
    }
    // 用 f64 計算，與 Python 的 float 逐位元相同，邊界案例才不會分歧。
    let ys = |b: &OcrBox| b.points.map(|(_, y)| f64::from(y));
    let y_center = |b: &OcrBox| ys(b).iter().sum::<f64>() / 4.0;
    let height = |b: &OcrBox| {
        let ys = ys(b);
        ys.iter().copied().fold(f64::MIN, f64::max) - ys.iter().copied().fold(f64::MAX, f64::min)
    };
    let x_left = |b: &OcrBox| {
        b.points
            .iter()
            .map(|&(x, _)| f64::from(x))
            .fold(f64::MAX, f64::min)
    };

    let mut heights: Vec<f64> = items.iter().map(height).collect();
    heights.sort_by(f64::total_cmp);
    let n = heights.len();
    let median = if n % 2 == 1 {
        heights[n / 2]
    } else {
        (heights[n / 2 - 1] + heights[n / 2]) / 2.0
    };
    let threshold = median / 2.0;

    let mut ordered: Vec<&OcrBox> = items.iter().collect();
    ordered.sort_by(|a, b| y_center(a).total_cmp(&y_center(b)));
    let mut lines: Vec<(f64, Vec<&OcrBox>)> = Vec::new();
    for b in ordered {
        let yc = y_center(b);
        match lines.last_mut() {
            Some((line_y, line)) if (yc - *line_y).abs() < threshold => {
                line.push(b);
                *line_y = line.iter().map(|b| y_center(b)).sum::<f64>() / line.len() as f64;
            }
            _ => lines.push((yc, vec![b])),
        }
    }
    lines
        .into_iter()
        .map(|(_, mut line)| {
            line.sort_by(|a, b| x_left(a).total_cmp(&x_left(b)));
            line.iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// 辨識 PNG 裡的文字，各行以 `\n` 連接（見 `merge_lines`）；沒有文字回空字串。
///
/// 會阻塞到辨識完成（第一次還要建立引擎），不可在 UI 或 async worker 執行緒上呼叫。
pub fn recognize(png: &[u8]) -> Result<String, OcrError> {
    recognize_with(
        &mut lock(&ENGINE),
        REGISTERED.get(),
        &local_state_dir().join("ocr"),
        png,
    )
}

/// `recognize` 的本體：引擎放在 `engine`，有 `models` 時先解到 `extract_root` 底下。
///
/// 引擎建立失敗不會留在 `engine` 裡，下次呼叫會重新解出、重新建立。
pub fn recognize_with(
    engine: &mut Option<RustO>,
    models: Option<&Models>,
    extract_root: &Path,
    png: &[u8],
) -> Result<String, OcrError> {
    let started = Instant::now();
    let engine = match engine {
        Some(engine) => engine,
        None => engine.insert(create_engine(models, extract_root)?),
    };
    let result = engine
        .detect_text(&ImageSource::Bytes(png.to_vec()), &run_options(png))
        .map_err(|e| {
            log(format!("[region] rusto detection failed: {e}"));
            OcrError::Failed(e.to_string())
        })?;
    let DetectTextResult::Structured(results) = result else {
        unreachable!("line output is always structured");
    };
    let boxes: Vec<OcrBox> = results
        .into_iter()
        .map(|r| OcrBox {
            points: r.box_points,
            text: r.text,
        })
        .collect();
    let lines = merge_lines(&boxes);
    log(format!(
        "[region] local OCR done (boxes={}, lines={}, ms={})",
        boxes.len(),
        lines.len(),
        started.elapsed().as_millis()
    ));
    Ok(lines.join("\n"))
}

/// rten 推論執行緒數：使用者設了 `RTEN_NUM_THREADS` 就不動（回 None），否則用一半的邏輯核心。
pub fn inference_threads(user_set: bool, parallelism: usize) -> Option<usize> {
    (!user_set).then_some((parallelism / 2).max(1))
}

/// 偵測的 `limit_side_len`：短邊照常放大到 736，但放大後長邊約不超過 2000。
pub fn det_limit_side_len(width: u32, height: u32) -> i32 {
    let (short, long) = (width.min(height), width.max(height).max(1));
    let capped = u64::from(DET_LONG_SIDE_CAP) * u64::from(short) / u64::from(long);
    capped.min(u64::from(DET_SHORT_SIDE)) as i32
}

/// 這張圖的辨識參數；讀不到尺寸就用預設值，交給 rusto 回報錯誤。
fn run_options(png: &[u8]) -> OcrRunOptions {
    let size = image::ImageReader::new(io::Cursor::new(png))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok());
    let Some((width, height)) = size else {
        return OcrRunOptions::default();
    };
    OcrRunOptions {
        detection: Some(DetectionRunOptions {
            limit_side_len: Some(det_limit_side_len(width, height)),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn create_engine(models: Option<&Models>, extract_root: &Path) -> Result<RustO, OcrUnavailable> {
    let dir = match models {
        Some(models) => ensure_extracted(extract_root, models).map_err(|e| {
            unavailable(format!(
                "cannot extract models under {}: {e}",
                extract_root.display()
            ))
        })?,
        None => PathBuf::from(ASSETS_DIR),
    };
    let engine = RustO::initialize(engine_config(&dir))
        .map_err(|e| unavailable(format!("failed to create rusto engine: {e}")))?;
    log("[region] local OCR engine ready (rusto)");
    Ok(engine)
}

/// 引擎參數照 RapidOCR 3.9.2 的 config.yaml；mean/std 改用 ImageNet 官方值（spec 唯一刻意的行為改變）。
fn engine_config(dir: &Path) -> InitializeConfig {
    InitializeConfig::ppv6(dir.join(DET), dir.join(REC), dir.join(KEYS))
        .with_min_side_len(30.0)
        .with_max_side_len(2000.0)
        .with_min_height(30.0)
        .with_width_height_ratio(8.0)
        .with_det_box_thresh(0.5)
        .with_unclip_ratio(1.6)
        .with_use_dilation(true)
        .with_limit_side_len(DET_SHORT_SIDE as i32)
        .with_limit_type("min")
        .with_det_thresh(0.3)
        .with_cls_threshold(dir.join(CLS), 0.9)
        .with_text_score(0.5)
        .with_det_mean([0.485, 0.456, 0.406])
        .with_det_std([0.229, 0.224, 0.225])
}

fn unavailable(reason: String) -> OcrUnavailable {
    log(format!("[region] local OCR unavailable: {reason}"));
    OcrUnavailable(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_preprocessing_matches_rapidocr() {
        let g = engine_config(Path::new(ASSETS_DIR)).global;
        assert_eq!(
            (
                g.min_side_len,
                g.max_side_len,
                g.min_height,
                g.width_height_ratio
            ),
            (30.0, 2000.0, 30.0, 8.0)
        );
    }

    #[test]
    fn inference_threads_default_to_half_unless_user_set() {
        assert_eq!(inference_threads(false, 24), Some(12));
        assert_eq!(inference_threads(false, 1), Some(1));
        assert_eq!(inference_threads(true, 24), None);
    }

    #[test]
    fn det_upscale_keeps_the_long_side_within_the_cap() {
        // 679×76 的聊天裁切：短邊放大到 736 的話長邊會有好幾千
        assert_eq!(det_limit_side_len(679, 76), 223);
        assert_eq!(det_limit_side_len(60, 625), 192);
        assert_eq!(det_limit_side_len(376, 373), 736);
        assert_eq!(det_limit_side_len(0, 0), 0);
    }
}
