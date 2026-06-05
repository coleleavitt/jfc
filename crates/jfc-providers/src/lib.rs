//! Concrete provider backends implementing the `jfc-provider` traits.
//!
//! Supports Anthropic (API key + multi-account Claude.ai OAuth), OpenAI,
//! Codex/ChatGPT OAuth, OpenWebUI/LiteLLM proxies, Bedrock, Vertex, and
//! Gemini/Antigravity. Each backend handles its own auth lifecycle, model
//! catalogue, streaming SSE parsing, and request/response transformation,
//! including inline `<tool_call>` XML interception for proxy routes that don't
//! emit native tool calls.

pub mod anthropic;
pub mod anthropic_accounts;
pub mod anthropic_models;
pub mod anthropic_oauth;
pub mod anthropic_oauth_login;
pub mod antigravity_oauth;
pub mod antigravity_transform;
pub mod bedrock;
pub mod bedrock_wizard;
pub mod codex_oauth;
pub mod gemini_api;

pub mod litellm;
pub mod login_dispatch;
pub mod models_dev;
pub mod openai;
pub mod openrouter;
pub mod openwebui;
mod sse;
pub mod unified;
pub mod vertex;
pub mod vertex_wizard;

pub use anthropic::AnthropicProvider;
pub use anthropic_oauth::AnthropicOAuthProvider;
pub use antigravity_oauth::AntigravityOAuthProvider;
pub use bedrock::BedrockProvider;
pub use codex_oauth::CodexOAuthProvider;
pub use gemini_api::GeminiApiProvider;
pub use litellm::LiteLLMProvider;
pub use openai::OpenAIProvider;
pub use openrouter::OpenRouterProvider;
pub use openwebui::OpenWebUIProvider;
pub use vertex::VertexProvider;
