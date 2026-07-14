//! Firecrawl API 客户端
//!
//! 通过 [Firecrawl SDK](https://firecrawl.dev) 进行云端网页抓取，支持 JS 渲染。
//! 适用于 SPA 站点、需要 JavaScript 执行的页面。
//!
//! # 示例
//!
//! ```rust,no_run
//! use crawlkit_extensions::firecrawl::FirecrawlClient;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let client = FirecrawlClient::builder()
//!     .with_tokens(&["fc-api-token-1", "fc-api-token-2"])
//!     .build()?;
//!
//! let response = client.get("https://example.com", &Default::default()).await?;
//! println!("{}", response.body);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;

use async_trait::async_trait;
use backon::{ExponentialBuilder, Retryable};
use crawlkit_core::{CrawlError, HttpClient, Response};
use rand::Rng;

/// Firecrawl API 客户端
///
/// 支持多 token 轮换，适合免费套餐多账号场景。
pub struct FirecrawlClient {
    name: String,
    api_tokens: Vec<String>,
    max_retries: usize,
}

impl FirecrawlClient {
    /// 创建 Builder
    pub fn builder() -> FirecrawlClientBuilder {
        FirecrawlClientBuilder::default()
    }
}

/// FirecrawlClient 构建器
pub struct FirecrawlClientBuilder {
    name: String,
    api_tokens: Vec<String>,
    max_retries: usize,
}

impl Default for FirecrawlClientBuilder {
    fn default() -> Self {
        Self {
            name: "firecrawl".to_string(),
            api_tokens: Vec::new(),
            max_retries: 3,
        }
    }
}

impl FirecrawlClientBuilder {
    /// 设置客户端名称（用于日志和 CompositeFetcher 识别，默认 "firecrawl"）
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// 设置 API token 列表（每次请求随机选择一个）
    pub fn with_tokens(mut self, tokens: &[impl Into<String> + Clone]) -> Self {
        self.api_tokens = tokens.iter().map(|t| t.clone().into()).collect();
        self
    }

    /// 从环境变量读取 token（逗号分隔多个）
    pub fn with_tokens_from_env(mut self, env_var: &str) -> Self {
        if let Ok(val) = std::env::var(env_var) {
            self.api_tokens = val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
        self
    }

    /// 设置最大重试次数（默认 3）
    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// 构建 FirecrawlClient
    ///
    /// # 错误
    ///
    /// 如果未设置任何 token，返回错误。
    pub fn build(self) -> anyhow::Result<FirecrawlClient> {
        if self.api_tokens.is_empty() {
            anyhow::bail!("Firecrawl 至少需要一个 API Token");
        }
        Ok(FirecrawlClient {
            name: self.name,
            api_tokens: self.api_tokens,
            max_retries: self.max_retries,
        })
    }
}

#[async_trait]
impl HttpClient for FirecrawlClient {
    async fn get(&self, url: &str, _headers: &HashMap<String, String>) -> crawlkit_core::Result<Response> {
        let tokens = self.api_tokens.clone();
        let max_retries = self.max_retries;

        let fetch = || {
            let tokens = tokens.clone();
            async move {
                let token = {
                    let mut rng = rand::rng();
                    let index = rng.random_range(0..tokens.len());
                    tokens[index].clone()
                };

                let client = firecrawl2::Client::new(&token)
                    .map_err(|e: firecrawl2::error::FirecrawlError| anyhow::anyhow!("Firecrawl 客户端创建失败: {e}"))?;

                let options = firecrawl2::ScrapeOptions {
                    formats: Some(vec![firecrawl2::Format::Html]),
                    only_main_content: Some(true),
                    ..Default::default()
                };

                let doc = client.scrape(url, options).await
                    .map_err(|e: firecrawl2::error::FirecrawlError| anyhow::anyhow!("Firecrawl 请求失败: {e}"))?;

                let html = doc.html
                    .ok_or_else(|| anyhow::anyhow!("Firecrawl 返回 HTML 内容为空"))?;

                Ok::<(String, String), anyhow::Error>((html, url.to_string()))
            }
        };

        let (body, final_url) = fetch
            .retry(&ExponentialBuilder::default().with_max_times(max_retries))
            .await
            .map_err(|e| CrawlError::Http(format!("Firecrawl 请求失败（重试 {max_retries} 次）: {e}")))?;

        Ok(Response {
            url: final_url,
            status: 200,
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
