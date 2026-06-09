I have everything needed. `MessageContent` does not exist anywhere in the ecosystem; the message payload type is `waproto::whatsapp::Message`. Let me confirm the `Event` enum is `pub` in `wacore::types::events` (so reachable as `whatsapp_rust::types::Event`) and double-check there's no top-level `Event` re-export in whatsapp-rust lib.rs (there wasn't one in the lib.rs I read).

I now have all the verbatim data. Here is the complete report.

---

# BotBuilder pluggable deps: exact concrete types, constructors, and traits

All paths are absolute. The builder lives in
`/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/bot.rs`.

Crate inventory (registry root `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`):
- `whatsapp-rust-tokio-transport-0.6.0` → crate `whatsapp_rust_tokio_transport`
- `whatsapp-rust-ureq-http-client-0.6.0` → crate `whatsapp_rust_ureq_http_client`
- `whatsapp-rust-0.6.0` → crate `whatsapp_rust` (this is where `TokioRuntime` lives; there is **no** `tokio-runtime` crate, it's a feature-gated module)

---

## 1. TokioWebSocketTransportFactory

- **Crate:** `whatsapp_rust_tokio_transport` (also re-exported by `whatsapp_rust::transport`)
- **File:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-tokio-transport-0.6.0/src/lib.rs`
- **Constructor:** `TokioWebSocketTransportFactory::new()` (also `Default`; builder-style `.with_url(...)` and `.with_connector(...)`).
- **Trait implemented:** `TransportFactory` (defined in `wacore::net`, re-exported as `whatsapp_rust::transport::TransportFactory`).

Concrete type + constructor (lib.rs lines 229-262):
```rust
pub struct TokioWebSocketTransportFactory {
    url: String,
    connector: Option<Connector>,
}

impl TokioWebSocketTransportFactory {
    pub fn new() -> Self {
        Self {
            url: WHATSAPP_WEB_WS_URL.to_string(),
            connector: None,
        }
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    pub fn with_connector(mut self, connector: Connector) -> Self {
        self.connector = Some(connector);
        self
    }
}

impl Default for TokioWebSocketTransportFactory {
    fn default() -> Self {
        Self::new()
    }
}
```

`TransportFactory` trait definition — file `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/net.rs` (lines 33-41):
```rust
/// A factory responsible for creating new transport instances.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait TransportFactory: Send + Sync {
    /// Creates a new transport and returns it, along with a stream of events.
    async fn create_transport(
        &self,
    ) -> Result<(Arc<dyn Transport>, async_channel::Receiver<TransportEvent>), anyhow::Error>;
}
```

---

## 2. UreqHttpClient

- **Crate:** `whatsapp_rust_ureq_http_client` (also re-exported as `whatsapp_rust::transport::UreqHttpClient` under the `ureq-client` feature; note: re-exported from the `transport` module, not `http`).
- **File:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-ureq-http-client-0.6.0/src/lib.rs`
- **Constructor:** `UreqHttpClient::new()` (also `Default`; plus `UreqHttpClient::with_agent(ureq::Agent)` and `.with_max_body_bytes(u64)`).
- **Trait implemented:** `HttpClient` (defined in `wacore::net`, re-exported as `whatsapp_rust::http::HttpClient`).

Concrete type + constructor (lib.rs lines 15-54):
```rust
#[derive(Debug, Clone)]
pub struct UreqHttpClient {
    agent: ureq::Agent,
    max_body_bytes: u64,
}

impl UreqHttpClient {
    pub fn new() -> Self {
        Self {
            agent: build_agent(),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    pub fn with_agent(agent: ureq::Agent) -> Self {
        Self {
            agent,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    pub fn with_max_body_bytes(mut self, max_body_bytes: u64) -> Self {
        self.max_body_bytes = max_body_bytes;
        self
    }
}

impl Default for UreqHttpClient {
    fn default() -> Self {
        Self::new()
    }
}
```

`HttpClient` trait definition — file `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/net.rs` (lines 102-121):
```rust
/// Trait for executing HTTP requests in a runtime-agnostic way
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait HttpClient: Send + Sync {
    /// Executes a given HTTP request and returns the response.
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse>;

    /// Whether this client supports synchronous streaming downloads.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Synchronous streaming variant — returns a reader over the response body.
    /// Must be called from a blocking context.
    fn execute_streaming(&self, _request: HttpRequest) -> Result<StreamingHttpResponse> {
        Err(anyhow::anyhow!(
            "Streaming not supported by this HTTP client"
        ))
    }
}
```

---

## 3. TokioRuntime

- **Lives in:** `whatsapp_rust::TokioRuntime` (NOT a separate `tokio-runtime` crate; no such crate exists). It is a feature-gated (`tokio-runtime` *feature*) re-export.
- **Source file:** `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/runtime_impl.rs`
- **Re-export:** `whatsapp-rust-0.6.0/src/lib.rs` lines 41-45:
  ```rust
  #[cfg(feature = "tokio-runtime")]
  pub mod runtime_impl;
  #[cfg(feature = "tokio-runtime")]
  pub use runtime_impl::TokioRuntime;
  pub use wacore::runtime::Runtime;
  ```
- **How to construct it:** It is a **unit struct**. Construct it as the value `TokioRuntime` — **NOT** `TokioRuntime::new()` (there is no `new`). The bot tests pass it as `.with_runtime(TokioRuntime)`.

Definition (runtime_impl.rs lines 11-42):
```rust
pub struct TokioRuntime;

#[async_trait]
impl Runtime for TokioRuntime {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) -> AbortHandle {
        let handle = tokio::spawn(future);
        AbortHandle::new(move || handle.abort())
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(tokio::time::sleep(duration))
    }

    fn spawn_blocking(
        &self,
        f: Box<dyn FnOnce() + Send + 'static>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async {
            let _ = tokio::task::spawn_blocking(f).await;
        })
    }

    fn yield_now(&self) -> Option<Pin<Box<dyn Future<Output = ()> + Send>>> {
        None
    }
}
```

`Runtime` trait definition (native, non-wasm) — file `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/runtime.rs` (lines 11-38):
```rust
#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
pub trait Runtime: Send + Sync + 'static {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) -> AbortHandle;
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>>;
    fn spawn_blocking(
        &self,
        f: Box<dyn FnOnce() + Send + 'static>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;

    /// Cooperatively yield, allowing other tasks and I/O to make progress.
    fn yield_now(&self) -> Option<Pin<Box<dyn Future<Output = ()> + Send>>>;

    /// How often to yield in tight loops (every N items). Defaults to 10.
    fn yield_frequency(&self) -> u32 {
        10
    }
}
```

---

## 4. Pass-by-move (generic) vs `Arc<dyn ...>` — exact builder signatures

All three required setters take the value **by move as a generic** and box it into an `Arc` internally. The caller passes the bare concrete value (e.g. `TokioWebSocketTransportFactory::new()`, `UreqHttpClient::new()`, `TokioRuntime`), **not** a pre-wrapped `Arc`. (`with_backend` is the exception: it takes `Arc<dyn Backend>` directly.)

Verbatim from `bot.rs`:

`with_backend` — takes `Arc<dyn Backend>` (line 325):
```rust
pub fn with_backend(self, backend: Arc<dyn Backend>) -> BotBuilder<Provided, T, H, R> {
```

`with_transport_factory` — generic by move (lines 361-364):
```rust
pub fn with_transport_factory<F>(self, factory: F) -> BotBuilder<B, Provided, H, R>
where
    F: crate::transport::TransportFactory + 'static,
{
```
(internally: `transport_factory: Some(Arc::new(factory))`)

`with_http_client` — generic by move (lines 399-402):
```rust
pub fn with_http_client<C>(self, client: C) -> BotBuilder<B, T, Provided, R>
where
    C: crate::http::HttpClient + 'static,
{
```
(internally: `http_client: Some(Arc::new(client))`)

`with_runtime` — generic by move (line 425):
```rust
pub fn with_runtime<Rt: Runtime>(self, runtime: Rt) -> BotBuilder<B, T, H, Provided> {
```
(internally: `runtime: Some(Arc::new(runtime))`)

The internal storage fields (bot.rs lines 272-275) are the trait objects:
```rust
backend: Option<Arc<dyn Backend>>,
transport_factory: Option<Arc<dyn crate::transport::TransportFactory>>,
http_client: Option<Arc<dyn crate::http::HttpClient>>,
runtime: Option<Arc<dyn Runtime>>,
```

Typestate note: `build()` is only callable on `BotBuilder<Provided, Provided, Provided, Provided>`, so all four required setters must run (in any order).

---

## 5. Exact `use` paths for a downstream crate

```rust
use whatsapp_rust::bot::Bot;
use whatsapp_rust::Client;                       // re-exported at crate root (also whatsapp_rust::client::Client)
use whatsapp_rust::types::Event;                 // wacore::types::events::Event, re-exported via whatsapp_rust::types::*
use whatsapp_rust::store::traits::Backend;       // also whatsapp_rust::store::Backend (glob re-export)

// The store traits (wacore defines FOUR domain traits; Backend is the 5th, combined, trait).
// All reachable through whatsapp_rust::store::traits::* (and whatsapp_rust::store::*):
use whatsapp_rust::store::traits::SignalStore;
use whatsapp_rust::store::traits::AppSyncStore;
use whatsapp_rust::store::traits::ProtocolStore;
use whatsapp_rust::store::traits::DeviceStore;
use whatsapp_rust::store::traits::Backend;       // the combined trait (blanket-impl'd)

use whatsapp_rust::store::StoreError;            // = wacore::store::error::StoreError (re-exported)

use whatsapp_rust::TokioRuntime;                 // requires the "tokio-runtime" feature
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory; // or whatsapp_rust::transport::TokioWebSocketTransportFactory ("tokio-transport" feature)
use whatsapp_rust_ureq_http_client::UreqHttpClient;                // or whatsapp_rust::transport::UreqHttpClient ("ureq-client" feature)

use whatsapp_rust::pair_code::PairCodeOptions;   // = wacore::pair_code::PairCodeOptions (re-exported)
use whatsapp_rust::Jid;                          // re-exported at crate root from wacore_binary (also wacore_binary::Jid)
```

### IMPORTANT corrections / things you asked that don't exist as stated

- **`MessageContent` does not exist** anywhere in `wacore`, `whatsapp-rust`, `wacore-binary`, or the project. There is no type by that name to import. The message payload type used throughout the API (e.g. `Client::send_message`, `MessageContext::send_message`) is `waproto::whatsapp::Message` — imported as:
  ```rust
  use whatsapp_rust::waproto::whatsapp as wa;   // then wa::Message
  // or: use waproto::whatsapp::Message;
  ```
  The closest *named* type in `bot.rs` is **`MessageContext`** (`whatsapp_rust::bot::MessageContext`), a context wrapper around `Arc<wa::Message>` + `MessageInfo` + `Arc<Client>`. If you meant that, import `use whatsapp_rust::bot::MessageContext;`. Please confirm which one you intended.

- **"5 store traits":** the source documents only **four** domain traits (`SignalStore`, `AppSyncStore`, `ProtocolStore`, `DeviceStore`) defined in `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/store/traits.rs`, plus the combined **`Backend`** trait (auto-implemented via blanket impl for any type implementing all four). I listed all five above (the 4 domain traits + `Backend`). There is no fifth *domain* trait.

- **`Event` has no crate-root re-export** in `whatsapp_rust`; it comes through the `types` glob, so use `whatsapp_rust::types::Event` (canonical origin `wacore::types::events::Event`, file `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/types/events.rs:391`).

- **No `tokio-runtime` crate** exists (your grep used `tokio-runtime`). `TokioRuntime` is a feature-gated module *inside* the `whatsapp-rust` crate (`runtime_impl.rs`), re-exported at the crate root under the `tokio-runtime` Cargo feature.

### `StoreError` definition (verbatim)
File `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/store/error.rs` (lines 3-34); re-exported by `whatsapp-rust-0.6.0/src/store/error.rs` as `pub use wacore::store::error::{Result, StoreError};`:
```rust
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("I/O error")]
    Io(#[from] std::io::Error),
    #[error("serialization/deserialization error")]
    Serialization(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("data validation failed: {0}")]
    Validation(String),
    #[error("database connection error")]
    Connection(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("database operation error")]
    Database(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("database operation '{op}' exhausted retries")]
    RetriesExhausted { op: String },
    #[error("migration error")]
    Migration(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("store configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("device with ID {0} not found")]
    DeviceNotFound(i32),
}
```

### Concrete `Backend` you'll most likely inject (for context)
`SqliteStore` (crate `whatsapp_rust_sqlite_storage`, re-exported as `whatsapp_rust::store::SqliteStore` under the `sqlite-storage` feature). File `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-sqlite-storage-0.6.0/src/sqlite_store.rs`. Constructors: `SqliteStore::new(database_url: &str) -> Result<Self, StoreError>` (line 153) and `SqliteStore::new_for_device(...)` (line 192). Pass it as `Arc::new(SqliteStore::new(...).await?) as Arc<dyn Backend>` to `.with_backend(...)`.

### Minimal canonical build call (from bot.rs tests, lines 759-766)
```rust
let bot = Bot::builder()
    .with_backend(backend)                                    // Arc<dyn Backend>
    .with_transport_factory(TokioWebSocketTransportFactory::new())  // by-move generic
    .with_http_client(UreqHttpClient::new())                  // by-move generic
    .with_runtime(TokioRuntime)                               // unit struct, by-move generic
    .build()
    .await?;
```