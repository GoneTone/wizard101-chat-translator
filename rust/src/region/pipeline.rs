//! 框選區域翻譯的流程：本機 OCR 辨識畫面文字，再交給翻譯後端翻成目標語言。
//!
//! 一律走本機 OCR、不把截圖交給模型看圖，截圖不離開本機。

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::i18n::t;
use crate::logs::log;
use crate::region::ocr::OcrUnavailable;
use crate::translation::errors::{TranslateError, friendly_error};
use crate::translation::translator::Translator;
use crate::ui::geometry::Rect;

/// 辨識函式：PNG bytes → 文字；測試注入替身，正式用 `ocr::recognize`。
pub type Recognize = Arc<dyn Fn(&[u8]) -> Result<String, OcrUnavailable> + Send + Sync>;

/// 一次框選的結果：`text` 是譯文（空字串代表畫面上沒有文字），`source` 是 OCR 原文。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionResult {
    pub text: String,
    pub source: String,
}

/// 流程失敗：OCR 不可用，或翻譯失敗；由 UI 端用 [`describe_error`] 轉成文案。
#[derive(Debug, Clone, PartialEq)]
pub enum RegionError {
    Ocr(OcrUnavailable),
    Translate(TranslateError),
}

impl From<TranslateError> for RegionError {
    fn from(e: TranslateError) -> Self {
        Self::Translate(e)
    }
}

/// 本機 OCR → 文字翻譯。
pub struct RegionPipeline {
    translator: Arc<Translator>,
    recognize: Recognize,
}

impl RegionPipeline {
    pub fn new(translator: Arc<Translator>, recognize: Recognize) -> Self {
        Self {
            translator,
            recognize,
        }
    }

    /// 這條流程用的是哪一組翻譯服務（失敗 log 的診斷欄位）。
    pub fn describe(&self) -> String {
        self.translator.describe()
    }

    /// 辨識並翻譯一張截圖；`rect` 只用來記 log，`cancel` 只作用於翻譯請求。
    ///
    /// OCR 會阻塞，所以丟到 blocking 執行緒，不佔用 async worker。
    pub async fn run(
        &self,
        png: Vec<u8>,
        rect: Rect,
        cancel: &CancellationToken,
    ) -> Result<RegionResult, RegionError> {
        let started = Instant::now();
        let recognize = Arc::clone(&self.recognize);
        let source = tokio::task::spawn_blocking(move || recognize(&png))
            .await
            .map_err(|e| {
                log(format!("[region] OCR task failed: {e}"));
                RegionError::Ocr(OcrUnavailable(format!("OCR task failed: {e}")))
            })?
            .map_err(RegionError::Ocr)?;
        if source.is_empty() {
            log(format!("[region] local OCR found no text (rect={rect:?})"));
            return Ok(RegionResult::default());
        }
        let text = self
            .translator
            .translate_region_text(&source, Some(cancel))
            .await?;
        log(format!(
            "[region] done in {:.1}s (rect={rect:?}, source_chars={}, chars={})",
            started.elapsed().as_secs_f64(),
            source.chars().count(),
            text.chars().count()
        ));
        Ok(RegionResult { text, source })
    }
}

/// 失敗 → 卡片上的一句話（已經 `t()` 過）；翻譯端錯誤沿用設定視窗測試連線那套文案。
pub fn describe_error(e: &RegionError) -> String {
    let (key, args) = match e {
        RegionError::Ocr(_) => return t("region.ocr_unavailable", &[]),
        RegionError::Translate(TranslateError::BadOutput(message)) => {
            ("region.failed", vec![("error", message.clone())])
        }
        RegionError::Translate(e) => friendly_error(e),
    };
    let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
    t(key, &args)
}
