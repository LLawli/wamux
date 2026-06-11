# Breaking changes para a borda (wamux-http-edge) — 2026-06-11

Consolidação de TODAS as mudanças de contrato do core desta rodada de
code-review (correções críticas + lote de contrato). Tudo entra junto na
working tree de hoje; a borda deve tratar este documento como o delta completo
em relação ao contrato do fim do Sprint 5.

Arquivos .proto alterados: `proto/events.proto` e `proto/account.proto`
(texto integral na seção final). Os demais (`messaging.proto`, `common.proto`,
`media.proto`, `groups.proto`, `contacts.proto`, `admin.proto`) não mudaram
nesta rodada.

Resumo por impacto:

| # | Mudança | Impacto na borda |
|---|---------|------------------|
| 1 | `ReceiptEvent.type` agora em tokens minúsculos | **BREAKING** (valores de string) |
| 2 | `PresenceUpdate.chat_state` em minúsculas + novo valor `recording` | **BREAKING** (valores de string) |
| 3 | `PairedInfo`: `push_name` removido; `business_name`, `lid`, `platform` adicionados | **BREAKING** (codegen + semântica) |
| 4 | Gap do canal de criação virou `kind="subscription_gap"` | **BREAKING** (dispatch de gap) |
| 5 | Replay sem duplicatas + replay full-ring para contas criadas pós-subscribe | Comportamental (a favor da borda) |
| 6 | `SendMediaHeader.quote`/`mentions` agora são honrados | Comportamental (campos antes ignorados passam a ter efeito) |
| 7 | Erros IQ: trailers `wa-code`/`wa-text` estruturados | Aditivo (migrar do parse de prosa) |
| 8 | Wire WhatsApp: sem ContextInfo vazio; participant de quote em DM ausente em vez de `""` | Invisível na API do socket |

---

## 1. `ReceiptEvent.type`: tokens minúsculos (BREAKING)

**Antes:** o core emitia o Debug do Rust: `"Read"`, `"Delivered"`,
`"Played"`, e até `"Other(\"...\")"` para tipos desconhecidos. O comentário do
proto prometia `delivered|read|played` e nunca foi verdade.

**Agora:** tokens minúsculos estáveis, exatamente os documentados no proto:

```
delivered | read | played | read-self | played-self | sender | retry |
enc_rekey_retry | server-error | inactive | peer_msg | hist_sync
```

Tipo desconhecido relaya o atributo bruto da stanza verbatim (sem wrapper
`Other(...)`).

**Migração:** trocar qualquer match/comparação de `"Read"`/`"Delivered"` para
os tokens minúsculos. Se a borda já normalizava com lowercase, conferir os
hifenizados (`read-self`, `server-error`) que antes vinham como `ReadSelf`/
`ServerError`.

## 2. `PresenceUpdate.chat_state`: minúsculas + `recording` (BREAKING)

**Antes:** `"Composing"` ou `"Paused"` (Debug). "Gravando áudio" era
indistinguível de "digitando" (a lib modela recording como Composing com
media=Audio, e o Debug ignorava o media).

**Agora:** `composing` | `recording` | `paused` | `""` (string vazia continua
sendo "presença sem chat state", vinda de PresenceUpdate online/offline).

Bônus de simetria: esses são os mesmos tokens que `SendPresenceRequest.state`
aceita, então a borda pode espelhar o valor observado de volta sem traduzir.

**Migração:** trocar `"Composing"` → `"composing"`, `"Paused"` → `"paused"`,
e tratar o novo `"recording"` (antes ele chegava como `"Composing"`).

## 3. `PairedInfo` reformulado (BREAKING)

**Antes:**
```proto
message PairedInfo {
  Jid jid = 1;
  string push_name = 2;   // na prática carregava o business_name!
}
```

**Agora:**
```proto
message PairedInfo {
  Jid jid = 1;
  reserved 2; reserved "push_name";
  string business_name = 3;   // vazio para contas pessoais
  Jid lid = 4;                // identidade LID (hidden-PN) da conta
  string platform = 5;        // ex.: "android", "smba", "iphone"
}
```

**Por quê:** o evento PairSuccess da lib não carrega push name nenhum; o campo
antigo entregava o nome comercial sob um nome enganoso (e string vazia para
toda conta pessoal). Push names reais chegam depois, via eventos
`PushNameUpdate` (e `Account.push_name`/`AccountStatus.push_name`, que não
mudaram).

**Migração:** regenerar o cliente a partir dos protos; qualquer leitura de
`paired.push_name` quebra em compile/codegen (intencional). Usar
`business_name` se o que se queria era o nome comercial; para push name,
consumir `PushNameUpdate`. `lid` e `platform` são informação nova que antes
era descartada.

## 4. Gap do canal de criação: `kind="subscription_gap"` (BREAKING)

**Antes:** os dois tipos de gap usavam `RawEvent.kind="gap"`, distinguíveis só
pelo `account_uuid` vazio e pelo texto da nota. Uma borda que seguisse o
contrato documentado ("gap → resync via GetAccountStatus") chamaria
`GetAccountStatus("")` e receberia InvalidArgument.

**Agora:** dois kinds reservados, documentados no proto (`RawEvent`):

- `kind="gap"`: o assinante de UMA conta lagou o buffer; `account_uuid`
  preenchido; recuperação = `GetAccountStatus` daquela conta.
- `kind="subscription_gap"`: o canal de contas-criadas do stream all-accounts
  lagou (burst de >64 creates); `account_uuid` vazio; recuperação =
  re-assinar ou reconciliar via `ListAccounts`.

Ambos mantêm `monotonic_seq = -1` e payload vazio. A nota continua existindo,
mas é prosa para humanos, não contrato.

**Migração:** dispatch de gaps por `kind`, nunca por nota nem por uuid vazio.
O `subscription_gap` só existe desde o Sprint 5; se a borda ainda não trata o
gap de all-accounts, basta implementar direto no kind novo.

## 5. Semântica de replay (comportamental, a favor da borda)

Duas garantias novas, documentadas em `SubscribeRequest.replay_from_ring`:

- **Sem duplicatas dentro de um stream.** Antes, um evento publicado na
  janela entre o subscribe interno e o snapshot do ring chegava duas vezes
  (replay + live) com o mesmo `monotonic_seq`. Agora o core filtra o overlap.
  Borda que já dedupava por `(account_uuid, monotonic_seq)` continua correta
  e simplesmente nunca mais verá duplicata.
- **Conta criada DEPOIS do subscribe all-accounts sempre replaya o ring
  inteiro, mesmo com `replay_from_ring=0`.** "Live only" não se aplica a
  eventos emitidos antes do forwarder anexar; sem isso, o QR de pareamento de
  uma conta recém-criada podia se perder. Efeito visível: um stream live-only
  passa a receber os primeiros eventos de contas novas que antes sumiam.
  (Exceção de config: `ring_capacity=0` desliga o ring e a janela volta a
  existir.)

**Migração:** nenhuma obrigatória. Se a borda tinha workaround de
"re-assinar com replay>=1 depois de CreateAccount", pode remover.

## 6. `SendMediaHeader.quote` e `mentions` agora têm efeito (comportamental)

**Antes:** os campos 7/8 do `SendMediaHeader` existiam no proto mas eram
silenciosamente descartados: mídia saía sem balão de resposta e sem menção,
com SendResult OK.

**Agora:** quote + mentions + ephemeral compõem num único ContextInfo, com a
mesma semântica do caminho de texto (incluindo a precedência
`QuoteContext.participant` > `quoted.participant` e o mapeamento de string
vazia para campo ausente).

**Migração:** nenhuma mudança de chamada. Mas atenção: se a borda já enviava
esses campos achando que funcionavam, eles passam a funcionar de verdade
agora; se ela os preenchia com lixo "porque não fazia diferença", revisar.

## 7. Erros IQ: metadata estruturada `wa-code`/`wa-text` (aditivo)

Toda rejeição IQ do servidor WhatsApp (`WaServer`) agora carrega trailers
gRPC além do Status:

- `wa-code`: o código IQ bruto (ex.: `403`, `409`), sempre presente.
- `wa-text`: o texto da stanza (ex.: `forbidden`), presente quando for ASCII
  válido.

O mapeamento código→Status não mudou (400→InvalidArgument,
401→Unauthenticated, 403→PermissionDenied, 404→NotFound,
429→ResourceExhausted, resto→Unavailable), e a mensagem em prosa
(`whatsapp server rejected the request: code=..., text='...'`) continua igual
por compatibilidade.

**Migração recomendada:** trocar qualquer regex sobre a mensagem
(`code=(\d+)`) pela leitura dos trailers. A prosa passa a ser considerada
livre para mudar; os trailers são o contrato. Em especial, códigos não
mapeados (405/406/409/500) que colapsam em Unavailable agora são
distinguíveis de queda de conexão pelo `wa-code` (erro de conexão genérico
não carrega trailer nenhum).

## 8. Wire WhatsApp (sem impacto na API do socket)

Mudanças no que o core põe no fio para o WhatsApp, não no gRPC:

- Texto só com link_preview não emite mais um ContextInfo presente-porém-vazio
  (forma não canônica que nenhum cliente oficial produz).
- Quote em DM (participants vazios) emite o campo participant AUSENTE em vez
  de `Some("")` (JID vazio inválido).
- Recibos/presence/pareamento: nada mudou no fio, só na representação gRPC
  (itens 1 a 3).

## Mudanças internas sem contrato (FYI)

- Forwarders de eventos agora encerram quando o cliente desconecta
  (`tx.closed()`), eliminando vazamento de tasks/handles de contas ociosas ou
  deletadas em streams longos.
- `MediaKind` interno único deriva upload e sub-message (impossível mislabel
  de mídia por drift de matches).
- `scripts/ci.sh`: aceita DATABASE_URL sem porta explícita e falha se um gate
  filtrado rodar 0 testes.
- Adiado de propósito: capacidade 64 do canal created_tx (ver
  `docs/DEFERRED.md`).

---

## proto/events.proto (novo, integral)

```proto
syntax = "proto3";

package wamux.v1;

import "common.proto";
import "account.proto";

message SubscribeRequest {
  oneof selector {
    AccountRef account = 1;
    // "all" is DYNAMIC: it includes accounts created after subscribing, so the
    // stream stays open indefinitely. Under an extreme burst of creates the
    // core may emit a `subscription_gap` RawEvent meaning some newly created
    // accounts are missing from the stream: re-subscribe or reconcile via
    // ListAccounts.
    Empty all_accounts = 2;
  }
  // 0 = live only; N = replay up to N buffered events for quick reconnect.
  // No duplicates within one stream: live events already delivered by the
  // replay are skipped (filtered by monotonic_seq). Accounts created AFTER an
  // all-accounts subscribe always replay their full ring regardless of N —
  // "live only" cannot apply to events emitted before their forwarder
  // attached, so e.g. a just-created account's pairing QR is never lost.
  uint32 replay_from_ring = 3;
}

message InboundMessage {
  MessageKey key = 1;
  string chat = 2;
  string sender = 3;
  int64 timestamp = 4;
  string push_name = 5;
  string text = 6;
  QuoteContext quote = 7;
  repeated Mention mentions = 8;
  MediaDescriptor media = 9;        // set for media messages
  string caption = 10;
  string reaction = 11;             // set for reaction messages
  MessageKey reaction_target = 12;
  bool is_edit = 13;
  bool is_delete = 14;
  bytes raw_message = 20;           // serialized wa.Message for advanced consumers
}

message ReceiptEvent {
  string chat = 1;
  string sender = 2;
  repeated string message_ids = 3;
  // Lowercase tokens: delivered|read|played plus the rarer read-self|
  // played-self|sender|retry|enc_rekey_retry|server-error|inactive|peer_msg|
  // hist_sync; an unknown type relays the raw stanza attribute verbatim.
  string type = 4;
  int64 timestamp = 5;
}

message UndecryptableEvent {
  string chat = 1;
  string sender = 2;
  string reason = 3;
}

message ConnectionStateChanged {
  ConnectionState state = 1;
  string detail = 2;
}

message PresenceUpdate {
  string jid = 1;
  bool online = 2;
  int64 last_seen = 3;
  string chat_state = 4;            // composing|recording|paused|""
}

message GroupUpdate {
  string group_jid = 1;
  string kind = 2;
  bytes raw = 3;
}

message PushNameUpdate {
  string jid = 1;
  string push_name = 2;
}

message ContactUpdate {
  string jid = 1;
  string kind = 2;
  bytes raw = 3;
}

// History-sync chunk, only emitted when the account connected with backfill
// enabled (ConnectAccountRequest.backfill_history) or in answer to
// FetchMessageHistory. The core relays the blob verbatim: the edge decodes
// `raw` (a `wa.HistorySync` protobuf) itself. Delivered in chunks.
message HistorySyncEvent {
  int32 sync_type = 1;            // wa.HistorySync.HistorySyncType (InitialBootstrap|Recent|PushName|OnDemand|...)
  optional uint32 chunk_order = 2;
  optional uint32 progress = 3;   // 0-100
  // Set for on-demand syncs: correlate with FetchMessageHistoryResponse.session_id.
  optional string session_id = 4;
  bytes raw = 20;                 // decompressed `wa.HistorySync` protobuf
}

// Catch-all so the edge never silently loses an event type or a broadcast gap.
// Reserved kinds (both carry monotonic_seq -1 and an empty payload):
//   "gap"              one account's subscriber lagged its broadcast buffer;
//                      account_uuid is set: resync it via GetAccountStatus.
//   "subscription_gap" the all-accounts created-channel lagged; account_uuid
//                      is empty: re-subscribe or reconcile via ListAccounts.
// Any other kind is a forward-compat relay of a lib event (payload = JSON).
message RawEvent {
  string kind = 1;
  bytes payload = 2;
  string note = 3;
}

message EventEnvelope {
  string account_uuid = 1;
  int64 monotonic_seq = 2;
  int64 ts_unix_ms = 3;
  oneof event {
    InboundMessage message = 10;
    ReceiptEvent receipt = 11;
    UndecryptableEvent undecryptable = 12;
    ConnectionStateChanged connection = 13;
    PairingUpdate pairing = 14;
    PresenceUpdate presence = 15;
    GroupUpdate group = 16;
    PushNameUpdate push_name = 17;
    ContactUpdate contact = 18;
    HistorySyncEvent history_sync = 19;
    RawEvent raw = 99;
  }
}

service EventService {
  rpc SubscribeEvents(SubscribeRequest) returns (stream EventEnvelope);
}
```

## proto/account.proto (novo, integral)

```proto
syntax = "proto3";

package wamux.v1;

import "common.proto";

message CreateAccountRequest {
  optional string external_ref = 1;
  reserved 2; // was ConnectionPolicy policy (removed: connect is edge-driven)
  reserved "policy";
}

message Account {
  string uuid = 1;
  string external_ref = 2;
  reserved 3; // was ConnectionPolicy policy (removed: connect is edge-driven)
  reserved "policy";
  ConnectionState state = 4;
  Jid jid = 5;
  string push_name = 6;
}

message ListAccountsRequest {}

message ListAccountsResponse {
  repeated Account accounts = 1;
}

message AccountStatus {
  string uuid = 1;
  ConnectionState state = 2;
  Jid jid = 3;
  string push_name = 4;
  int64 paired_at = 5;
}

message PairWithQrRequest {
  AccountRef account = 1;
  // true = connect with history backfill ON so the InitialBootstrap dump the
  // phone pushes right after linking is emitted as HistorySyncEvent(s).
  bool backfill_history = 2;
}

message PairWithCodeRequest {
  AccountRef account = 1;
  string phone_number = 2;
  optional string custom_code = 3;
  bool backfill_history = 4; // see PairWithQrRequest.backfill_history
}

message ConnectAccountRequest {
  AccountRef account = 1;
  // false (default) = skip the phone's history dump (relay-pure default).
  // true = process and emit it as HistorySyncEvent(s) for backfill (e.g. a CRM).
  bool backfill_history = 2;
}

message PairedInfo {
  Jid jid = 1;
  // Removed: the lib's PairSuccess carries no push name at pair time; the old
  // field relayed the business name under a misleading name. Use
  // business_name, and take push names from PushNameUpdate events.
  reserved 2;
  reserved "push_name";
  string business_name = 3;         // empty for personal (non-business) accounts
  Jid lid = 4;                      // the account's LID (hidden-PN) identity
  string platform = 5;              // e.g. "android", "smba", "iphone"
}

message PairingError {
  string message = 1;
}

// Streamed during pairing: QR refreshes, the pair code, then success or error.
message PairingUpdate {
  oneof event {
    string qr_code = 1;
    string pair_code = 2;
    PairedInfo paired = 3;
    PairingError error = 4;
  }
}

service AccountService {
  rpc CreateAccount(CreateAccountRequest) returns (Account);
  rpc ListAccounts(ListAccountsRequest) returns (ListAccountsResponse);
  rpc GetAccountStatus(AccountRef) returns (AccountStatus);
  rpc PairWithQr(PairWithQrRequest) returns (stream PairingUpdate);
  rpc PairWithCode(PairWithCodeRequest) returns (stream PairingUpdate);
  rpc ConnectAccount(ConnectAccountRequest) returns (AccountStatus);
  rpc DisconnectAccount(AccountRef) returns (AccountStatus);
  rpc Logout(AccountRef) returns (Empty);
  rpc DeleteAccount(AccountRef) returns (Empty);
}
```

## Checklist de migração da borda

1. Copiar os novos `events.proto` e `account.proto` (ou apontar para o
   `proto/` do core) e regenerar o cliente gRPC.
2. Corrigir o que o codegen quebrar: `PairedInfo.push_name` não existe mais.
3. Buscar e atualizar comparações de string: `"Read"`, `"Delivered"`,
   `"Played"`, `"Composing"`, `"Paused"` → tokens minúsculos; adicionar
   `"recording"`.
4. Dispatch de gaps por `kind` (`gap` vs `subscription_gap`).
5. Trocar parse de `code=(\d+)` na mensagem de erro pelos trailers
   `wa-code`/`wa-text`.
6. Remover workarounds de replay (dedupe por seq vira no-op; re-subscribe
   pós-CreateAccount fica desnecessário).
7. Revisar envios de mídia que preenchem `quote`/`mentions`: agora têm efeito.
