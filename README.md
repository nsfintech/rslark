# rslark

A Rust SDK for Feishu and Lark. This initial crate provides configuration and
module boundaries for future authentication, HTTP, WebSocket, event, instant
messaging, and interactive card support. Network operations are not implemented
yet.

The default client configuration targets Feishu. Use `ClientConfig::lark()` for
Lark or `ClientConfig::with_api_base_url(...)` for a custom API endpoint.
