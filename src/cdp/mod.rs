//! CDP (Chrome DevTools Protocol) 客户端
//!
//! 通过 chromiumoxide 连接到已运行的 CDP 服务器（Lightpanda / Obscura / Chrome 等），
//! 导航页面并提取 HTML。适用于需要 JavaScript 渲染的 SPA 站点。
//!
//! # 支持的 CDP 服务
//!
//! | 服务 | 启动方式 |
//! |------|---------|
//! | [Lightpanda](https://github.com/lightpanda-io/browser) | `lightpanda serve --port 9222` |
//! | [Obscura](https://github.com/h4ckf0r0day/obscura) | `obscura serve --port 9222` |
//! | Chrome/Chromium | `chrome --remote-debugging-port=9222` |
//!
//! # 示例
//!
//! ```rust,no_run
//! use crawlkit_extensions::cdp::{CdpClient, CdpPool, CdpStrategy};
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! // 单个客户端
//! let client = CdpClient::builder()
//!     .with_endpoint("http://127.0.0.1:9222")
//!     .build()
//!     .await?;
//!
//! // 多端点池（轮询策略）
//! let pool = CdpPool::builder()
//!     .with_endpoint("http://127.0.0.1:9222")
//!     .with_endpoint("http://127.0.0.1:9223")
//!     .with_strategy(CdpStrategy::RoundRobin)
//!     .build()
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chromiumoxide::browser::Browser;
use crawlkit_core::{CrawlError, HttpClient, Response};
use futures::StreamExt;
use tokio::sync::RwLock;

/// 端点选择策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdpStrategy {
    /// 随机选择一个可用端点
    Random,
    /// 轮询（Round-Robin），依次使用每个端点
    RoundRobin,
    /// 故障转移（Failover），按优先级顺序，失败后切到下一个
    Failover,
}

impl Default for CdpStrategy {
    fn default() -> Self {
        Self::RoundRobin
    }
}

/// 单个 CDP 端点状态（池内部使用）
struct CdpEndpoint {
    client: CdpClient,
    name: String,
    healthy: bool,
}

// ============================================================
// CdpClient（单端点）
// ============================================================

/// CDP 客户端构建器
pub struct CdpClientBuilder {
    endpoint: String,
    name: Option<String>,
    navigation_timeout: Duration,
}

impl Default for CdpClientBuilder {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:9222".to_string(),
            name: None,
            navigation_timeout: Duration::from_secs(30),
        }
    }
}

impl CdpClientBuilder {
    /// 设置 CDP 服务端点
    ///
    /// 支持 HTTP（`http://host:port`）或 WebSocket（`ws://host:port`）格式。
    /// HTTP 端点会自动从 `/json/version` 获取 WebSocket URL。
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// 设置端点名称（用于日志和调试）
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// 设置导航超时（默认 30 秒）
    pub fn with_navigation_timeout(mut self, timeout: Duration) -> Self {
        self.navigation_timeout = timeout;
        self
    }

    /// 构建 CdpClient
    ///
    /// 连接到 CDP 服务器。如果服务器未运行，返回错误。
    pub async fn build(self) -> anyhow::Result<CdpClient> {
        let (browser, mut handler) = Browser::connect(&self.endpoint)
            .await
            .map_err(|e| anyhow::anyhow!("CDP 连接失败 ({}): {e}", self.endpoint))?;

        tokio::spawn(async move {
            while let Some(h) = handler.next().await {
                if h.is_err() {
                    break;
                }
            }
        });

        Ok(CdpClient {
            browser,
            navigation_timeout: self.navigation_timeout,
            name: self.name.unwrap_or(self.endpoint),
        })
    }
}

/// CDP 客户端（单端点）
///
/// 通过 chromiumoxide 连接到 CDP 服务器，实现 `HttpClient` trait。
pub struct CdpClient {
    browser: Browser,
    navigation_timeout: Duration,
    name: String,
}

impl CdpClient {
    /// 创建 Builder
    pub fn builder() -> CdpClientBuilder {
        CdpClientBuilder::default()
    }

    /// 获取内部 Browser 引用（用于高级操作）
    pub fn browser(&self) -> &Browser {
        &self.browser
    }

    /// 获取端点名称
    pub fn endpoint_name(&self) -> &str {
        &self.name
    }
}

#[async_trait]
impl HttpClient for CdpClient {
    async fn get(
        &self,
        url: &str,
        _headers: &HashMap<String, String>,
    ) -> crawlkit_core::Result<Response> {
        let page = self
            .browser
            .new_page(url)
            .await
            .map_err(|e| CrawlError::Http(format!("CDP 创建页面失败: {e}")))?;

        match tokio::time::timeout(self.navigation_timeout, page.wait_for_navigation()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                eprintln!("[CDP:{}] 等待导航警告: {e}", self.name);
            }
            Err(_) => {
                eprintln!(
                    "[CDP:{}] 导航超时 ({:?})，继续提取",
                    self.name, self.navigation_timeout
                );
            }
        }

        let html = page
            .content()
            .await
            .map_err(|e| CrawlError::Http(format!("CDP 提取 HTML 失败: {e}")))?;

        let final_url = page
            .url()
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| url.to_string());

        let _ = page.close().await;

        Ok(Response {
            url: final_url,
            status: 200,
            headers: Default::default(),
            body: html,
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

// ============================================================
// CdpPool（多端点池）
// ============================================================

/// CDP 连接池构建器
pub struct CdpPoolBuilder {
    endpoints: Vec<(String, Option<String>)>,
    strategy: CdpStrategy,
    navigation_timeout: Duration,
}

impl Default for CdpPoolBuilder {
    fn default() -> Self {
        Self {
            endpoints: Vec::new(),
            strategy: CdpStrategy::default(),
            navigation_timeout: Duration::from_secs(30),
        }
    }
}

impl CdpPoolBuilder {
    /// 添加一个 CDP 端点
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoints.push((endpoint.into(), None));
        self
    }

    /// 添加一个带名称的 CDP 端点
    pub fn with_named_endpoint(
        mut self,
        name: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        self.endpoints.push((endpoint.into(), Some(name.into())));
        self
    }

    /// 设置端点选择策略（默认 RoundRobin）
    pub fn with_strategy(mut self, strategy: CdpStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// 设置导航超时（默认 30 秒，应用到所有端点）
    pub fn with_navigation_timeout(mut self, timeout: Duration) -> Self {
        self.navigation_timeout = timeout;
        self
    }

    /// 构建 CdpPool
    ///
    /// 依次连接所有端点。至少需要一个端点连接成功，否则返回错误。
    /// 连接失败的端点会被标记为不健康，后续请求会跳过。
    pub async fn build(self) -> anyhow::Result<CdpPool> {
        if self.endpoints.is_empty() {
            anyhow::bail!("CdpPool 至少需要一个 CDP 端点");
        }

        let mut endpoints = Vec::new();
        let mut last_error = None;

        for (url, name) in &self.endpoints {
            let ep_name = name.clone().unwrap_or_else(|| url.clone());
            match CdpClient::builder()
                .with_endpoint(url)
                .with_name(&ep_name)
                .with_navigation_timeout(self.navigation_timeout)
                .build()
                .await
            {
                Ok(client) => {
                    endpoints.push(CdpEndpoint {
                        client,
                        name: ep_name,
                        healthy: true,
                    });
                }
                Err(e) => {
                    eprintln!("[CdpPool] 端点 {ep_name} ({url}) 连接失败: {e}，标记为不健康");
                    last_error = Some(e);
                }
            }
        }

        if endpoints.is_empty() {
            anyhow::bail!(
                "CdpPool 所有端点均连接失败，最后错误: {}",
                last_error.map(|e| e.to_string()).unwrap_or_else(|| "未知".into())
            );
        }

        Ok(CdpPool {
            endpoints: Arc::new(RwLock::new(endpoints)),
            strategy: self.strategy,
            counter: AtomicUsize::new(0),
        })
    }
}

/// CDP 连接池
///
/// 管理多个 CDP 端点，支持随机、轮询、故障转移策略。
/// 实现 `HttpClient` trait，可直接用于 `CompositeFetcher`。
pub struct CdpPool {
    endpoints: Arc<RwLock<Vec<CdpEndpoint>>>,
    strategy: CdpStrategy,
    counter: AtomicUsize,
}

impl CdpPool {
    /// 创建 Builder
    pub fn builder() -> CdpPoolBuilder {
        CdpPoolBuilder::default()
    }

    /// 获取当前健康端点数量
    pub async fn healthy_count(&self) -> usize {
        self.endpoints
            .read()
            .await
            .iter()
            .filter(|e| e.healthy)
            .count()
    }

    /// 获取端点总数
    pub async fn total_count(&self) -> usize {
        self.endpoints.read().await.len()
    }

    /// 选择下一个端点索引（返回时立即释放锁）
    async fn select_index(&self) -> Option<usize> {
        let snapshot: Vec<(usize, bool, String)> = {
            let endpoints = self.endpoints.read().await;
            endpoints
                .iter()
                .enumerate()
                .map(|(i, e)| (i, e.healthy, e.name.clone()))
                .collect()
        };

        let healthy: Vec<(usize, &str)> = snapshot
            .iter()
            .filter(|(_, healthy, _)| *healthy)
            .map(|(i, _, name)| (*i, name.as_str()))
            .collect();

        if healthy.is_empty() {
            // 所有端点不健康，重置并返回第一个
            self.reset_all_health().await;
            return Some(0);
        }

        match self.strategy {
            CdpStrategy::Random => {
                use rand::Rng;
                let idx = rand::rng().random_range(0..healthy.len());
                Some(healthy[idx].0)
            }
            CdpStrategy::RoundRobin => {
                let counter = self.counter.fetch_add(1, Ordering::Relaxed);
                let idx = counter % healthy.len();
                Some(healthy[idx].0)
            }
            CdpStrategy::Failover => Some(healthy[0].0),
        }
    }

    /// 标记端点为不健康
    async fn mark_unhealthy(&self, index: usize) {
        let mut endpoints = self.endpoints.write().await;
        if let Some(ep) = endpoints.get_mut(index) {
            eprintln!("[CdpPool] 端点 {} 标记为不健康", ep.name);
            ep.healthy = false;
        }
    }

    /// 重置所有端点为健康状态
    async fn reset_all_health(&self) {
        let mut endpoints = self.endpoints.write().await;
        for ep in endpoints.iter_mut() {
            ep.healthy = true;
        }
    }
}

#[async_trait]
impl HttpClient for CdpPool {
    async fn get(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
    ) -> crawlkit_core::Result<Response> {
        let total = self.total_count().await;
        let mut last_error = None;

        for _ in 0..total {
            let idx = match self.select_index().await {
                Some(i) => i,
                None => break,
            };

            // 获取端点名称（快速操作，立即释放锁）
            let name = {
                let endpoints = self.endpoints.read().await;
                endpoints[idx].name.clone()
            };

            // 发起请求（不持有锁）
            let result = {
                let endpoints = self.endpoints.read().await;
                endpoints[idx].client.get(url, headers).await
            };

            match result {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    eprintln!("[CdpPool] 端点 {name} 请求失败: {e}");
                    self.mark_unhealthy(idx).await;
                    last_error = Some(e);
                }
            }
        }

        Err(CrawlError::Http(format!(
            "CdpPool: 所有 {total} 个端点均失败，最后错误: {}",
            last_error.unwrap_or_else(|| CrawlError::Http("无可用端点".to_string()))
        )))
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
        "cdp-pool"
    }
}
