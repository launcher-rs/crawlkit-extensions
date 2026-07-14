//! Jina Reader API 客户端
//!
//! 将任意网页 URL 通过 Jina Reader API（`r.jina.ai`）转换为干净的 Markdown。
//! 免费额度 1000 次/月，无需 token 即可使用（有速率限制）。
//!
//! # 示例
//!
//! ```rust,no_run
//! use crawlkit_extensions::jina::JinaClient;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let client = JinaClient::builder()
//!     .with_token("your-api-token")
//!     .with_timeout(std::time::Duration::from_secs(30))
//!     .build();
//!
//! let response = client.get("https://example.com", &Default::default()).await?;
//! println!("{}", response.body);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use backon::{ExponentialBuilder, Retryable};
use crawlkit_core::{CrawlError, HttpClient, Response};

/// Jina Reader API 客户端
///
/// 通过 Builder 模式配置，使用 [`JinaClient::builder`] 创建。
pub struct JinaClient {
    api_token: Option<String>,
    client: reqwest::Client,
    max_retries: usize,
}

impl JinaClient {
    /// 创建 Builder，用于灵活配置客户端参数
    pub fn builder() -> JinaClientBuilder {
        JinaClientBuilder::default()
    }

    /// 从 builder 构建最终客户端
    fn from_builder(builder: JinaClientBuilder) -> Self {
        let client = reqwest::Client::builder()
            .timeout(builder.timeout)
            .build()
            .expect("failed to build reqwest client");

        Self {
            api_token: builder.api_token,
            client,
            max_retries: builder.max_retries,
        }
    }
}

impl Default for JinaClient {
    fn default() -> Self {
        Self::builder().build()
    }
}

/// JinaClient 构建器
pub struct JinaClientBuilder {
    api_token: Option<String>,
    timeout: Duration,
    max_retries: usize,
}

impl Default for JinaClientBuilder {
    fn default() -> Self {
        Self {
            api_token: None,
            timeout: Duration::from_secs(60),
            max_retries: 3,
        }
    }
}

impl JinaClientBuilder {
    /// 设置 API token（可选，免费额度无需 token）
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.api_token = Some(token.into());
        self
    }

    /// 设置 API token（从环境变量读取）
    pub fn with_token_from_env(mut self, env_var: &str) -> Self {
        self.api_token = std::env::var(env_var).ok().filter(|s| !s.is_empty());
        self
    }

    /// 设置请求超时
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 设置最大重试次数（默认 3）
    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// 构建 JinaClient
    pub fn build(self) -> JinaClient {
        JinaClient::from_builder(self)
    }
}

#[async_trait]
impl HttpClient for JinaClient {
    async fn get(&self, url: &str, _headers: &HashMap<String, String>) -> crawlkit_core::Result<Response> {
        let client = self.client.clone();
        let reader_url = format!("https://r.jina.ai/{url}");
        let token = self.api_token.clone();
        let max_retries = self.max_retries;

        let fetch = || {
            let client = client.clone();
            let reader_url = reader_url.clone();
            let token = token.clone();
            async move {
                let mut req = client
                    .get(&reader_url)
                    .header("Accept", "text/markdown");

                if let Some(ref t) = token {
                    req = req.header("Authorization", format!("Bearer {t}"));
                }

                let resp = req.send().await.map_err(|e| {
                    anyhow::anyhow!("Jina 请求失败: {e}")
                })?;

                let status = resp.status().as_u16();
                let final_url = resp.url().to_string();
                let body = resp.text().await.map_err(|e| {
                    anyhow::anyhow!("Jina 读取响应失败: {e}")
                })?;

                if status >= 400 {
                    return Err(anyhow::anyhow!("Jina 返回错误状态 {status}: {body}"));
                }

                Ok((final_url, status, body))
            }
        };

        let (final_url, status, body) = fetch
            .retry(&ExponentialBuilder::default().with_max_times(max_retries))
            .await
            .map_err(|e| CrawlError::Http(format!("Jina 请求失败（重试 {max_retries} 次）: {e}")))?;

        Ok(Response {
            url: final_url,
            status,
            headers: Default::default(),
            body,
        })
    }

    async fn post(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        _body: Vec<u8>,
    ) -> crawlkit_core::Result<Response> {
        self.get(url, headers).await
    }

    fn name(&self) -> &str {
        "jina"
    }
}
