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
- IM message create, text create, interactive-card create, reply, text reply,
  get, and recall.
- Card JSON 2.0 construction, CardKit entity creation, and card-entity
  delivery.
- Event-name routing, message-event parsing, card-action parsing, and
  Feishu/Lark WebSocket long connections.

The SDK does not yet cover user OAuth tokens, marketplace-application tokens,
media upload APIs, full CardKit card updates, or every IM endpoint. Raw
platform JSON can be sent with `send_message` where a message type is already
supported by the platform but lacks a typed helper.
