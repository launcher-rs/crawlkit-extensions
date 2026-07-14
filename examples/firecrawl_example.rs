//! Firecrawl 示例
//!
//! 演示如何使用 FirecrawlClient 获取需要 JS 渲染的页面。
//!
//! 运行：
//! ```bash
//! FIRECRAWL_API_TOKENS=fc-token-1,fc-token-2 cargo run --example firecrawl_example
//! ```
//!
//! 需要启用 `firecrawl` feature：
//! ```bash
//! cargo run --example firecrawl_example --features firecrawl
//! ```

use std::collections::HashMap;

use crawlkit::{Collector, CompositeFetcher, ReqwestClient};
use crawlkit_extensions::firecrawl::FirecrawlClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Firecrawl 示例 ===\n");

    // 1. 单独使用 FirecrawlClient（多 token 轮换）
    let firecrawl = FirecrawlClient::builder()
        .with_tokens_from_env("FIRECRAWL_API_TOKENS")
        .build()?;

    let resp = firecrawl.get("https://example.com", &HashMap::new()).await?;
    println!("Firecrawl 获取成功：");
    println!("  URL: {}", resp.url);
    println!("  状态码: {}", resp.status);
    println!("  长度: {} 字节\n", resp.body.len());

    // 2. 组合：reqwest 优先 → Firecrawl 兜底（JS 渲染）
    println!("=== CompositeFetcher 集成 ===\n");

    let fetcher = CompositeFetcher::new(vec![
        Box::new(ReqwestClient::builder().name("reqwest").max_retries(2).build()?),
        Box::new(firecrawl),
    ])
    .on_backend_error(|name, err| {
        eprintln!("  [{}] 失败: {}", name, err);
    });

    let mut collector = Collector::with_client(fetcher);
    collector.on_html(|ctx| {
        println!("  [HTML] {} - {} 字节", ctx.url, ctx.body.len());
    });

    // 尝试抓取 SPA 站点
    collector.visit("https://news.ycombinator.com/").await?;

    Ok(())
}
