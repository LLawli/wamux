I have everything needed. Here is the complete verbatim API reference.

---

# whatsapp-rust 0.6.0 — Real Public API

**Critical architectural fact:** This crate is **low-level**. There are NO top-level `Client::send_text` / `send_image` / `send_video` / `send_audio` / `send_document` / `send_reaction` convenience methods for normal chats. You build a `waproto::whatsapp::Message` (aliased `wa::Message`, a protobuf struct with optional fields like `conversation`, `image_message`, `reaction_message`, etc.) and send it via `Client::send_message`. The only named `send_text`/`send_image`/`send_video` live on the `Status` sub-handle, and `send_reaction` only on `Newsletter`. Media (image/video/audio/document) all flow through `upload()` → construct `wa::Message` → `send_message()`.

## 1. Bot + BotBuilder

Source: `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/bot.rs`

It uses a **typestate builder** with 4 type params `<B, T, H, R>` (backend, transport_factory, http_client, runtime). `build()` only exists when all 4 are `Provided`. All four required setters take **generic / `Arc<dyn>` forms as noted below**.

```rust
// Typestate markers
pub struct Missing;
pub struct Provided;

impl Bot {
    pub fn builder() -> BotBuilder<Missing, Missing, Missing, Missing>;
    pub fn client(&self) -> Arc<Client>;            // accessor — works BEFORE and AFTER run()
    pub async fn run(&mut self) -> Result<BotHandle>; // anyhow::Result
}

pub struct BotBuilder<B = Missing, T = Missing, H = Missing, R = Missing> { /* ... */ }
```

Required-field setters (each flips one type param to `Provided`):

```rust
// with_backend takes an Arc<dyn Backend> (NOT generic)
impl<T, H, R> BotBuilder<Missing, T, H, R> {
    pub fn with_backend(self, backend: Arc<dyn Backend>) -> BotBuilder<Provided, T, H, R>;
}

// with_transport_factory is GENERIC over F, wraps in Arc internally
impl<B, H, R> BotBuilder<B, Missing, H, R> {
    pub fn with_transport_factory<F>(self, factory: F) -> BotBuilder<B, Provided, H, R>
    where
        F: crate::transport::TransportFactory + 'static;
}

// with_http_client is GENERIC over C, wraps in Arc internally
impl<B, T, R> BotBuilder<B, T, Missing, R> {
    pub fn with_http_client<C>(self, client: C) -> BotBuilder<B, T, Provided, R>
    where
        C: crate::http::HttpClient + 'static;
}

// with_runtime is GENERIC over Rt, wraps in Arc internally
impl<B, T, H> BotBuilder<B, T, H, Missing> {
    pub fn with_runtime<Rt: Runtime>(self, runtime: Rt) -> BotBuilder<B, T, H, Provided>;
}
```

Optional setters (available in any typestate, return `Self`):

```rust
impl<B, T, H, R> BotBuilder<B, T, H, R> {
    // on_event: F is a Fn taking (Arc<Event>, Arc<Client>) returning a Future<Output = ()>
    pub fn on_event<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(Arc<Event>, Arc<Client>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static;

    pub fn with_enc_handler<Eh>(mut self, enc_type: impl Into<String>, handler: Eh) -> Self
    where
        Eh: EncHandler + 'static;

    pub fn with_version(mut self, version: (u32, u32, u32)) -> Self;
    pub fn with_device_props(mut self, override_: DevicePropsOverride) -> Self;
    pub fn with_pair_code(mut self, options: PairCodeOptions) -> Self;
    pub fn skip_history_sync(mut self) -> Self;          // no arg, sets flag true
    pub fn with_push_name(mut self, name: impl Into<String>) -> Self;
    pub fn with_cache_config(mut self, config: CacheConfig) -> Self;
}

// build() only when all four are Provided. Returns BotBuilderError, NOT anyhow.
impl BotBuilder<Provided, Provided, Provided, Provided> {
    pub async fn build(self) -> std::result::Result<Bot, BotBuilderError>;
}
```

Note: there is **no `with_pair_code`-returning struct-only variant**; the closure arg types are exactly `(Arc<Event>, Arc<Client>)` and the closure body must produce `Future<Output = ()>`.

The `on_event` closure is stored internally as:
```rust
type EventHandlerCallback =
    Arc<dyn Fn(Arc<Event>, Arc<Client>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
```

`BotHandle` (returned by `run()`):
```rust
pub struct BotHandle { /* done_rx, _abort_handle */ }

impl BotHandle {
    pub fn abort(&self);  // aborts the run task
}

// BotHandle is itself awaitable:
impl std::future::Future for BotHandle {
    type Output = Result<(), futures::channel::oneshot::Canceled>;
    // resolves when the client run loop finishes
}
```

`BotBuilderError`:
```rust
#[derive(Debug, Error)]
pub enum BotBuilderError {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

`MessageContext` (convenience, built from an event; carries `Arc<Client>`):
```rust
#[derive(Clone)]
pub struct MessageContext {
    pub message: Arc<wa::Message>,
    pub info: MessageInfo,
    pub client: Arc<Client>,
}
impl MessageContext {
    pub fn from_parts(message: &wa::Message, info: &MessageInfo, client: Arc<Client>) -> Self;
    pub fn from_arc(message: Arc<wa::Message>, info: &MessageInfo, client: Arc<Client>) -> Self;
    pub fn from_event(event: &Event, client: Arc<Client>) -> Option<Self>;
    pub async fn send_message(&self, message: wa::Message) -> Result<crate::send::SendResult, anyhow::Error>;
    pub fn build_quote_context(&self) -> wa::ContextInfo;
    pub fn message_key(&self) -> wa::MessageKey;
    pub async fn edit_message(&self, original_message_id: impl Into<String>, new_message: wa::Message) -> Result<String, anyhow::Error>;
    pub async fn revoke_message(&self, message_id: String, revoke_type: crate::send::RevokeType) -> Result<(), anyhow::Error>;
}
```

## 2. Obtaining `Arc<Client>`

Source: `bot.rs:191`

```rust
pub fn client(&self) -> Arc<Client>;   // on Bot
```

There is **no `bot.client` public field** (it is private). Use the `bot.client()` accessor. It works both **before and after** `run()` (it just clones the internal `Arc<Client>`). The same `Arc<Client>` is also handed to your `on_event` closure as the second argument and lives in `MessageContext::client`.

## 3. Client async methods

### Sending / editing / revoking — `src/send.rs` + `src/client.rs`

`/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/send.rs`
```rust
impl Client {
    pub async fn send_message(
        &self,
        to: Jid,
        message: wa::Message,
    ) -> Result<SendResult, anyhow::Error>;

    pub async fn send_message_with_options(
        &self,
        to: Jid,
        mut message: wa::Message,
        options: SendOptions,
    ) -> Result<SendResult, anyhow::Error>;

    // revoke == "delete for everyone"
    pub async fn revoke_message(
        &self,
        to: Jid,
        message_id: impl Into<String>,
        revoke_type: RevokeType,
    ) -> Result<(), anyhow::Error>;

    pub async fn pin_message(
        &self,
        chat: Jid,
        key: wa::MessageKey,
        duration: PinDuration,
    ) -> Result<(), anyhow::Error>;

    pub async fn unpin_message(&self, chat: Jid, key: wa::MessageKey) -> Result<(), anyhow::Error>;
}
```

`/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/client.rs:3532`
```rust
impl Client {
    pub async fn edit_message(
        &self,
        to: Jid,
        original_id: impl Into<String>,
        new_content: wa::Message,
    ) -> Result<String, anyhow::Error>;

    // src/client.rs:3678
    pub async fn get_push_name(&self) -> String;

    // src/client.rs:2010 — NOTE: this is "get_business_profile" returning typed Option
    pub async fn get_business_profile(
        &self,
        jid: &wacore_binary::Jid,
    ) -> Result<Option<wacore::iq::business::BusinessProfile>, crate::request::IqError>;
}
```

**send_text / send_image / send_video / send_audio / send_document / send_reaction (normal chats): DO NOT EXIST as Client methods.** Build a `wa::Message` and call `send_message`. Reactions: construct `wa::Message { reaction_message: Some(wa::message::ReactionMessage { key: Some(target_key), text: Some("❤".into()), .. }), .. }` then `send_message`. There is no public `build_reaction` helper.

### "delete for me" / mark read / chat actions — `src/features/chat_actions.rs`

`/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/features/chat_actions.rs`. Accessed via `client.chat_actions()` returning `ChatActions<'_>`:
```rust
impl Client {
    pub fn chat_actions(&self) -> ChatActions<'_>;   // src/features/chat_actions.rs:567
}

impl<'a> ChatActions<'a> {
    pub async fn archive_chat(&self, jid: &Jid, message_range: Option<SyncActionMessageRange>) -> Result<()>;
    pub async fn unarchive_chat(&self, jid: &Jid, message_range: Option<SyncActionMessageRange>) -> Result<()>;
    pub async fn pin_chat(&self, jid: &Jid) -> Result<()>;
    pub async fn unpin_chat(&self, jid: &Jid) -> Result<()>;
    pub async fn mute_chat(&self, jid: &Jid) -> Result<()>;
    pub async fn mute_chat_until(&self, jid: &Jid, mute_end_timestamp_ms: i64) -> Result<()>;
    pub async fn unmute_chat(&self, jid: &Jid) -> Result<()>;
    pub async fn star_message(/* ... */) -> Result<()>;
    pub async fn unstar_message(/* ... */) -> Result<()>;

    pub async fn mark_chat_as_read(
        &self,
        jid: &Jid,
        read: bool,
        message_range: Option<SyncActionMessageRange>,
    ) -> Result<()>;

    pub async fn delete_chat(
        &self,
        jid: &Jid,
        delete_media: bool,
        message_range: Option<SyncActionMessageRange>,
    ) -> Result<()>;

    // "delete message for me" (local only, not for everyone)
    pub async fn delete_message_for_me(
        &self,
        chat_jid: &Jid,
        participant_jid: Option<&Jid>,
        message_id: &str,
        from_me: bool,
        delete_media: bool,
        message_timestamp: Option<i64>,
    ) -> Result<()>;
}
```
(`Result<T>` here is `anyhow::Result<T>`.)

### Chat state (typing/recording) — `src/features/chatstate.rs`

Accessed via `client.chatstate()` returning `Chatstate<'_>`:
```rust
impl Client {
    pub fn chatstate(&self) -> Chatstate<'_>;   // src/features/chatstate.rs:73
}

pub enum ChatStateType { Composing, Recording, Paused }

impl<'a> Chatstate<'a> {
    pub async fn send(&self, to: &Jid, state: ChatStateType) -> Result<(), crate::client::ClientError>;
    pub async fn send_composing(&self, to: &Jid) -> Result<(), crate::client::ClientError>;
    pub async fn send_recording(&self, to: &Jid) -> Result<(), crate::client::ClientError>;
    pub async fn send_paused(&self, to: &Jid)    -> Result<(), crate::client::ClientError>;
}
```

### Presence (available/unavailable/subscribe) — `src/features/presence.rs`

Accessed via `client.presence()` returning `Presence<'_>`:
```rust
impl Client {
    pub fn presence(&self) -> Presence<'_>;   // src/features/presence.rs:237
}

pub enum PresenceStatus { Available, Unavailable }

impl<'a> Presence<'a> {
    pub async fn set(&self, status: PresenceStatus) -> Result<(), PresenceError>;
    pub async fn set_available(&self)   -> Result<(), PresenceError>;
    pub async fn set_unavailable(&self) -> Result<(), PresenceError>;
    pub async fn subscribe(&self, jid: &Jid)   -> Result<(), anyhow::Error>;   // subscribe to a contact's presence
    pub async fn unsubscribe(&self, jid: &Jid) -> Result<(), anyhow::Error>;
}
```

### Groups — `src/features/groups.rs`

Accessed via `client.groups()` returning `Groups<'_>`. `Result<T>` = `Result<T, anyhow::Error>` unless noted:
```rust
impl Client {
    pub fn groups(&self) -> Groups<'_>;   // src/features/groups.rs:871
}

impl<'a> Groups<'a> {
    pub async fn query_info(&self, jid: &Jid) -> Result<GroupInfo, anyhow::Error>;
    pub async fn get_participating(&self) -> Result<HashMap<String, GroupMetadata>, anyhow::Error>;
    pub async fn get_metadata(&self, jid: &Jid) -> Result<GroupMetadata, anyhow::Error>;

    pub async fn create_group(&self, mut options: GroupCreateOptions) -> Result<CreateGroupResult, anyhow::Error>;

    // set_subject takes a validated GroupSubject newtype, NOT &str
    pub async fn set_subject(&self, jid: &Jid, subject: GroupSubject) -> Result<(), anyhow::Error>;

    // there is NO set_topic; "topic"/"description" == set_description
    pub async fn set_description(
        &self,
        jid: &Jid,
        description: Option<GroupDescription>,
        prev: Option<String>,
    ) -> Result<(), anyhow::Error>;

    pub async fn leave(&self, jid: &Jid) -> Result<(), anyhow::Error>;

    pub async fn add_participants(&self, jid: &Jid, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, anyhow::Error>;
    pub async fn remove_participants(&self, jid: &Jid, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, anyhow::Error>;
    pub async fn promote_participants(&self, jid: &Jid, participants: &[Jid]) -> Result<(), anyhow::Error>;
    pub async fn demote_participants(&self, jid: &Jid, participants: &[Jid]) -> Result<(), anyhow::Error>;

    // get_invite_link(jid, reset): pass reset=true to revoke+regenerate the link
    pub async fn get_invite_link(&self, jid: &Jid, reset: bool) -> Result<String, anyhow::Error>;

    pub async fn set_locked(&self, jid: &Jid, locked: bool) -> Result<(), anyhow::Error>;
    pub async fn set_announce(&self, jid: &Jid, announce: bool) -> Result<(), anyhow::Error>;
    pub async fn set_ephemeral(&self, jid: &Jid, expiration: u32) -> Result<(), anyhow::Error>;
    pub async fn set_membership_approval(&self, jid: &Jid, mode: MembershipApprovalMode) -> Result<(), anyhow::Error>;

    // join via invite code (string or full link both accepted)
    pub async fn join_with_invite_code(&self, code: &str) -> Result<JoinGroupResult, anyhow::Error>;
    pub async fn join_with_invite_v4(&self, group_jid: &Jid, code: &str, expiration: i64, admin_jid: &Jid) -> Result<JoinGroupResult, anyhow::Error>;

    pub async fn get_invite_info(&self, code: &str) -> Result<GroupMetadata, anyhow::Error>;
    pub async fn get_membership_requests(&self, jid: &Jid) -> Result<Vec<MembershipRequest>, anyhow::Error>;
    pub async fn approve_membership_requests(&self, jid: &Jid, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, anyhow::Error>;
    pub async fn reject_membership_requests(&self, jid: &Jid, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, anyhow::Error>;
    pub async fn batch_get_info(/* ... */) -> /* ... */;            // src:672
    pub async fn get_profile_pictures(/* ... */) -> /* ... */;       // src:697
    pub async fn revoke_request_code(/* ... */) -> /* ... */;        // src:655 (revoke invite request code)
    pub async fn acknowledge(&self, jid: &Jid) -> Result<(), anyhow::Error>;
    pub async fn set_limit_sharing(&self, jid: &Jid, enabled: bool) -> Result<(), MexError>;
    // plus set_member_add_mode, set_member_link_mode, set_member_share_history_mode,
    //      set_group_history, set_no_frequently_forwarded, set_allow_admin_reports,
    //      cancel_membership_requests, update_member_label
}
```
There is **no separate "revoke invite link" method**; revoking is `get_invite_link(jid, reset = true)`.

### Profile (own account) — `src/features/profile.rs`

Accessed via `client.profile()` returning `Profile<'_>`. `Result<T>` = `anyhow::Result<T>`:
```rust
impl Client {
    pub fn profile(&self) -> Profile<'_>;   // src/features/profile.rs:169
}

impl<'a> Profile<'a> {
    pub async fn set_status_text(&self, text: &str) -> Result<()>;
    pub async fn set_push_name(&self, name: &str) -> Result<()>;     // SET push name (own)
    pub async fn set_profile_picture(&self, image_data: Vec<u8>) -> Result<SetProfilePictureResponse>;
    pub async fn remove_profile_picture(&self) -> Result<SetProfilePictureResponse>;
}
```
(To GET own push name use `Client::get_push_name(&self) -> String` from client.rs.)

### Contacts (is_on_whatsapp / profile picture / user info) — `src/features/contacts.rs`

Accessed via `client.contacts()` returning `Contacts<'_>`. `Result<T>` = `anyhow::Result<T>`:
```rust
impl Client {
    pub fn contacts(&self) -> Contacts<'_>;   // src/features/contacts.rs:189
}

impl<'a> Contacts<'a> {
    pub async fn is_on_whatsapp(&self, jids: &[Jid]) -> Result<Vec<IsOnWhatsAppResult>>;
    pub async fn get_profile_picture(&self, jid: &Jid, preview: bool) -> Result<Option<ProfilePicture>>;
    pub async fn get_user_info(&self, jids: &[Jid]) -> Result<HashMap<Jid, UserInfo>>;
}
```

### Newsletter `send_reaction` (the ONLY named send_reaction) — `src/features/newsletter.rs:372`
```rust
pub async fn send_reaction(/* ... */)   // newsletter-scoped; for normal chats build wa::Message{ reaction_message }
```

## 4. Media: upload / download

### Upload — `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/upload.rs`

```rust
impl Client {
    pub async fn upload(
        &self,
        data: Vec<u8>,
        media_type: MediaType,        // wacore::download::MediaType
        options: UploadOptions,
    ) -> Result<UploadResponse>;       // anyhow::Result
}

#[derive(Debug, Clone)]
pub struct UploadResponse {
    pub url: String,
    pub direct_path: String,
    pub media_key: [u8; 32],
    pub file_enc_sha256: [u8; 32],
    pub file_sha256: [u8; 32],
    pub file_length: u64,
    pub media_key_timestamp: i64,
}
impl UploadResponse {
    pub fn media_key_vec(&self) -> Vec<u8>;
    pub fn file_sha256_vec(&self) -> Vec<u8>;
    pub fn file_enc_sha256_vec(&self) -> Vec<u8>;
}

#[non_exhaustive]
#[derive(Default, Clone)]
pub struct UploadOptions {
    pub media_key: Option<[u8; 32]>,
}
impl UploadOptions {
    pub fn new() -> Self;
    pub fn with_media_key(mut self, key: [u8; 32]) -> Self;
}
```

### Download — `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/download.rs`

```rust
impl Client {
    pub async fn download(&self, downloadable: &dyn Downloadable) -> Result<Vec<u8>>;

    pub async fn download_to_file<W: Write + Seek + Send + Unpin>(
        &self,
        downloadable: &dyn Downloadable,
        mut writer: W,
    ) -> Result<()>;

    pub async fn download_from_params(
        &self,
        direct_path: &str,
        media_key: &[u8],
        file_sha256: &[u8],
        file_enc_sha256: &[u8],
        file_length: u64,
        media_type: MediaType,
    ) -> Result<Vec<u8>>;

    pub async fn download_to_writer<W: Write + Seek + Send + 'static>(
        &self,
        downloadable: &dyn Downloadable,
        writer: W,
    ) -> Result<W>;

    pub async fn download_from_params_to_writer<W: Write + Seek + Send + 'static>(
        &self,
        direct_path: &str,
        media_key: &[u8],
        file_sha256: &[u8],
        file_enc_sha256: &[u8],
        file_length: u64,
        media_type: MediaType,
        writer: W,
    ) -> Result<W>;
}
```
(`Result<T>` = `anyhow::Result<T>`.)

`MediaType` — `wacore::download::MediaType` (re-exported through `upload`/`download`):
```rust
pub enum MediaType {
    Image, Video, Audio, Document, History, AppState,
    Sticker, StickerPack, StickerPackThumbnail, LinkThumbnail, ProductCatalogImage,
}
```

`Downloadable` trait (the `wa::message::ImageMessage`/`VideoMessage`/`AudioMessage`/`DocumentMessage` proto types implement it, so you pass `&msg.image_message.unwrap()` etc.):
```rust
pub trait Downloadable: Sync + Send {
    fn direct_path(&self) -> Option<&str>;
    fn media_key(&self) -> Option<&[u8]>;
    fn file_enc_sha256(&self) -> Option<&[u8]>;
    fn file_sha256(&self) -> Option<&[u8]>;
    fn file_length(&self) -> Option<u64>;
    fn app_info(&self) -> MediaType;
    fn static_url(&self) -> Option<&str> { None }
    fn is_encrypted(&self) -> bool { self.media_key().is_some() }
}
```

## 4b. Message-building types & result types

The "message content" type is the protobuf `waproto::whatsapp::Message` (re-exported; idiomatically `use waproto::whatsapp as wa;` then `wa::Message`). There is no custom `MessageContent` builder enum — you populate the proto struct's optional fields directly (e.g. `conversation`, `extended_text_message`, `image_message`, `video_message`, `audio_message`, `document_message`, `reaction_message`). Example of constructing an image message is in `status.rs` (shown below).

`SendResult` / `SendOptions` / `RevokeType` / `PinDuration` — `src/send.rs` (re-exported at crate root via `pub use send::{PinDuration, RevokeType, SendOptions, SendResult}`):
```rust
#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    pub message_id: Option<String>,
    pub extra_stanza_nodes: Vec<Node>,
    pub ephemeral_expiration: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendResult {
    pub message_id: String,
    pub to: Jid,
}
impl SendResult {
    pub fn message_key(&self) -> wa::MessageKey;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PinDuration { Hours24, #[default] Days7, Days30 }

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RevokeType {
    #[default] Sender,
    Admin { original_sender: Jid },
}
```
There is **no `SendResponse`** type — it is `SendResult`. (Note `lib.rs` also re-exports `SendResult` from `send`; do not confuse with `wacore`'s `SendResult`.)

Group/contact/profile types (`src/features/groups.rs`, `wacore`):
```rust
// src/features/groups.rs
#[derive(Debug, Clone)]
pub struct CreateGroupResult { pub metadata: GroupMetadata }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupParticipant {
    pub jid: Jid,
    pub phone_number: Option<Jid>,
    pub participant_type: ParticipantType,
}
// GroupMetadata: large struct (id, subject, participants: Vec<GroupParticipant>, creator, ... many fields)

// wacore::iq::groups
#[derive(Debug, Clone, TypedBuilder)]
#[builder(build_method(into))]
pub struct GroupCreateOptions {
    pub subject: String,
    pub participants: Vec<GroupParticipantOptions>,
    pub member_link_mode: Option<MemberLinkMode>,
    pub member_add_mode: Option<MemberAddMode>,
    pub membership_approval_mode: Option<MembershipApprovalMode>,
    pub ephemeral_expiration: Option<u32>,
    pub is_parent: bool,
    pub closed: bool,
    pub allow_non_admin_sub_group_creation: bool,
    pub create_general_chat: bool,
}
impl GroupCreateOptions { pub fn new(subject: impl Into<String>) -> Self; }

#[derive(Debug, Clone, TypedBuilder)]
#[builder(build_method(into))]
pub struct GroupParticipantOptions {
    pub jid: Jid,
    pub phone_number: Option<Jid>,
    pub privacy: Option<Vec<u8>>,
}
impl GroupParticipantOptions {
    pub fn new(jid: Jid) -> Self;
    pub fn from_phone(phone_number: Jid) -> Self;
    pub fn from_lid_and_phone(lid: Jid, phone_number: Jid) -> Self;
    pub fn with_phone_number(mut self, phone_number: Jid) -> Self;
    pub fn with_privacy(mut self, privacy: Vec<u8>) -> Self;
}

// GroupSubject / GroupDescription are validated-string newtypes (max-len checked):
pub struct GroupSubject(/* validated String */);     // via define_validated_string!
pub struct GroupDescription(/* validated String */);

pub enum MembershipApprovalMode { Off, On }
pub enum ParticipantType { Member, Admin, SuperAdmin }

// wacore::iq::contacts
pub struct ProfilePicture {
    pub id: String,
    pub url: String,
    pub direct_path: Option<String>,
    pub hash: Option<String>,
}
pub struct SetProfilePictureResponse { pub id: String }

// wacore::iq::usync
pub struct IsOnWhatsAppResult {
    pub jid: Jid,
    pub lid: Option<Jid>,
    pub pn_jid: Option<Jid>,
    pub is_registered: bool,
    pub is_business: bool,
}
pub struct UserInfo {
    pub jid: Jid,
    pub lid: Option<Jid>,
    pub status: Option<String>,
    pub picture_id: Option<String>,
    pub is_business: bool,
}
```

Canonical example of building a media `wa::Message` from an `UploadResponse` (from `src/features/status.rs:74`, applies the same to normal chats):
```rust
let message = wa::Message {
    image_message: Some(Box::new(wa::message::ImageMessage {
        url: Some(upload.url),
        direct_path: Some(upload.direct_path),
        media_key: Some(upload.media_key.to_vec()),
        file_sha256: Some(upload.file_sha256.to_vec()),
        file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
        file_length: Some(upload.file_length),
        mimetype: Some("image/jpeg".to_string()),
        jpeg_thumbnail: Some(thumbnail),
        caption: caption.map(|c| c.to_string()),
        ..Default::default()
    })),
    ..Default::default()
};
// then: client.send_message(chat_jid, message).await?;
```

## 5. PairCodeOptions + pair_with_code

`PairCodeOptions` is defined in `wacore::pair_code` (re-exported as `whatsapp_rust::pair_code::PairCodeOptions`):

`/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/pair_code.rs:113`
```rust
pub struct PairCodeOptions {
    pub phone_number: String,
    pub show_push_notification: bool,
    pub custom_code: Option<String>,
    pub platform_id: Option<CompanionWebClientType>,
}
// Has #[derive(Default)] — used as `PairCodeOptions { phone_number, ..Default::default() }`
```

`pair_with_code` — `/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/whatsapp-rust-0.6.0/src/pair_code.rs:111`. Note the **`self: &Arc<Self>`** receiver (must be called on an `Arc<Client>`, e.g. `bot.client().pair_with_code(...)`):
```rust
impl Client {
    pub async fn pair_with_code(
        self: &Arc<Self>,
        options: PairCodeOptions,
    ) -> Result<String, PairError>;   // returns the 8-char code
}

#[derive(Debug, thiserror::Error)]
pub enum PairError {
    #[error(transparent)]
    PairCode(#[from] PairCodeError),
    #[error("pair-code IQ request failed")]
    RequestFailed(#[from] IqError),
}
```

## Bonus — the `Event` enum (the `on_event` first arg, `Arc<Event>`)

`/var/home/luka/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wacore-0.6.0/src/types/events.rs:391` (re-exported as `whatsapp_rust::types::events::Event`):
```rust
pub enum Event {
    Connected(Connected),
    Disconnected(Disconnected),
    PairSuccess(PairSuccess),
    PairError(PairError),
    LoggedOut(LoggedOut),
    PairingQrCode { code: String, timeout: std::time::Duration },
    PairingCode  { code: String, timeout: std::time::Duration },
    QrScannedWithoutMultidevice(QrScannedWithoutMultidevice),
    ClientOutdated(ClientOutdated),
    Message(Arc<wa::Message>, Arc<MessageInfo>),
    // ... more variants follow
}
impl Event {
    pub fn as_message(&self) -> Option<(&Arc<wa::Message>, &MessageInfo)>;
    pub fn message_text(&self) -> Option<&str>;
}

pub trait EventHandler: Send + Sync {
    fn handle_event(&self, event: Arc<Event>);
}
```