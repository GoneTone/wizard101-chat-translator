//! 本地服務：向本地模型 hub 要 llama-server 的位址，再轉給 OpenAI 相容 client。

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use super::{ChatRequest, ModelInfo, OpenAiCompatClient, cancellable};
use crate::config::services::EFFORT_AUTO;
use crate::local;
use crate::logs::log;
use crate::translation::errors::TranslateError;
use crate::util::lock;

/// 本地服務的 client：每次請求前確保模型的 llama-server 在跑。
pub struct LocalClient {
    model_id: String,
    thinking: bool,
    /// 上次的 base URL 與對應的 client；子程序換埠重啟後才重建。
    inner: Mutex<Option<(String, Arc<OpenAiCompatClient>)>>,
}

impl LocalClient {
    pub fn new(model_id: &str, thinking: bool) -> Self {
        Self {
            model_id: model_id.to_owned(),
            thinking,
            inner: Mutex::default(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model_id
    }

    /// 確保 llama-server 在跑後轉送請求；`cancel` 觸發時不等模型載入完就回 `Cancelled`。
    pub async fn chat(
        &self,
        req: ChatRequest<'_>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, TranslateError> {
        let local = local::get().ok_or_else(|| config_error("local runtime unavailable"))?;
        let id = self.model_id.clone();
        // 另開 task：取消翻譯不該連帶中斷載入到一半的模型
        let ensure = tokio::spawn(async move { local.ensure(&id).await });
        let url = cancellable(cancel, async {
            ensure
                .await
                .map_err(|e| TranslateError::Unexpected(e.to_string()))?
                .map_err(|e| config_error(&e))
        })
        .await?;
        self.client_for(url).chat(req, cancel).await
    }

    /// 已下載完成的模型 id；hub 不在時為空。
    pub fn list_models(&self) -> Vec<ModelInfo> {
        local::get().map_or_else(Vec::new, |l| {
            l.snapshot()
                .models
                .into_iter()
                .filter(|m| m.complete)
                .map(|m| ModelInfo::bare(m.id))
                .collect()
        })
    }

    fn client_for(&self, url: String) -> Arc<OpenAiCompatClient> {
        let mut inner = lock(&self.inner);
        if let Some((cached, client)) = &*inner
            && *cached == url
        {
            return Arc::clone(client);
        }
        log(format!("[local] {} served at {url}", self.model_id));
        let client = Arc::new(OpenAiCompatClient::new(
            &url,
            "",
            &self.model_id,
            self.thinking,
            EFFORT_AUTO,
            false,
        ));
        *inner = Some((url, Arc::clone(&client)));
        client
    }
}

fn config_error(detail: &str) -> TranslateError {
    TranslateError::Config {
        status: None,
        detail: detail.to_owned(),
    }
}
