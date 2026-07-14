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

pub use jina::JinaClient;
#[cfg(feature = "firecrawl")]
pub use firecrawl::FirecrawlClient;
