I have all the information needed. Here is the verbatim extraction.

# WhatsApp-rust / wacore: Events, Message-receive payloads, and Media download API

All paths absolute. Versions: `wacore-0.6.0`, `whatsapp-rust-0.6.0`, `waproto-0.6.0`.

---

## 1. The full `Event` enum

File: `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/types/events.rs` (lines 389-477)

```rust
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub enum Event {
    Connected(Connected),
    Disconnected(Disconnected),
    PairSuccess(PairSuccess),
    PairError(PairError),
    LoggedOut(LoggedOut),
    PairingQrCode {
        code: String,
        timeout: std::time::Duration,
    },
    /// Generated pair code for phone number linking.
    /// User should enter this code on their phone in WhatsApp > Linked Devices.
    PairingCode {
        /// The 8-character pairing code to display.
        code: String,
        /// Approximate validity duration (~180 seconds).
        timeout: std::time::Duration,
    },
    QrScannedWithoutMultidevice(QrScannedWithoutMultidevice),
    ClientOutdated(ClientOutdated),

    Message(Arc<wa::Message>, Arc<MessageInfo>),
    Receipt(Receipt),
    UndecryptableMessage(UndecryptableMessage),
    #[serde(skip)]
    Notification(Arc<OwnedNodeRef>),

    ChatPresence(ChatPresenceUpdate),
    Presence(PresenceUpdate),
    PictureUpdate(PictureUpdate),
    UserAboutUpdate(UserAboutUpdate),
    ContactUpdated(ContactUpdated),
    ContactNumberChanged(ContactNumberChanged),
    ContactSyncRequested(ContactSyncRequested),

    /// Group metadata/settings/participant change from w:gp2 notification.
    GroupUpdate(GroupUpdate),
    ContactUpdate(ContactUpdate),

    /// Incoming `<call>` stanza from the server (offer, preaccept, accept,
    /// reject, terminate). Mirror of WA Web's inbound call signaling.
    IncomingCall(IncomingCall),

    PushNameUpdate(PushNameUpdate),
    SelfPushNameUpdated(SelfPushNameUpdated),
    PinUpdate(PinUpdate),
    MuteUpdate(MuteUpdate),
    ArchiveUpdate(ArchiveUpdate),
    StarUpdate(StarUpdate),
    MarkChatAsReadUpdate(MarkChatAsReadUpdate),
    DeleteChatUpdate(DeleteChatUpdate),
    DeleteMessageForMeUpdate(DeleteMessageForMeUpdate),

    HistorySync(Box<LazyHistorySync>),
    OfflineSyncPreview(OfflineSyncPreview),
    OfflineSyncCompleted(OfflineSyncCompleted),

    /// Device list changed for a user (device added/removed/updated)
    DeviceListUpdate(DeviceListUpdate),

    /// Identity key changed (user reinstalled WhatsApp)
    IdentityChange(IdentityChange),

    /// Business account status changed (verified name, profile, conversion to personal)
    BusinessStatusUpdate(BusinessStatusUpdate),

    StreamReplaced(StreamReplaced),
    TemporaryBan(TemporaryBan),
    ConnectFailure(ConnectFailure),
    StreamError(StreamError),

    /// A contact changed their default disappearing messages setting.
    DisappearingModeChanged(DisappearingModeChanged),

    /// Newsletter live update (reaction counts changed, message updates, etc.).
    NewsletterLiveUpdate(NewsletterLiveUpdate),

    /// Raw decoded stanza, emitted before router dispatch.
    /// Library extension — no WA Web equivalent (WA Web has no raw stanza observer).
    /// Gated by `Client::set_raw_node_forwarding(true)` to avoid overhead when unused.
    #[serde(skip)]
    RawNode(Arc<OwnedNodeRef>),

    /// Server-pushed MEX (GraphQL) update. Routed by the textual `op_name`,
    /// which is stable across WA Web bundle releases.
    MexNotification(MexNotification),
}
```

Note: `#[non_exhaustive]` — match arms must include a `_ =>` fallback.

---

## 2. Message-receive variants and their payloads

### `Event::Message(Arc<wa::Message>, Arc<MessageInfo>)`

Tuple variant: the decrypted protobuf message `Arc<wa::Message>` plus envelope metadata `Arc<MessageInfo>`. Convenience accessors live on `Event` (events.rs lines 491-504):

```rust
impl Event {
    pub fn as_message(&self) -> Option<(&Arc<wa::Message>, &MessageInfo)> {
        if let Event::Message(msg, info) = self {
            Some((msg, &**info))
        } else {
            None
        }
    }

    pub fn message_text(&self) -> Option<&str> {
        let (msg, _) = self.as_message()?;
        msg.conversation.as_deref()
    }
}
```

### `MessageInfo` and `MessageSource` (sender/chat identification)

File: `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/types/message.rs`

`MessageSource` (lines 42-53) — identifies WHO and WHICH chat:

```rust
#[derive(Debug, Clone, Default, Serialize)]
pub struct MessageSource {
    pub chat: Jid,
    pub sender: Jid,
    pub is_from_me: bool,
    pub is_group: bool,
    pub addressing_mode: Option<AddressingMode>,
    pub sender_alt: Option<Jid>,
    pub recipient_alt: Option<Jid>,
    pub broadcast_list_owner: Option<Jid>,
    pub recipient: Option<Jid>,
}
```

`MessageInfo` (lines 123-146) — full envelope, embeds `MessageSource`:

```rust
#[derive(Debug, Clone, Default, Serialize)]
pub struct MessageInfo {
    pub source: MessageSource,
    pub id: MessageId,
    pub server_id: MessageServerId,
    pub r#type: String,
    pub push_name: String,
    pub timestamp: DateTime<Utc>,
    pub category: MessageCategory,
    pub multicast: bool,
    pub media_type: String,
    pub edit: EditAttribute,
    pub bot_info: Option<MsgBotInfo>,
    pub meta_info: MsgMetaInfo,
    pub verified_name: Option<wa::VerifiedNameCertificate>,
    pub device_sent_meta: Option<DeviceSentMeta>,
    /// Ephemeral duration in seconds, extracted from `contextInfo.expiration`.
    pub ephemeral_expiration: Option<u32>,
    /// Whether this message was delivered during offline sync.
    pub is_offline: bool,
    /// Set when this message was recovered via PDO rather than normal decryption.
    /// Contains the PDO request message ID.
    pub unavailable_request_id: Option<String>,
}
```

So to identify sender/chat: use `info.source.chat`, `info.source.sender`, `info.source.is_from_me`, `info.source.is_group`, `info.push_name`, `info.id`, `info.timestamp`.

### `Event::UndecryptableMessage(UndecryptableMessage)` (events.rs lines 700-706)

```rust
#[derive(Debug, Clone, Serialize)]
pub struct UndecryptableMessage {
    pub info: Arc<MessageInfo>,
    pub is_unavailable: bool,
    pub unavailable_type: UnavailableType,
    pub decrypt_fail_mode: DecryptFailMode,
}
```

Supporting enums (events.rs lines 683-698):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, crate::WireEnum)]
pub enum DecryptFailMode {
    #[wire = "show"]
    Show,
    #[wire = "hide"]
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq, crate::WireEnum)]
pub enum UnavailableType {
    #[wire_default]
    #[wire = "unknown"]
    Unknown,
    #[wire = "view_once"]
    ViewOnce,
}
```

### `Event::Receipt(Receipt)` (events.rs lines 708-714)

```rust
#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub source: crate::types::message::MessageSource,
    pub message_ids: Vec<MessageId>,
    pub timestamp: DateTime<Utc>,
    pub r#type: ReceiptType,
}
```

### Where the inbound media-download parameters come from

The media-download params are **not** on `MessageInfo`. They live inside the decrypted `Arc<wa::Message>` payload, on the specific media sub-message (`ImageMessage`, `VideoMessage`, `AudioMessage`, `DocumentMessage`, `StickerMessage`, ...). Those proto structs implement `Downloadable` directly via the `impl_downloadable!` macro in `wacore-0.6.0/src/download.rs` (lines 190-215), so you can pass e.g. `msg.image_message.as_ref().unwrap()` straight to `client.download(...)`.

Verbatim proto for `wa::message::ImageMessage`, showing the download-relevant fields, from `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/waproto-0.6.0/src/whatsapp.rs` (lines 8198-8259, fields trimmed to the load-bearing ones):

```rust
pub struct ImageMessage {
    #[prost(string, optional, tag = "1")]
    pub url: ::core::option::Option<::prost::alloc::string::String>,
    #[prost(string, optional, tag = "2")]
    pub mimetype: ::core::option::Option<::prost::alloc::string::String>,
    #[prost(string, optional, tag = "3")]
    pub caption: ::core::option::Option<::prost::alloc::string::String>,
    #[prost(bytes = "vec", optional, tag = "4")]
    pub file_sha256: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
    #[prost(uint64, optional, tag = "5")]
    pub file_length: ::core::option::Option<u64>,
    // ... height (6), width (7) ...
    #[prost(bytes = "vec", optional, tag = "8")]
    pub media_key: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "9")]
    pub file_enc_sha256: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
    // ... interactive_annotations (10) ...
    #[prost(string, optional, tag = "11")]
    pub direct_path: ::core::option::Option<::prost::alloc::string::String>,
    #[prost(int64, optional, tag = "12")]
    pub media_key_timestamp: ::core::option::Option<i64>,
    // ... jpeg_thumbnail (16), context_info (17), scans, etc. ...
    #[prost(string, optional, tag = "29")]
    pub static_url: ::core::option::Option<::prost::alloc::string::String>,
    // ...
}
```

The six download params (`direct_path`, `media_key`, `file_enc_sha256`, `file_sha256`, `file_length`, `mimetype`) all come off this proto struct. `VideoMessage`, `AudioMessage`, `DocumentMessage`, `StickerMessage`, `StickerPackMessage` carry the same set of fields and all get a `Downloadable` impl. The macro maps these fields to the trait; the `Downloadable` trait reads them (wacore `download.rs` lines 125-146):

```rust
pub trait Downloadable: Sync + Send {
    fn direct_path(&self) -> Option<&str>;
    fn media_key(&self) -> Option<&[u8]>;
    fn file_enc_sha256(&self) -> Option<&[u8]>;
    fn file_sha256(&self) -> Option<&[u8]>;
    fn file_length(&self) -> Option<u64>;
    fn app_info(&self) -> MediaType;

    /// Static CDN URL for direct download, bypassing host construction.
    fn static_url(&self) -> Option<&str> { None }

    /// Whether this media requires decryption.
    fn is_encrypted(&self) -> bool { self.media_key().is_some() }
}
```

And the macro that wires the proto fields to the trait (wacore `download.rs` lines 148-215):

```rust
impl_downloadable!(wa::message::ImageMessage,    MediaType::Image,    file_length, static_url);
impl_downloadable!(wa::message::VideoMessage,    MediaType::Video,    file_length, static_url);
impl_downloadable!(wa::message::DocumentMessage, MediaType::Document, file_length);
impl_downloadable!(wa::message::AudioMessage,    MediaType::Audio,    file_length);
impl_downloadable!(wa::message::StickerMessage,  MediaType::Sticker,  file_length);
impl_downloadable!(wa::message::StickerPackMessage, MediaType::StickerPack, file_length);
impl_downloadable!(ExternalBlobReference, MediaType::AppState, file_size_bytes);
impl_downloadable!(HistorySyncNotification, MediaType::History, file_length);
```

Note `mimetype` is metadata only (not used for the CDN request); the URL token derives from `file_enc_sha256` for encrypted media and `file_sha256` for unencrypted/newsletter media.

---

## 3. The download API

File: `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/download.rs`

Re-exports (lines 6-8):

```rust
pub use wacore::download::{
    DownloadUtils, Downloadable, MediaDecryption, MediaDecryptionError, MediaType,
};
```

### Two ways to download

**(a) From a message proto (anything implementing `Downloadable`)** — lines 235-253:

```rust
/// Downloads and decrypts media from WhatsApp's CDN into memory.
pub async fn download(&self, downloadable: &dyn Downloadable) -> Result<Vec<u8>> { ... }

pub async fn download_to_file<W: Write + Seek + Send + Unpin>(
    &self,
    downloadable: &dyn Downloadable,
    mut writer: W,
) -> Result<()> { ... }
```

Streaming variant (constant memory) — lines 339-351:

```rust
pub async fn download_to_writer<W: Write + Seek + Send + 'static>(
    &self,
    downloadable: &dyn Downloadable,
    writer: W,
) -> Result<W> { ... }
```

**(b) From raw params (no original message needed)** — `download_from_params`, lines 256-274:

```rust
/// Downloads and decrypts media from raw parameters without needing the original message.
pub async fn download_from_params(
    &self,
    direct_path: &str,
    media_key: &[u8],
    file_sha256: &[u8],
    file_enc_sha256: &[u8],
    file_length: u64,
    media_type: MediaType,
) -> Result<Vec<u8>> { ... }
```

Streaming raw-params variant — lines 355-375:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn download_from_params_to_writer<W: Write + Seek + Send + 'static>(
    &self,
    direct_path: &str,
    media_key: &[u8],
    file_sha256: &[u8],
    file_enc_sha256: &[u8],
    file_length: u64,
    media_type: MediaType,
    writer: W,
) -> Result<W> { ... }
```

### The descriptor/param struct it consumes

`download_from_params` builds an internal `DownloadParams` (whatsapp-rust `download.rs` lines 25-54) which implements `Downloadable`:

```rust
/// Implements `Downloadable` from raw media parameters.
struct DownloadParams {
    direct_path: String,
    media_key: Option<Vec<u8>>,
    file_sha256: Vec<u8>,
    file_enc_sha256: Option<Vec<u8>>,
    file_length: u64,
    media_type: MediaType,
}

impl Downloadable for DownloadParams {
    fn direct_path(&self) -> Option<&str> { Some(&self.direct_path) }
    fn media_key(&self) -> Option<&[u8]> { self.media_key.as_deref() }
    fn file_enc_sha256(&self) -> Option<&[u8]> { self.file_enc_sha256.as_deref() }
    fn file_sha256(&self) -> Option<&[u8]> { Some(&self.file_sha256) }
    fn file_length(&self) -> Option<u64> { Some(self.file_length) }
    fn app_info(&self) -> MediaType { self.media_type }
}
```

`DownloadParams` is private; you don't construct it directly. Either implement `Downloadable` yourself, pass a proto media message, or call `download_from_params(...)`.

### `MediaType` (the `media_type` arg) — wacore `download.rs` lines 31-47

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
    Audio,
    Document,
    History,
    AppState,
    Sticker,
    StickerPack,
    StickerPackThumbnail,
    LinkThumbnail,
    /// Product catalog image — unencrypted, uploads to `/product/image`.
    ProductCatalogImage,
}
```

URL construction (`DownloadUtils::prepare_download_requests`, wacore `download.rs` lines 235-299): per media host, `https://{host}{direct_path}?auth={auth}&token={token}` where `token = base64url(file_enc_sha256)` for encrypted media (else `base64url(file_sha256)`); decryption is AES-256-CBC + HMAC-SHA256 keyed by HKDF expansion of `media_key` with the per-type `app_info` salt.

---

## 4. Pairing-related event variants

File: `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/types/events.rs`

### `Event::PairingQrCode` (struct variant, events.rs lines 397-400)

```rust
PairingQrCode {
    code: String,
    timeout: std::time::Duration,
},
```

### `Event::PairingCode` (struct variant, events.rs lines 401-408)

```rust
/// Generated pair code for phone number linking.
/// User should enter this code on their phone in WhatsApp > Linked Devices.
PairingCode {
    /// The 8-character pairing code to display.
    code: String,
    /// Approximate validity duration (~180 seconds).
    timeout: std::time::Duration,
},
```

### `Event::PairSuccess(PairSuccess)` (events.rs lines 529-535)

```rust
#[derive(Debug, Clone, Serialize)]
pub struct PairSuccess {
    pub id: Jid,
    pub lid: Jid,
    pub business_name: String,
    pub platform: String,
}
```

### `Event::PairError(PairError)` (events.rs lines 537-544)

```rust
#[derive(Debug, Clone, Serialize)]
pub struct PairError {
    pub id: Jid,
    pub lid: Jid,
    pub business_name: String,
    pub platform: String,
    pub error: String,
}
```

### Related pairing-flow events

`QrScannedWithoutMultidevice` (unit struct, events.rs line 547):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct QrScannedWithoutMultidevice;
```

`LoggedOut` (events.rs lines 556-559):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct LoggedOut {
    pub on_connect: bool,
    pub reason: ConnectFailureReason,
}
```

---

## Usage sketch (incoming media)

```rust
if let Event::Message(msg, info) = &*event {
    // sender/chat: info.source.chat, info.source.sender, info.source.is_group
    if let Some(img) = msg.image_message.as_ref() {
        // img implements Downloadable directly:
        let bytes = client.download(img.as_ref()).await?;
        // or from raw params (img.direct_path, img.media_key, img.file_sha256,
        //   img.file_enc_sha256, img.file_length, MediaType::Image):
        // client.download_from_params(dp, mk, fsha, fesha, len, MediaType::Image).await?
    }
}
```

(`img` is `&Box<ImageMessage>`; the `Downloadable` impl is on `wa::message::ImageMessage`, so pass `img.as_ref()`.)