# rslark

`rslark` is a Rust SDK for the Feishu and Lark Open APIs. It supports
tenant-access-token caching, common instant-messaging APIs, Card JSON and
CardKit cards, and the binary WebSocket long-connection event protocol.

## Installation

The crate has not yet been published to a package registry. Use a Git
dependency while development is on the `test` branch:

```toml
[dependencies]
rslark = { git = "https://github.com/nsfintech/rslark.git", branch = "test" }
```

The default features include Tokio, Reqwest with Rustls, WebSocket support,
Serde, and Prost. The minimum supported Rust version is 1.85.

## Create a client

```rust
use rslark::{AppCredentials, Client};

# async fn example() -> Result<(), rslark::Error> {
let feishu = Client::with_app_credentials(AppCredentials::new("cli_app", "secret"))?;
let lark = Client::with_lark_credentials(AppCredentials::new("cli_app", "secret"))?;

assert_eq!(feishu.config().api_base_url, "https://open.feishu.cn");
assert_eq!(lark.config().api_base_url, "https://open.larksuite.com");
# Ok(())
# }
```

The client obtains a `tenant_access_token`, caches it in memory, and refreshes
it five minutes before expiry. API requests automatically carry the token.

## Authorize a user with Device Flow

User authorization uses OAuth Device Flow and does not require a redirect URL
or HTTP callback. The application must show the authorization URI and user code
to the correct user; the SDK does not assume a Feishu card, chat context, or
other interactive UI.

```rust
use rslark::{
    AppCredentials, ClientConfig, DeviceFlowClient, InMemoryTokenStore, TokenStore,
};
use std::time::Duration;

# async fn example() -> Result<(), rslark::Error> {
let credentials = AppCredentials::new("cli_app", "app_secret");
let device_flow = DeviceFlowClient::new(credentials, ClientConfig::feishu())?;
let store = InMemoryTokenStore::default();

let authorization = device_flow
    .start(["im:message", "im:message.send_as_user", "offline_access"])
    .await?;

// Your application renders this to the user. For example, show a terminal
// prompt, web page, email, or another channel that the user already trusts.
println!(
    "Open {} and enter code {}",
    authorization
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&authorization.verification_uri),
    authorization.user_code
);

let user_token = device_flow
    .poll(
        authorization.device_code,
        Some(Duration::from_secs(authorization.interval)),
        Some(Duration::from_secs(authorization.expires_in)),
    )
    .await?;

// Feishu token responses do not include open_id. Use your own stable user key,
// such as an open_id obtained before authorization, as the TokenStore key.
store.set("ou_user", user_token).await?;
# Ok(())
# }
```

`poll` follows the returned interval and handles `authorization_pending`,
`slow_down`, `access_denied`, and `expired_token`. `slow_down` increases the
next interval by five seconds. The token type redacts tokens in its `Debug`
output; the SDK does not log credentials.

Refresh a token before it expires. A refresh token is single-use: if a new
`refresh_token` is returned, persist the whole token immediately and discard
the old refresh token.

```rust
use rslark::{DeviceFlowClient, TokenStore};
use std::time::Duration;

# async fn example(
#     device_flow: DeviceFlowClient,
#     store: impl TokenStore,
#     refresh_token: &str,
# ) -> Result<(), rslark::Error> {
let refreshed = device_flow.refresh(refresh_token).await?;
store.set("ou_user", refreshed).await?;
# Ok(())
# }
```

`TokenStore` is an async trait for production storage backends. The built-in
`InMemoryTokenStore` is process-local and intended for tests and short-lived
processes. For persistence, implement `TokenStore` with an encrypted or
access-controlled backend such as a KMS, Vault, or database.

## Send and receive messages

```rust
use rslark::ReceiveIdType;

# async fn example(client: rslark::Client) -> Result<(), rslark::Error> {
let message = client
    .send_text(
        ReceiveIdType::ChatId,
        "oc_chat",
        "Deployment finished",
        Some("message-deduplication-id".into()),
    )
    .await?;

println!("{}", message.message_id.as_deref().unwrap_or_default());

let reply = client
    .reply_text("om_message", "Acknowledged", None, None)
    .await?;
println!("{}", reply.message_id.as_deref().unwrap_or_default());
# Ok(())
# }
```

`send_message` and `reply_message` also accept raw platform message JSON when
an application needs message types without dedicated helpers.

To create a message as the authorized user, use `send_message_as_user`. The
user token is sent directly as `Authorization: Bearer <user_access_token>` and
is never exchanged for or mixed with the tenant token.

```rust
use rslark::ReceiveIdType;
use rslark::im::SendMessageRequest;

# async fn example(
#     client: rslark::Client,
#     access_token: &str,
# ) -> Result<(), rslark::Error> {
let message = client
    .send_message_as_user(
        access_token,
        ReceiveIdType::OpenId,
        SendMessageRequest {
            receive_id: "ou_user".into(),
            msg_type: "text".into(),
            content: r#"{"text":"Sent as the user"}"#.into(),
            uuid: Some("user-message-uuid".into()),
        },
    )
    .await?;
println!("{}", message.message_id.as_deref().unwrap_or_default());
# Ok(())
# }
```

For user-identity message creation, the current official IM send-message
documentation requires both `im:message` and `im:message.send_as_user`. Scopes
must also be approved for the application; requesting a scope in Device Flow
does not bypass open-platform permission approval.

## Send interactive cards

```rust
use rslark::card::CardBuilder;
use rslark::ReceiveIdType;

# async fn example(client: rslark::Client) -> Result<(), rslark::Error> {
let card = CardBuilder::new()
    .title("Deployment")
    .template("blue")
    .markdown("Deployment succeeded")
    .build()?;

client
    .send_card(ReceiveIdType::ChatId, "oc_chat", &card, None)
    .await?;
# Ok(())
# }
```

`Card::new` accepts any valid Card JSON object. `Client::create_card` creates
a CardKit entity, and `Client::send_card_entity` sends that reusable entity.

## Update Card Kit streaming cards

CardKit streaming updates use a caller-owned monotonic sequence. The SDK sends
the requests in the order you call them but intentionally provides no throttling,
task state, retry loop, or card layout policy; those concerns belong to the
backend using the card.

```rust
use rslark::ReceiveIdType;
use rslark::card::{CardBuilder, UpdateCardContentRequest, UpdateCardRequest, UpdateCardSettingsRequest};

# async fn example(
#     client: rslark::Client,
#     receive_id: &str,
# ) -> Result<(), rslark::Error> {
let initial = CardBuilder::new()
    .streaming_mode(true)
    .markdown_with_id("research_stream", "**准备中**")
    .build()?;
let created = client.create_streaming_card(&initial).await?;
client
    .send_card_entity(
        ReceiveIdType::ChatId,
        receive_id,
        created.card_id.as_str(),
        None,
    )
    .await?;

client
    .update_card_element_content(
        &created.card_id,
        "research_stream",
        UpdateCardContentRequest::new("**分析中**\n\n已收集 3 条资料", 1),
    )
    .await?;

let final_card = CardBuilder::new()
    .markdown("### 研究报告\n\n最终内容")
    .build()?;
client
    .update_card(
        &created.card_id,
        UpdateCardRequest::new(&final_card, 2)?,
    )
    .await?;

let settings = serde_json::json!({ "config": { "streaming_mode": false } });
client
    .update_card_settings(
        &created.card_id,
        UpdateCardSettingsRequest::new(&settings, 3)?,
    )
    .await?;
# Ok(())
# }
```

For the same card, every update must use a strictly increasing `sequence`.
The value is an `i32` because the platform accepts `1..=2147483647`. The final
settings update can set `config.streaming_mode` to `false` after replacing the
placeholder with the final report.

## Receive events over WebSocket

```rust
use rslark::events::{CardActionEvent, EventRouter, MessageEvent, RawEvent};
use rslark::{AppCredentials, ClientConfig, WsClient};

# fn example() -> Result<(), rslark::Error> {
let router = EventRouter::new()
    .on("im.message.receive_v1", |event: &RawEvent| {
        let message = MessageEvent::parse(event)?;
        println!("received {:?}", message.message.message_id);
        Ok(None)
    })
    .on("card.action.trigger", |event: &RawEvent| {
        let action = CardActionEvent::parse(event)?;
        println!("card action: {:?}", action.action);
        Ok(None)
    });

let client = WsClient::new(
    AppCredentials::new("cli_app", "secret"),
    ClientConfig::feishu(),
)?
.with_router(router);

tokio::select! {
    result = client.run() => result?,
    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => (),
}
# Ok(())
# }
```

The WebSocket client bootstraps through `/callback/ws/endpoint`, sends the
platform's binary Ping frames, reassembles fragmented events, routes schema
1.0 and 2.0 payloads, writes event ACK frames, and reconnects using the
server-provided retry policy. Disable this with `with_auto_reconnect(false)`.

A card-action handler can return `Some(json!({ "toast": ... }))` or a card
response in its `Ok` value; the value is placed in the callback ACK's `data`
field.

## API coverage

Current Open API support includes:

- Self-built application `tenant_access_token` acquisition and cached refresh.
- OAuth Device Flow authorization, user access token polling and refresh, and
  an async TokenStore abstraction with an in-memory implementation.
- IM message create, text create, interactive-card create, reply, text reply,
  get, and recall, including message create with an explicit user access
  token.
- Card JSON 2.0 construction, CardKit entity creation, and card-entity
  delivery.
- Streaming CardKit creation, Markdown element content updates, full-card
  replacement, and settings updates (including turning streaming mode off).
- Event-name routing, message-event parsing, card-action parsing, and
  Feishu/Lark WebSocket long connections.

The SDK does not yet cover user OAuth tokens, marketplace-application tokens,
media upload APIs, every CardKit update operation, or every IM endpoint. Raw
platform JSON can be sent with `send_message` where a message type is already
supported by the platform but lacks a typed helper.
