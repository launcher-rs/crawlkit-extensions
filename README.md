# crawlkit-extensions

crawlkit 官方第三方服务扩展集，提供开箱即用的云爬虫后端实现。

## 支持的服务

| 服务 | Feature | 免费额度 | 说明 |
|------|---------|---------|------|
| [Jina Reader](https://jina.ai/reader) | `jina`（默认） | 1000 次/月 | 网页转 Markdown |
| [Firecrawl](https://firecrawl.dev) | `firecrawl` | 500 次/月 | JS 渲染 + 结构化提取 |

## 安装

```toml
[dependencies]
crawlkit-extensions = "0.1"

# 可选：启用 Firecrawl 支持
crawlkit-extensions = { version = "0.1", features = ["firecrawl"] }
```

## 快速开始

### Jina Reader

```rust
use crawlkit_extensions::jina::JinaClient;

let client = JinaClient::builder()
    .with_token("your-api-token")           // 可选
    .with_timeout(Duration::from_secs(30))
    .build();

let response = client.get("https://example.com", &Default::default()).await?;
```

### Firecrawl

```rust
use crawlkit_extensions::firecrawl::FirecrawlClient;

let client = FirecrawlClient::builder()
    .with_tokens(&["token-1", "token-2"])  // 多 token 轮换
    .build()?;

let response = client.get("https://example.com", &Default::default()).await?;
```

### 集成到 CompositeFetcher

```rust
use crawlkit::{Collector, CompositeFetcher, ReqwestClient};
use crawlkit_extensions::jina::JinaClient;

let fetcher = CompositeFetcher::new(vec![
    Box::new(ReqwestClient::builder().name("reqwest").build()?),
    Box::new(JinaClient::new().build()),
]);

let collector = Collector::with_client(fetcher);
collector.visit("https://example.com").await?;
```

## 环境变量

| 变量 | 说明 |
|------|------|
| `JINA_API_TOKEN` | Jina Reader API Token |
| `FIRECRAWL_API_TOKENS` | Firecrawl API Token（逗号分隔多个） |

## 开发

```bash
# 联调模式（自动使用本地 crawlkit-core）
cargo check
cargo test

# 带 Firecrawl 支持
cargo check --features firecrawl
```

## License

MIT OR Apache-2.0
