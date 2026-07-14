//! CDP 无头浏览器示例
//!
//! 演示如何使用 CdpClient（单端点）和 CdpPool（多端点池 + 策略）。
//!
//! 前置条件：启动 CDP 服务端，例如：
//! ```bash
//! # Lightpanda
//! lightpanda serve --port 9222
//!
//! # Obscura（多实例）
//! obscura serve --port 9222
//! obscura serve --port 9223
//!
//! # Chrome
//! chrome --remote-debugging-port=9222 --headless
//! ```
//!
//! 运行：
//! ```bash
//! cargo run --example cdp_example --features cdp
//! ```

use std::collections::HashMap;
use std::time::Duration;

use crawlkit::{Collector, CompositeFetcher, HttpClient, ReqwestClient};
use crawlkit_extensions::cdp::{CdpClient, CdpPool, CdpStrategy};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== CDP 无头浏览器示例 ===\n");

    // ---- 1. 单端点 ----
    println!("--- 1. 单端点 CdpClient ---\n");

    let cdp = CdpClient::builder()
        .with_endpoint("http://127.0.0.1:9222")
        .with_navigation_timeout(Duration::from_secs(15))
        .build()
        .await?;

    let resp = cdp.get("https://example.com", &HashMap::new()).await?;
    println!("  URL: {}", resp.url);
    println!("  长度: {} 字节\n", resp.body.len());

    // ---- 2. 多端点池：轮询策略 ----
    println!("--- 2. CdpPool 轮询（RoundRobin）---\n");

    let pool = CdpPool::builder()
        .with_named_endpoint("lightpanda", "http://127.0.0.1:9222")
        .with_named_endpoint("obscura", "http://127.0.0.1:9223")
        .with_strategy(CdpStrategy::RoundRobin)
        .with_navigation_timeout(Duration::from_secs(15))
        .build()
        .await?;

    println!("  健康端点: {}/{}", pool.healthy_count().await, pool.total_count().await);

    for i in 0..4 {
        let resp = pool.get("https://httpbin.org/ip", &HashMap::new()).await?;
        println!("  请求 {}: {} ({} 字节)", i + 1, resp.url, resp.body.len());
    }

    // ---- 3. 多端点池：随机策略 ----
    println!("\n--- 3. CdpPool 随机（Random）---\n");

    let pool_random = CdpPool::builder()
        .with_endpoint("http://127.0.0.1:9222")
        .with_endpoint("http://127.0.0.1:9223")
        .with_strategy(CdpStrategy::Random)
        .build()
        .await?;

    for i in 0..3 {
        let resp = pool_random.get("https://example.com", &HashMap::new()).await?;
        println!("  请求 {}: {} 字节", i + 1, resp.body.len());
    }

    // ---- 4. 多端点池：故障转移策略 ----
    println!("\n--- 4. CdpPool 故障转移（Failover）---\n");

    let pool_failover = CdpPool::builder()
        .with_named_endpoint("primary", "http://127.0.0.1:9222")
        .with_named_endpoint("backup", "http://127.0.0.1:9223")
        .with_strategy(CdpStrategy::Failover)
        .build()
        .await?;

    let resp = pool_failover.get("https://example.com", &HashMap::new()).await?;
    println!("  获取成功: {} 字节", resp.body.len());

    // ---- 5. 集成到 CompositeFetcher（reqwest 优先 → CDP 兜底）----
    println!("\n--- 5. CompositeFetcher 集成 ---\n");

    let fetcher = CompositeFetcher::new(vec![
        Box::new(ReqwestClient::builder().name("reqwest").build()?),
        Box::new(
            CdpPool::builder()
                .with_endpoint("http://127.0.0.1:9222")
                .with_strategy(CdpStrategy::Failover)
                .build()
                .await?,
        ),
    ])
    .on_backend_error(|name, err| {
        eprintln!("  [{}] 失败: {}", name, err);
    });

    let collector = Collector::with_client(fetcher);
    let mut collector = collector;
    collector.on_html(|ctx| {
        println!("  [HTML] {} - {} 字节", ctx.url, ctx.body.len());
    });

    collector.visit("https://news.ycombinator.com/").await?;

    Ok(())
}
