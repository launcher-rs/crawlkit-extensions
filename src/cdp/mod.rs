//! CDP (Chrome DevTools Protocol) 客户端
//!
//! 通过原生 WebSocket 直接连接 CDP 服务器（Lightpanda / Obscura / Chrome 等），
//! 导航页面并提取 HTML。一个客户端适用于所有 CDP 实现。
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
//! // 多端点池
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
use async_tungstenite::tokio::connect_async;
use async_tungstenite::tungstenite::Message as WsMessage;
use crawlkit_core::{CrawlError, HttpClient, Response};
use futures::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio::time::timeout;

// ============================================================
// 工具函数
// ============================================================

/// 获取浏览器级 WebSocket URL
async fn resolve_browser_ws(endpoint: &str) -> anyhow::Result<String> {
    let http_url = if endpoint.starts_with("ws") {
        endpoint.replacen("ws", "http", 1).to_string()
    } else {
        endpoint.to_string()
    };
    let version_url = format!("{}/json/version", http_url.trim_end_matches('/'));
    let resp = reqwest::get(&version_url)
        .await
        .map_err(|e| anyhow::anyhow!("HTTP 请求失败 ({}): {}", version_url, e))?;
    let info: Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("解析 /json/version 失败: {}", e))?;
    info["webSocketDebuggerUrl"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("响应缺少 webSocketDebuggerUrl"))
}

/// 发送 CDP 命令并等待响应
async fn cdp_call(
    write: &mut (impl SinkExt<WsMessage> + Unpin),
    read: &mut (impl StreamExt<Item = Result<WsMessage, async_tungstenite::tungstenite::Error>> + Unpin),
    id: u32,
    session_id: Option<&str>,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    let mut cmd = json!({"id": id, "method": method, "params": params});
    if let Some(sid) = session_id {
        cmd["sessionId"] = json!(sid);
    }
    let msg = serde_json::to_string(&cmd)?;
    write
        .send(WsMessage::Text(msg.into()))
        .await
        .map_err(|_| anyhow::anyhow!("发送 CDP 命令 {} 失败", method))?;

    while let Some(msg_result) = read.next().await {
        let msg = msg_result.map_err(|e| anyhow::anyhow!("WebSocket 错误: {:?}", e))?;
        match msg {
            WsMessage::Text(text) => {
                if let Ok(val) = serde_json::from_str::<Value>(&text) {
                    if val.get("id").and_then(|v| v.as_u64()) == Some(id as u64) {
                        if let Some(err) = val.get("error") {
                            anyhow::bail!("CDP 错误 [{}]: {}", method, err);
                        }
                        return Ok(val["result"].clone());
                    }
                }
            }
            WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Binary(_) | WsMessage::Frame(_) => {}
            WsMessage::Close(_) => anyhow::bail!("WebSocket 已关闭"),
        }
    }
    anyhow::bail!("WebSocket 连接已关闭")
}

/// 等待页面加载事件
async fn wait_page_load(
    read: &mut (impl StreamExt<Item = Result<WsMessage, async_tungstenite::tungstenite::Error>> + Unpin),
    session_id: Option<&str>,
    timeout_dur: Duration,
) -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout_dur {
        let remaining = timeout_dur.saturating_sub(start.elapsed());
        match tokio::time::timeout(remaining, read.next()).await {
            Ok(Some(Ok(msg))) => match msg {
                WsMessage::Text(text) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                        if let Some(sid) = session_id {
                            if val.get("sessionId").and_then(|v| v.as_str()) != Some(sid) {
                                continue;
                            }
                        }
                        if let Some(method) = val.get("method").and_then(|v| v.as_str()) {
                            match method {
                                "Page.frameStoppedLoading" => return Ok(()),
                                "Page.lifecycleEvent" if val["params"]["name"] == "load" => return Ok(()),
                                _ => {}
                            }
                        }
                    }
                }
                WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Binary(_) | WsMessage::Frame(_) => {}
                WsMessage::Close(_) => anyhow::bail!("WebSocket 已关闭"),
            },
            Ok(Some(Err(e))) => anyhow::bail!("WebSocket 错误: {:?}", e),
            Ok(None) => anyhow::bail!("WebSocket 已关闭"),
            Err(_) => anyhow::bail!("等待页面加载超时 ({:?})", timeout_dur),
        }
    }
    anyhow::bail!("等待页面加载超时 ({:?})", timeout_dur)
}

// ============================================================
// CdpClient（单端点）
// ============================================================

/// CDP 客户端构建器
pub struct CdpClientBuilder {
    endpoint: String,
    name: Option<String>,
    navigation_timeout: Duration,
    connection_timeout: Duration,
}

impl Default for CdpClientBuilder {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:9222".to_string(),
            name: None,
            navigation_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(10),
        }
    }
}

impl CdpClientBuilder {
    /// 设置 CDP 服务端点，支持 `http://host:port` 或 `ws://host:port/devtools/browser` 格式
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

    /// 设置连接超时（默认 10 秒）
    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = timeout;
        self
    }

    /// 构建 CdpClient
    pub async fn build(self) -> anyhow::Result<CdpClient> {
        let name = self.name.unwrap_or_else(|| self.endpoint.clone());
        let browser_ws_url = resolve_browser_ws(&self.endpoint).await?;

        // 验证连接：创建并销毁一个测试目标
        let (ws, _) = timeout(self.connection_timeout, connect_async(&browser_ws_url))
            .await
            .map_err(|_| anyhow::anyhow!("连接超时"))?
            .map_err(|e| anyhow::anyhow!("连接失败: {}", e))?;
        let (mut w, mut r) = ws.split();
        let r = cdp_call(&mut w, &mut r, 1, None, "Target.createTarget", json!({"url": "about:blank"})).await;
        let _ = w.close(None).await;
        r.map_err(|e| anyhow::anyhow!("CDP 验证失败 ({}): {}", self.endpoint, e))?;

        Ok(CdpClient {
            browser_ws_url,
            navigation_timeout: self.navigation_timeout,
            connection_timeout: self.connection_timeout,
            name,
        })
    }
}

/// CDP 客户端
///
/// 通过原生 WebSocket 直接连接 CDP 服务器，支持所有标准 CDP 实现
/// （Chrome、Lightpanda、Obscura 等）。
pub struct CdpClient {
    browser_ws_url: String,
    navigation_timeout: Duration,
    connection_timeout: Duration,
    name: String,
}

impl CdpClient {
    /// 创建 Builder
    pub fn builder() -> CdpClientBuilder {
        CdpClientBuilder::default()
    }

    /// 获取端点名称
    pub fn endpoint_name(&self) -> &str {
        &self.name
    }
}

async fn fetch_page(
    browser_ws_url: &str,
    target_url: &str,
    nav_timeout: Duration,
    conn_timeout: Duration,
) -> anyhow::Result<(String, String)> {
    let (ws, _) = timeout(conn_timeout, connect_async(browser_ws_url))
        .await
        .map_err(|_| anyhow::anyhow!("连接超时"))?
        .map_err(|e| anyhow::anyhow!("连接失败: {}", e))?;

    let (mut write, mut read) = ws.split();
    let mut id = 0u32;

    // 1. 创建目标
    id += 1;
    let create = cdp_call(&mut write, &mut read, id, None, "Target.createTarget", json!({"url": "about:blank"})).await?;
    let target_id = create["targetId"].as_str().ok_or_else(|| anyhow::anyhow!("缺少 targetId"))?;

    // 2. 附加到目标
    id += 1;
    let attach = cdp_call(&mut write, &mut read, id, None, "Target.attachToTarget", json!({"targetId": target_id, "flatten": true})).await?;
    let sid = attach["sessionId"].as_str().ok_or_else(|| anyhow::anyhow!("缺少 sessionId"))?;

    // 3. 启用运行时并恢复执行
    id += 1;
    let _ = cdp_call(&mut write, &mut read, id, Some(sid), "Runtime.enable", json!({})).await;
    id += 1;
    let _ = cdp_call(&mut write, &mut read, id, Some(sid), "Runtime.runIfWaitingForDebugger", json!({})).await;

    // 4. 启用页面事件
    id += 1;
    let _ = cdp_call(&mut write, &mut read, id, Some(sid), "Page.enable", json!({})).await;

    // 5. 导航
    id += 1;
    cdp_call(&mut write, &mut read, id, Some(sid), "Page.navigate", json!({"url": target_url})).await?;

    // 6. 等待页面加载
    wait_page_load(&mut read, Some(sid), nav_timeout).await?;

    // 7. 获取 HTML
    id += 1;
    let html = cdp_call(
        &mut write, &mut read, id, Some(sid),
        "Runtime.evaluate",
        json!({"expression": "document.documentElement.outerHTML", "returnByValue": true}),
    ).await?;
    let body = html["result"]["value"].as_str().unwrap_or("").to_string();

    // 8. 获取最终 URL
    id += 1;
    let url = cdp_call(
        &mut write, &mut read, id, Some(sid),
        "Runtime.evaluate",
        json!({"expression": "document.URL", "returnByValue": true}),
    ).await?;
    let final_url = url["result"]["value"].as_str().unwrap_or(target_url).to_string();

    let _ = write.close(None).await;
    Ok((final_url, body))
}

#[async_trait]
impl HttpClient for CdpClient {
    async fn get(&self, url: &str, _headers: &HashMap<String, String>) -> crawlkit_core::Result<Response> {
        let (final_url, body) = fetch_page(&self.browser_ws_url, url, self.navigation_timeout, self.connection_timeout)
            .await
            .map_err(|e| CrawlError::Http(format!("CDP 获取失败: {}", e)))?;
        Ok(Response { url: final_url, status: 200, headers: Default::default(), body })
    }

    async fn post(&self, url: &str, headers: &HashMap<String, String>, _body: Vec<u8>) -> crawlkit_core::Result<Response> {
        self.get(url, headers).await
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ============================================================
// CdpPool（多端点池）
// ============================================================

/// 端点选择策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CdpStrategy {
    /// 随机选择一个可用端点
    Random,
    /// 轮询（Round-Robin），依次使用每个端点
    #[default]
    RoundRobin,
    /// 故障转移（Failover），按优先级顺序，失败后切到下一个
    Failover,
}

/// 单个 CDP 端点状态（池内部使用）
struct CdpEndpoint {
    client: CdpClient,
    name: String,
    healthy: bool,
}

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
    pub fn with_named_endpoint(mut self, name: impl Into<String>, endpoint: impl Into<String>) -> Self {
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
                    endpoints.push(CdpEndpoint { client, name: ep_name, healthy: true });
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
        self.endpoints.read().await.iter().filter(|e| e.healthy).count()
    }

    /// 获取端点总数
    pub async fn total_count(&self) -> usize {
        self.endpoints.read().await.len()
    }

    async fn select_index(&self) -> Option<usize> {
        let snapshot: Vec<(usize, bool, String)> = {
            let endpoints = self.endpoints.read().await;
            endpoints.iter().enumerate().map(|(i, e)| (i, e.healthy, e.name.clone())).collect()
        };

        let healthy: Vec<(usize, &str)> = snapshot
            .iter()
            .filter(|(_, healthy, _)| *healthy)
            .map(|(i, _, name)| (*i, name.as_str()))
            .collect();

        if healthy.is_empty() {
            self.reset_all_health().await;
            return Some(0);
        }

        match self.strategy {
            CdpStrategy::Random => {
                let idx = rand::rng().random_range(0..healthy.len());
                Some(healthy[idx].0)
            }
            CdpStrategy::RoundRobin => {
                let counter = self.counter.fetch_add(1, Ordering::Relaxed);
                Some(healthy[counter % healthy.len()].0)
            }
            CdpStrategy::Failover => Some(healthy[0].0),
        }
    }

    async fn mark_unhealthy(&self, index: usize) {
        let mut endpoints = self.endpoints.write().await;
        if let Some(ep) = endpoints.get_mut(index) {
            eprintln!("[CdpPool] 端点 {} 标记为不健康", ep.name);
            ep.healthy = false;
        }
    }

    async fn reset_all_health(&self) {
        let mut endpoints = self.endpoints.write().await;
        for ep in endpoints.iter_mut() {
            ep.healthy = true;
        }
    }
}

#[async_trait]
impl HttpClient for CdpPool {
    async fn get(&self, url: &str, headers: &HashMap<String, String>) -> crawlkit_core::Result<Response> {
        let total = self.total_count().await;
        let mut last_error = None;

        for _ in 0..total {
            let idx = match self.select_index().await {
                Some(i) => i,
                None => break,
            };
            let name = { self.endpoints.read().await[idx].name.clone() };
            let result = { self.endpoints.read().await[idx].client.get(url, headers).await };

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

    async fn post(&self, url: &str, headers: &HashMap<String, String>, _body: Vec<u8>) -> crawlkit_core::Result<Response> {
        self.get(url, headers).await
    }

    fn name(&self) -> &str {
        "cdp-pool"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cdp_strategy_default() {
        assert_eq!(CdpStrategy::default(), CdpStrategy::RoundRobin);
    }

    #[test]
    fn test_cdp_builder_defaults() {
        let builder = CdpClientBuilder::default();
        assert_eq!(builder.endpoint, "http://127.0.0.1:9222");
        assert!(builder.name.is_none());
        assert_eq!(builder.navigation_timeout, Duration::from_secs(30));
        assert_eq!(builder.connection_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_cdp_pool_builder_defaults() {
        let builder = CdpPoolBuilder::default();
        assert!(builder.endpoints.is_empty());
        assert_eq!(builder.strategy, CdpStrategy::RoundRobin);
        assert_eq!(builder.navigation_timeout, Duration::from_secs(30));
    }
}
