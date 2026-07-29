//! CDP 连接诊断工具
//!
//! 测试 CDP 连接，验证 Obscura/Lightpanda/Chrome 兼容性。
//!
//! 前置条件：启动 CDP 服务端
//! ```bash
//! obscura serve --port 9222
//! ```
//!
//! 运行：
//! ```bash
//! cargo run --example cdp_diagnose --features cdp
//! ```

use std::collections::HashMap;
use std::time::Duration;

use crawlkit_extensions::cdp::CdpClient;
use crawlkit_core::HttpClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eprintln!("CDP 连接诊断工具\n");
    eprintln!("请确保 CDP 服务端已在运行，例如：");
    eprintln!("  obscura serve --port 9222\n");

    for port in [9222, 9223] {
        let endpoint = format!("http://127.0.0.1:{}", port);
        eprintln!("--- 测试端点: {} ---\n", endpoint);

        let client = match CdpClient::builder()
            .with_endpoint(&endpoint)
            .with_connection_timeout(Duration::from_secs(5))
            .with_navigation_timeout(Duration::from_secs(15))
            .with_name(&format!("cdp-{}", port))
            .build()
            .await
        {
            Ok(c) => {
                eprintln!("  ✓ 连接成功\n");
                c
            }
            Err(e) => {
                eprintln!("  ✗ 连接失败: {}\n", e);
                continue;
            }
        };

        eprintln!("--- 获取页面: https://example.com ---\n");

        match tokio::time::timeout(Duration::from_secs(20), client.get("https://example.com", &HashMap::new())).await {
            Ok(Ok(resp)) => {
                eprintln!("  ✓ 成功");
                eprintln!("  URL: {}", resp.url);
                eprintln!("  长度: {} 字节", resp.body.len());
                let preview: String = resp.body.chars().take(200).collect();
                for line in preview.lines().take(8) {
                    eprintln!("  {}", line);
                }
            }
            Ok(Err(e)) => eprintln!("  ✗ 失败: {}", e),
            Err(_) => eprintln!("  ✗ 超时"),
        }

        eprintln!();
    }

    Ok(())
}
