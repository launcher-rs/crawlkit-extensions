//! # crawlkit-extensions
//!
//! crawlkit 官方第三方服务扩展集，提供开箱即用的云爬虫后端实现。
//!
//! ## 支持的服务
//!
//! | 服务 | Feature | 免费额度 | 说明 |
//! |------|---------|---------|------|
//! | [Jina Reader](https://jina.ai/reader) | `jina`（默认） | 1000 次/月 | 网页转 Markdown |
//! | [Firecrawl](https://firecrawl.dev) | `firecrawl` | 500 次/月 | JS 渲染 + 结构化提取 |
//! | [Lightpanda](https://github.com/lightpanda-io/browser) | `cdp` | 免费开源 | 轻量级 CDP 无头浏览器 |
//! | [Obscura](https://github.com/h4ckf0r0day/obscura) | `cdp` | 免费开源 | Rust 编写，内置反检测 |
//!
//! ## 快速开始
//!
//! ```toml
//! [dependencies]
//! crawlkit-extensions = "0.1"
//! ```
//!
//! ```rust
//! use crawlkit_extensions::jina::JinaClient;
//!
//! let client = JinaClient::builder()
//!     .with_token("your-api-token")
//!     .build();
//! ```

pub mod jina;

#[cfg(feature = "firecrawl")]
pub mod firecrawl;

#[cfg(feature = "cdp")]
pub mod cdp;

pub use jina::JinaClient;
#[cfg(feature = "firecrawl")]
pub use firecrawl::FirecrawlClient;
#[cfg(feature = "cdp")]
pub use cdp::CdpClient;
