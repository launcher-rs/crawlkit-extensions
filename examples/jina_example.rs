//! Jina Reader 示例
//!
//! 演示如何使用 JinaClient 获取网页内容并集成到 CompositeFetcher。
//!
//! 运行：
//! ```bash
//! JINA_API_TOKEN=your-token cargo run --example jina_example
//! ```

use std::collections::HashMap;

use crawlkit::{Collector, CompositeFetcher, ReqwestClient};
use crawlkit_core::HttpClient;
use crawlkit_extensions::jina::JinaClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // 1. 单独使用 JinaClient
    println!("=== Jina Reader 示例 ===\n");

    let jina = JinaClient::builder()
        .with_token_from_env("JINA_API_TOKEN")
        .build();

    let resp = jina.get("https://example.com", &HashMap::new()).await?;
    println!("Jina 获取成功：");
    println!("  URL: {}", resp.url);
    println!("  状态码: {}", resp.status);
    println!("  长度: {} 字节", resp.body.len());
    println!("  前 200 字符: {}\n", &resp.body[..resp.body.len().min(200)]);

    // 2. 组合到 CompositeFetcher（reqwest 优先，Jina 兜底）
    println!("=== CompositeFetcher 集成 ===\n");

    let reqwest_client = ReqwestClient::builder()
        .name("reqwest")
        .max_retries(2)
        .build()?;

    let fetcher = CompositeFetcher::new(vec![Box::new(reqwest_client), Box::new(jina)]);

    let mut collector = Collector::with_client(fetcher);
    collector.on_html(|ctx| {
        println!("  [HTML] {} - {} 字节", ctx.url, ctx.body.len());
    });

    collector.visit("https://httpbin.org/get").await?;

    // 3. 与 Collector 回调链集成
    println!("\n=== Collector 回调链 ===\n");

    let jina2 = JinaClient::builder()
        .with_token_from_env("JINA_API_TOKEN")
        .build();

    let fetcher2 = CompositeFetcher::new(vec![
        Box::new(ReqwestClient::builder().name("reqwest").build()?),
        Box::new(jina2),
    ]);

    let mut collector2 = Collector::with_client(fetcher2);
    collector2.on_request(|req| println!("  [请求] {}", req.url));
    collector2.on_response(|resp| println!("  [响应] {} - {}", resp.url, resp.status));
    collector2.on_html(|ctx| println!("  [HTML] {} 字节", ctx.body.len()));
    collector2.on_error(|err| eprintln!("  [错误] {err}"));

    collector2.visit("https://news.ycombinator.com/").await?;

    Ok(())
}
