//! Jina Reader API 客户端
//!
//! 将任意网页 URL 通过 Jina Reader API（`r.jina.ai`）转换为干净的 Markdown。
//! 免费额度 1000 次/月，无需 token 即可使用（有速率限制）。
//!
//! # 示例
//!
//! ```rust,no_run
//! use crawlkit_core::HttpClient;
//! use crawlkit_extensions::jina::{JinaClient, JinaFormat};
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let client = JinaClient::builder()
//!     .with_token("your-api-token")
//!     .with_timeout(std::time::Duration::from_secs(30))
//!     .with_format(JinaFormat::Html)
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

/// Jina Reader 输出格式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JinaFormat {
    /// Markdown 格式（默认，干净的文本内容）
    #[default]
    Markdown,
    /// HTML 格式（完整 HTML，可用于 on_html 回调）
    Html,
    /// 纯文本格式
    Text,
}

impl JinaFormat {
    fn accept_header(&self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown",
            Self::Html => "text/html",
            Self::Text => "text/plain",
        }
    }
}

/// Jina Reader API 客户端
///
/// 通过 Builder 模式配置，使用 [`JinaClient::builder`] 创建。
pub struct JinaClient {
    name: String,
    api_token: Option<String>,
    client: reqwest::Client,
    max_retries: usize,
    format: JinaFormat,
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
            name: builder.name,
            api_token: builder.api_token,
            client,
            max_retries: builder.max_retries,
            format: builder.format,
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
    name: String,
    api_token: Option<String>,
    timeout: Duration,
    max_retries: usize,
    format: JinaFormat,
}

impl Default for JinaClientBuilder {
    fn default() -> Self {
        Self {
            name: "jina".to_string(),
            api_token: None,
            timeout: Duration::from_secs(60),
            max_retries: 3,
            format: JinaFormat::default(),
        }
    }
}

impl JinaClientBuilder {
    /// 设置客户端名称（用于日志和 CompositeFetcher 识别，默认 "jina"）
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

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

    /// 设置输出格式（默认 Markdown）
    ///
    /// - `JinaFormat::Markdown`：干净的 Markdown（适合文本处理）
    /// - `JinaFormat::Html`：完整 HTML（可用于 `on_html` 回调）
    /// - `JinaFormat::Text`：纯文本
    pub fn with_format(mut self, format: JinaFormat) -> Self {
        self.format = format;
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
        let accept = self.format.accept_header().to_string();

        let fetch = || {
            let client = client.clone();
            let reader_url = reader_url.clone();
            let token = token.clone();
            let accept = accept.clone();
            async move {
                let mut req = client
                    .get(&reader_url)
                    .header("Accept", &accept);

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
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jina_format_default() {
        assert_eq!(JinaFormat::default(), JinaFormat::Markdown);
    }

    #[test]
    fn test_jina_format_accept_header() {
        assert_eq!(JinaFormat::Markdown.accept_header(), "text/markdown");
        assert_eq!(JinaFormat::Html.accept_header(), "text/html");
        assert_eq!(JinaFormat::Text.accept_header(), "text/plain");
    }

    #[test]
    fn test_jina_builder_defaults() {
        let client = JinaClient::builder().build();
        assert_eq!(client.name(), "jina");
        assert!(client.api_token.is_none());
    }

    #[test]
    fn test_jina_builder_with_token() {
        let client = JinaClient::builder()
            .with_token("test-token")
            .build();
        assert_eq!(client.api_token.as_deref(), Some("test-token"));
    }

    #[test]
    fn test_jina_builder_with_name() {
        let client = JinaClient::builder()
            .with_name("custom")
            .build();
        assert_eq!(client.name(), "custom");
    }
}
