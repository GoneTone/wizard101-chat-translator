//! 本地服務：向本地模型 hub 要 llama-server 的位址，再轉給 OpenAI 相容 client。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{ChatRequest, ModelInfo, OpenAiCompatClient, cancellable};
use crate::config::services::EFFORT_AUTO;
use crate::local::{self, hf::hf_client};
use crate::logs::log;
use crate::net::error_chain;
use crate::translation::errors::TranslateError;
use crate::util::lock;

/// 查 `/props` 的上限：本機請求，慢了就當沒睡、直接送翻譯。
const PROPS_TIMEOUT: Duration = Duration::from_secs(2);

/// 本地服務的 client：每次請求前確保模型的 llama-server 在跑。
pub struct LocalClient {
    model_id: String,
    thinking: bool,
    /// 上次的 base URL 與對應的 client；子程序換埠重啟後才重建。
    inner: Mutex<Option<(String, Arc<OpenAiCompatClient>)>>,
    /// 查 llama-server 狀態用（只連本機）。
    http: reqwest::Client,
}

impl LocalClient {
    pub fn new(model_id: &str, thinking: bool) -> Self {
        Self {
            model_id: model_id.to_owned(),
            thinking,
            inner: Mutex::default(),
            http: hf_client("http://127.0.0.1"),
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
        let (hub, id) = (Arc::clone(&local), self.model_id.clone());
        // 另開 task：取消翻譯不該連帶中斷載入到一半的模型
        let ensure = tokio::spawn(async move { hub.ensure(&id).await });
        let url = cancellable(cancel, async {
            ensure
                .await
                .map_err(|e| TranslateError::Unexpected(e.to_string()))?
                .map_err(|e| config_error(&e))
        })
        .await?;
        // 閒置睡著的模型會在這個請求重新載入：期間顯示載入橫幅，譯文回來（或失敗）才收起
        let _banner = self
            .sleeping(&url)
            .await
            .then(|| local.waking(&self.model_id));
        self.client_for(url).chat(req, cancel).await
    }

    /// llama-server 是否因閒置睡著；查不到時當作沒睡，不擋翻譯。
    async fn sleeping(&self, url: &str) -> bool {
        let props = async {
            let response = self
                .http
                .get(format!("{url}/props"))
                .timeout(PROPS_TIMEOUT)
                .send()
                .await
                .map_err(error_chain)?;
            if !response.status().is_success() {
                return Err(format!("HTTP {}", response.status().as_u16()));
            }
            let props: Value = response.json().await.map_err(error_chain)?;
            Ok(props["is_sleeping"].as_bool().unwrap_or(false))
        };
        props.await.unwrap_or_else(|e: String| {
            log(format!(
                "[local] {} /props check failed, sending anyway: {e}",
                self.model_id
            ));
            false
        })
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
        let client = Arc::new(
            OpenAiCompatClient::new(&url, "", &self.model_id, self.thinking, EFFORT_AUTO, false)
                .with_usage(),
        );
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
