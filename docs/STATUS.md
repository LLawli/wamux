# wamux — Relatório de status (2026-06-09)

Core (daemon) de um sistema WhatsApp não oficial, estilo Evolution API, **só Unix
socket** (gRPC), multi-conta, **relay puro**, sobre `whatsapp-rust 0.6.0` (sem
Baileys/JS), Rust nightly + edição 2024. Auth/permissões/HTTP ficam numa **borda
separada** (fora de escopo).

## 1. Implementado

### Infra / build
- Crate `wamux` (lib + binário) e bins utilitários; `rust-toolchain.toml` pinado
  (`nightly-2026-06-08`, por causa de `simd`/edição 2024 da lib).
- `build.rs` com **protoc vendorizado** (`protoc-bin-vendored`) — sem instalar nada
  no host; regenera de `proto/` a cada build; emite `FILE_DESCRIPTOR_SET` (reflection).
- Dual prost: `prost 0.13` (codegen tonic) + alias `prost014` (`waproto`, p/ encode de `wa::Message`).
- Layout conforme `CLAUDE.md`: `proto/`, `src/{main,lib,server,proto,config,error}`,
  `transport/`, `state/`, `domain/`, `services/`, `storage/postgres/`, `migrations/`.

### Contratos gRPC (`proto/`, pacote `wamux.v1`)
- `common`, `account`, `events`, `messaging`, `media`, `groups`, `contacts`, `admin`.
- 7 serviços: **Account, Event, Messaging, Media, Group, Contact, Admin**.

### Storage (Postgres, `sqlx`)
- Reimplementação das **4 traits** do wacore (`SignalStore`/`AppSyncStore`/`ProtocolStore`/`DeviceStore`)
  → `Backend` por blanket impl. ~57 métodos.
- **16 tabelas** (15 da lib + `accounts`), todas escopadas por `device_id`; `accounts`
  mapeia UUID/`external_ref` → `device_id` (IDENTITY), FK cascade.
- Formatos de bytes **idênticos** à referência SQLite (raw / bincode-standard /
  serde_json) e o `Device` inteiro como 1 blob bincode (restaura `device_props` no load).
- `error_map` (sqlx→StoreError), migrations embutidas (`sqlx::migrate!`).

### Runtime multi-conta (`state/`, `domain/bot_factory`)
- `AccountRegistry` (único `Arc<T>` injetado; `DashMap<uuid, AccountHandle>` + índice `external_ref`).
- `AccountHandle`: `Arc<Client>`, **supervisor task** (dona do `Bot`; aguarda o fim terminal
  do run loop → `on_bot_exited` deixa `is_running` verdadeiro), `broadcast` de eventos
  (capacidade via config), ring buffer, `watch<ConnectionState>`. Stop **gracioso**
  (`Client::disconnect` + espera com timeout). Budget `max_connected_accounts` no connect.
- `bot_factory` (wira PgBackend + tokio transport/runtime + ureq + on_event), `event_bridge`
  (Event→envelope, broadcast + ring **só se `replayable`**: history sync e eventos acima de
  `replay_max_event_bytes` ficam fora do ring). **History sync: skip por default**
  (relay puro); a borda liga o backfill no `ConnectAccount.backfill_history` **e no
  `PairWithQr`/`PairWithCode.backfill_history`** (este último capta o InitialBootstrap).
- **Conexão dirigida pela borda**: o core carrega as contas no boot mas NÃO conecta
  sozinho (sem política always-on / sem coluna `connection_policy`). A borda chama
  `ConnectAccount`/`DisconnectAccount` para as contas que quer vivas (inclusive após restart).
- Identidade da conta: **híbrida** UUID canônico + `external_ref` opcional.

### Mensageria / mídia / grupos / contatos
- **Envio escolhe a conta por request** (`account: AccountRef`); `to` é o destinatário.
- Mensageria: texto (+menções +reply), mídia (**só inline streaming**; a borda baixa
  qualquer URL e manda os bytes — o core não faz HTTP de saída),
  reação, editar, apagar (revoke/for-me), presença (digitando/gravando), mark-read,
  **FetchMessageHistory** (PDO sob demanda: pede N msgs mais antigas de um chat; resposta
  chega assíncrona como `HistorySyncEvent` correlacionada por `session_id`).
- Mídia recebida: **lazy** (evento traz descritor → `DownloadMedia` decifra/stream).
- Grupos: create, add/remove, promote/demote, subject/description, metadata, invite link
  (get/revoke), join, **ListParticipating**, **LeaveGroup**.
- Contatos/perfil: CheckOnWhatsApp (usync; **recebe JIDs completos**, o core não normaliza),
  foto (get/set/remove), push name (get/set),
  about, business profile, subscribe presence.
- Admin: `GetMetrics` (render Prometheus **real** via facade `metrics` +
  `metrics-exporter-prometheus`, sem 2º listener/TCP) e `Check` (health do **daemon**:
  `serving` + `ready` por `SELECT 1` no pool + `version`).
- **Observabilidade (Sprint 2)**: tower layer `RequestObserveLayer` → **1 linha/request**
  (método, `peer_uid` via SO_PEERCRED, latência, `grpc_status`) + métricas
  (`wamux_grpc_requests_total{method,status}`, `wamux_grpc_request_duration_seconds{method}`,
  gauges `wamux_accounts_{total,connected}`). Logs **JSON** opcionais (`log_format`).

### Eventos
- `SubscribeEvents` por conta / todas / nenhuma (= só envio) + replay do ring.
- `EventEnvelope` cobre message, receipt, undecryptable, connection, pairing, presence,
  group, push_name, contact, **history_sync** (só com backfill ligado; carrega o protobuf
  `wa.HistorySync` cru + sync_type/chunk_order/progress/session_id) e **`raw` catch-all**;
  mensagens carregam `raw_message` (protobuf).

### Correções/decisões notáveis
- **Roteamento DM: o core NÃO reescreve JID** (decisão de pureza). O destinatário é
  relayado verbatim. O bug PN→LID da lib (envio 1:1 ganha id mas não entrega em algumas
  contas companion) é contornado **pela borda**, que manda `<numero>@c.us` (legacy) em vez
  de `<numero>@s.whatsapp.net`. Confirmado na lib: `resolve_encryption_jid` só faz upgrade
  de `Server::Pn`/`Hosted`; um JID `@c.us` passa intacto. Sem knob de `dm_routing` no daemon.
- Erros: `WamuxError`→`tonic::Status` com **log da causa íntegra no boundary** (cliente
  recebe Status limpo).
- Transport: UDS bind/unlink, **chmod 0660**, shutdown gracioso (SIGTERM/SIGINT), reflection.
- Config: TOML + env `WAMUX_` (figment).
- `run_isolated`: roda futuros **não-Send** da lib (usync/grupos) em runtime current-thread.

### Docs / tooling
- `docs/PRD.md`, `docs/SPEC.md`, `docs/crate-notes/` (extração verbatim da API), `CLAUDE.md`
  atualizado, `AGENTS.md`→`CLAUDE.md`, `wamux.toml.example`, página fixada no ai-memory.
- Bins de validação: `pair_cli`, `pair_socket`, `e2e`, `e2e_all`, `send_group`, `whois`,
  `validate1`, `recv_media`, `set_pfp`.

## 2. Testado

### Automatizado (verde)
- `cargo build` (nightly), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- Unit: `domain::event_mapping` (mapeamento pairing QR/código) — 2 testes.
- Integração `tests/postgres_backend.rs`: isolamento por `device_id` + round-trip +
  prova `PgBackend: Backend` (contra Postgres docker).
- Integração `tests/grpc_server.rs`: ciclo de conta **sobre socket gRPC real**
  (create/list/status por uuid e external_ref/NotFound/delete).

### Manual, WhatsApp real, **pela API do socket**
- **Pareamento QR** ponta a ponta; persistência no Postgres; **reconexão sem reparear**.
- `e2e_all`: **37/41 PASS** numa rodada (os 4 não-PASS explicados em §3).
- Mensageria: SendText, SendMedia (**imagem, documento, vídeo, áudio/MP3, sticker/WebP**),
  SendReaction, EditMessage, DeleteMessage, SendPresence, MarkRead — entrega confirmada.
  (Áudio entrega como arquivo MP3; OGG/Vorbis o WhatsApp não processa.)
- **Recepção**: mensagens de entrada + presença (digitando) + recibos (Read/ReadSelf).
- **DownloadMedia**: imagem recebida → decifrada → **JPEG válido 960×1280 (93 KB)**.
- Grupos: ListParticipating (256), GetGroupMetadata, CreateGroup (novo), Set Subject/Description,
  GetInviteLink (admin), RevokeInviteLink, Promote/Demote, RemoveParticipants, LeaveGroup, envio em grupo.
- Contatos/perfil: CheckOnWhatsApp (JID canônico via usync), GetPushName, GetAbout,
  GetProfilePicture, GetBusinessProfile, SubscribePresence, SetPushName(+restaura),
  **SetProfilePicture** (JPEG quadrado, conexão estável, +restaura).
- Admin: GetMetrics.
- **History backfill (2026-06-09)**: pareamento de conta nova com backfill ON capturou o dump
  completo via `HistorySyncEvent`: InitialBootstrap (53 conv/153 msgs) + InitialStatusV3 + PushName
  + NonBlockingData + Recent (5 chunks, progress 20→100%, blobs de 2–3.7 MB) + Full = **780 conversas,
  24.485 mensagens**. `FetchMessageHistory` on-demand validado (resposta `sync_type=ON_DEMAND` com
  `session_id` casando). Bins: `pair_backfill`, `backfill`.
- **Logout real (2026-06-09)**: `Logout` sobre o socket numa conta conectada → IQ `RemoveCompanionDevice`,
  estado vira `LoggedOut`, device sai da lista de "aparelhos conectados" do celular. Bin: `logout_e2e`.
- Entrega 1:1 via `@c.us` (legacy) validada: a **borda** escolhe o JID; o core relaya verbatim.

## 3. Funcionando vs. não / gotchas

| Item | Estado |
|---|---|
| Tudo da §2 | ✅ funcionando |
| **Pair-code** | ❌ quebrado na `whatsapp-rust 0.6.0` (servidor 400; é a última versão no crates.io). Use **QR**. |
| **Logout** | ✅ unlink real server-side (IQ `RemoveCompanionDevice`); **requer conexão ativa**, senão `FailedPrecondition` (a borda decide conectar antes). Mantém keys locais (re-pareável); `DeleteAccount` limpa o estado local. |

Gotchas conhecidos:
- **Backfill de histórico**: `ConnectAccount.backfill_history` só controla se *processamos* o
  que o celular despeja. O **volume** do dump inicial é definido pelo `HistorySyncConfig` no
  **pareamento** (device props); numa reconexão o celular costuma mandar só o "recent". Para
  histórico mais antigo de um chat, use `FetchMessageHistory` (sob demanda). Os blobs vêm em
  chunks e podem ser grandes: vão pelo **broadcast ao vivo**, mas **Sprint 3** os exclui do
  **ring** de replay (`replayable`), então não pinam memória por conta.
- **On-demand exige o celular primário ONLINE**: `FetchMessageHistory` manda um PDO ao telefone;
  com o app **fechado/offline a resposta não chega** (fica pendente e chega quando o app reabre).
  Além disso, a resposta de on-demand **também** passa pelo gate de skip, então **só é entregue se
  a conexão estiver com `backfill_history=true`** (senão a lib dropa). Validado E2E (2026-06-09):
  resposta veio como `HistorySyncEvent` `sync_type=6` (ON_DEMAND), `session_id` casando, `wa.HistorySync`
  decodificado (1 conversa / 24 msgs).
- **Envio 1:1 PN→LID**: a lib faz upgrade automático PN→LID que não entrega em algumas
  contas companion; a **borda** resolve mandando o JID `@c.us` (o core não reescreve).
- **SetProfilePicture**: precisa de conexão **estável** (falha logo após o pareamento com
  `client is not connected`) e **JPEG quadrado**.
- **Número BR (9º dígito)**: o JID canônico vem do usync (geralmente **sem** o 9); não adivinhar.
- Enviar pro próprio JID com sufixo de device (`...:NN@s.whatsapp.net`) não aparece como conversa.

## 4. Lacunas conhecidas
- **Roteamento DM saiu do core por completo** (núcleo puro): nada de `dm_routing` (nem global,
  nem por request, nem por conta). A borda escolhe o JID (`@c.us` vs `@s.whatsapp.net`) e
  implementa qualquer fallback usando `SendResult.message_id` + eventos `Receipt`.
- ~~Backfill no pareamento não exposto pela borda.~~ **Feito**: `PairWithQr`/`PairWithCode` agora
  têm `backfill_history`, então a borda captura o InitialBootstrap pelo socket (pareia com backfill
  ON). Validado E2E via `pair_backfill` (registry-direct); a via socket usa o mesmo `pairing_stream`.
- ~~**Logout real** (unlink do device) ausente.~~ **Feito** (requer conexão ativa).
- ~~Observabilidade: falta tower layer 1 linha/request e métricas reais.~~ **Feito (Sprint 2)**:
  `RequestObserveLayer` (1 linha/request: método/uid/latência/status) + facade `metrics` +
  `GetMetrics` real + `Check` (health/readiness) + logs JSON (`log_format`).
- ~~Eventos: `SubscribeEvents` “todas as contas” não inclui contas criadas **após** a inscrição.~~
  **Feito (Sprint 5, 2026-06-09)**: assinatura “todas” é **dinâmica** — broadcast
  `subscribe_created` no registry + follower no `EventSvc` (inscreve **antes** do snapshot e
  dedupa por uuid); o stream “todas” agora fica **aberto indefinidamente**; lag no canal de
  criação (>64 creates em rajada) vira marcador `gap` (a borda re-inscreve ou `ListAccounts`).
  Teste de integração `tests/event_subscription.rs`.
  Sem filtro por tipo no core (a borda filtra).
- ~~Sem **CI**; poucos testes unitários.~~ **Feito (Sprint 4)**: CI local `scripts/ci.sh`
  (sem GitHub, por decisão) + 56 testes unitários no lib (+44). Sem `.sqlx`/`sqlx prepare`:
  as queries são runtime, não macros.
- ~~**Escala** não foi stress-testada.~~ **Sprint 3**: supervisor por conta (`is_running`
  verdadeiro), budget de conexões (`ResourceExhausted`), history sync fora do ring, stop
  gracioso, contrato de gap explícito; load test sintético ~200 contas (`#[ignore]`) prova
  ausência de HOL blocking + gap sob carga. **Emulador WSS (feature `stress`)**: M1 handshake,
  M2a login bidirecional, M2b push de stanza→evento + keepalive real, **M3 = 199 `Client`s reais
  conectados ao mock e mantidos vivos** (fds reais de centenas de WSS ✅). Falta validar caminho
  terminal real e **M4** (1 conta WhatsApp viva + probes de RTT sob a carga das 199).
- Mídia: imagem/documento/vídeo/áudio(MP3)/sticker **validados E2E**. Falta **nota de voz (PTT)**
  — exige OGG/Opus + `ptt=true` + `seconds` (campos ainda não expostos no `SendMedia`).
- A **borda HTTP/auth/permissões** é projeto à parte.

## 5. Próximas sprints (proposta)

**Sprint 1 — Entrega 1:1 robusta & roteamento** ✅ (2026-06-09)
- ✅ **Roteamento DM removido do core** (decisão de pureza, núcleo é relay verbatim): sem
  `dm_routing` no `Config`, nos RPCs ou por conta. A borda escolhe o JID (`@c.us`/`s.whatsapp.net`)
  e faz qualquer fallback com `SendResult.message_id` + eventos `Receipt`. Bins de validação
  passam a mandar `@c.us`.
- ✅ `Logout` real (IQ `RemoveCompanionDevice`); requer conexão ativa, senão `FailedPrecondition`.
- ⏭️ `ResolveContact` **descartado**: `CheckOnWhatsApp` (usync) já resolve número→JID canônico.
- ✅ **Auditoria de pureza do core** (princípio gravado em `CLAUDE.md` + ai-memory). Removidas 3
  impurezas: (1) `CheckOnWhatsApp` deixa de adivinhar/normalizar (recebe JIDs completos);
  (2) `SendMedia` perde o download por URL (só inline; sem HTTP de saída no core);
  (3) conexão dirigida pela borda (sem always-on / sem coluna `connection_policy`,
  migration `0002`).
- Testes: regressão Logout-desconectado (`FailedPrecondition`) em `tests/grpc_server.rs`.

**Sprint 2 — Observabilidade & ops** ✅ (2026-06-09)
- ✅ Tower layer `RequestObserveLayer` (`src/observe.rs`): **1 linha por request** (método,
  uid do peer via SO_PEERCRED nas extensions, latência, `grpc_status`). Lê o `grpc-status`
  do trailer (ou do header em respostas trailers-only) embrulhando o body; emite log+métrica
  uma vez no drop do body.
- ✅ Facade `metrics` + `metrics-exporter-prometheus` (default-features off → **sem listener TCP**;
  render só pelo socket). `GetMetrics` faz `handle.render()` + publica gauges de conta.
  Recorder global instalado uma vez (`OnceLock`).
- ✅ Logs JSON configuráveis (`log_format = "json"|"text"`).
- ✅ RPC `AdminService.Check` (health/readiness: `serving`+`ready`(SELECT 1)+`version`).
- ✅ Teste de integração `admin_health_and_metrics_over_socket` (Check + GetMetrics sobre socket
  real, prova a layer ponta a ponta) + unit tests de `observe` (method_label/code_name).

**Sprint 3 — Hardening & escala** ✅ (2026-06-09)
- ✅ **Supervisor por conta** (`account_registry::connect`): aguarda o `BotHandle` (fim
  *terminal* do run loop; a lib reconecta quedas transientes sozinha com backoff Fibonacci),
  então `on_bot_exited` limpa `running`+client, seta estado terminal e libera o slot do
  budget. Conserta o bug do `is_running` que **mentia** (keepalive `pending` eterno).
  **Sem backoff/retry no core** — reconexão pós-terminal é política da borda.
- ✅ **Stop/disconnect gracioso**: `Client::disconnect` (flush + fecha transporte) e espera o
  supervisor até `graceful_stop_timeout_ms`, destacando no timeout (sem run loop órfão).
- ✅ **Budget de conexões** (`max_connected_accounts`, 0=ilimitado): `Connect` além do cap →
  `ResourceExhausted`. Auto-proteção de fd/recursos; o número é config, não política.
- ✅ **History sync fora do ring** + cap `replay_max_event_bytes`: blobs de 2–3.7 MB não
  pinam memória no ring (continuam no broadcast ao vivo). Predicado `replayable`.
- ✅ **Contrato de gap explícito**: marcador `gap` carrega o estado atual; a borda re-sincroniza
  via `GetAccountStatus` (watch autoritativo). Forwarders por conta independentes (sem HOL
  blocking entre contas).
- ✅ `broadcast_capacity` virou config (antes hardcoded 1024).
- ✅ **Load test sintético ~200 contas** (`tests/load_multi_account.rs`, `#[ignore]`): prova
  ausência de HOL blocking + emissão de gap sob carga. + unit tests (`replayable`, budget,
  `on_bot_exited`, gap marker).
- 🟢 **Emulador WhatsApp para stress real** (feature `stress`, fora do build de produção;
  M1/M2a/M2b/M3 ✅, M4 com bin pronto p/ run ao vivo):
  servidor mock WSS que fala o lado servidor do handshake **Noise XX** (viável porque o
  `verify_server_cert` da lib é frouxo de propósito — `WA_CERT_PUB_KEY` fica unused p/ permitir
  mock e2e). **M1 ✅**: `Client` real do `whatsapp-rust` completa o handshake contra o mock
  sobre `ws://` loopback e o servidor decifra o ClientPayload (`tests/stress_handshake.rs`).
  Transporte injetável via `RegistryTuning::ws_url_override` + `build_bot(ws_url)`.
  **M2a ✅**: device registrado (`pn` set, persistido via `PgBackend::save`) → cliente envia
  payload de **login** → servidor manda `<success>` (nós binários via `wacore-binary`) → cliente
  **loga** e envia IQs pós-login que o servidor **decifra** (transporte cifrado bidirecional OK;
  cifras trocadas no servidor: send=read, recv=write; contadores por-direção). Servidor responde
  `<iq result>` genérico para sustentar a conexão.
  **M2b ✅**: o mock **empurra um `<receipt>`** pós-login (stanza server-originada, sem sessão
  Signal) → o `Client` real decodifica e despacha `Event::Receipt` → surge como `ReceiptEvent`
  no broadcast da conta (teste `pushed_receipt_surfaces_as_event`, rápido). **Keepalive sustentado**
  validado (teste `connection_survives_keepalive_window`, `#[ignore]` ~25 s): o client manda seu
  ping de keepalive (`<iq xmlns="w:p">`), o mock responde (pong RTT ~1 ms), a conexão **não
  reconecta** (1 handshake). **Bug do mock corrigido** (crítico p/ M3): o payload decifrado é
  `[flag_byte][nó]` (flag&2 ⇒ zlib), o mesmo envelope que `wacore_binary::Encoder` escreve no send
  (write_u8(0)); o recv do mock chamava `unmarshal_ref` direto → falha silenciosa → os IQs pós-login
  do client (usync) ficavam sem resposta → waiter eterno → keepalive pulava o ping (`has_pending`)
  → dead-socket reconnect. Fix: `wacore_binary::util::unpack` antes do `unmarshal_ref`. (M2a passava
  porque só checava o contador de frames, nunca parseava um nó.)
  **M3 ✅**: provisiona **199 devices registrados** e os conecta **todos ao mock de uma vez** —
  cada `Client` real completa o handshake XX, loga, e fica **supervisionado vivo** (teste
  `connect_many_clients_against_mock`, `#[ignore]`, ~42 s; N via `STRESS_ACCOUNTS`, default 199).
  Asserts: `handshakes_completed >= N`, `connected_count == N` (antes e depois de um hold de 2 s,
  sem saídas terminais), `post_login_frames >= N`. O fix do M2b é o que permite as N sobreviverem
  ao keepalive. fds folgados (limit 524k); pool PG 16; `graceful_stop_timeout` 500 ms p/ teardown.
  **M4 ✅ (validado ao vivo, 2026-06-09)**: `src/bin/stress_live.rs` (gated por `required-features
  = ["stress"]`) sobe **dois registries no mesmo pool PG** — um real (sem `ws_url_override`) e um
  mock — conecta a conta secundária real, mede **RTT do delivery-receipt** de um texto enviado ao
  primário **baseline (sem carga) vs sob a carga das 199 fakes**. **Resultado real**: baseline
  median **573 ms** (min 472 / max 1248, 0 timeouts) vs under-load median **574 ms** (min 552 /
  max 2138, 0 timeouts) — **as 199 conexões fake não pesam na mediana** (~+1 ms; o max é o primeiro
  probe de cada fase, warmup de sessão/phash). **Envio só p/ o destino em `WAMUX_LIVE_DEST`**
  (guard no código; regra ai-memory `_rules/live-whatsapp-sends.md`). Espera
  `client.is_logged_in()` (não o `Connected` de socket). Rodar:
  `cargo run --features stress --bin stress_live -- m4-real 199 3` (reusa a conta pareada, sem QR).

**Sprint 4 — CI & qualidade** ✅ (2026-06-09)
- ✅ **Repositório git local** (`main`; decisão: **sem GitHub**) com commit baseline do fim do Sprint 3.
- ✅ **CI local em vez de GitHub Actions**: `scripts/ci.sh` é O pipeline — fmt --check, clippy
  (default **e** `--features stress`, all-targets, `-D warnings`), `cargo test` (unit+integração),
  stress rápidos (M1/M2a/M2b); `--full` adiciona os `#[ignore]` (load HOL/gap, keepalive ~25 s,
  M3 com `STRESS_ACCOUNTS`). Checagem early de Postgres com erro acionável. Registrado no CLAUDE.md.
  (O passo `cargo sqlx prepare`/cache `.sqlx` da proposta caiu: as ~60 queries são **runtime**
  `sqlx::query`, sem macros — só precisa do Postgres de serviço.)
- ✅ **+44 testes unitários** (12 → 56 no lib): `event_mapping` 2→24 (mensagem texto/menção/quote/
  reação/mídia×5, receipt, undecryptable, presence/chat-presence, history_sync passthrough, estados
  de conexão, pairing, dropped Notification/RawNode→None, catch-all Raw+variant_name) — test mod
  extraído p/ `event_mapping_tests.rs` (regra de 500 linhas); **storage blob round-trips** 6 novos
  em `storage/postgres/mod.rs` (Device bincode com pn/push_name/key material, AppStateSyncKey,
  HashState com hash [u8;128], Vec<DeviceInfo> serde_json espelhando protocol_store, determinismo
  do encode, garbage→StoreError::Serialization sem panic; nota: `device_props` é #[serde(skip)] by
  design — o load() restaura); **domain puro** 16 novos (jid_parse 6 — inclui **regressão `@c.us`
  → Server::Legacy + to_string verbatim**, pureza PN→LID; messaging 3 — proto_key_to_wa participant
  vazio→None, send_result_to_proto; media_transfer 5 — case-sensitive pinado, build_media_message
  image/document; groups 2 — metadata_json round-trip).

**Sprint 5 — Completude de features** ✅ (2026-06-09)
- ✅ **Nota de voz (PTT)**: `SendMediaHeader` ganha `ptt`/`seconds`/`waveform` (relay verbatim;
  forma ausente pinada em teste: ptt=false→None, seconds=0→None, waveform vazio→None). WhatsApp
  só renderiza PTT para **OGG/Opus** — os bytes certos são responsabilidade da borda. **Validado
  ao vivo**: `WAMUX_REF=m4-real send_types /tmp/wamux.sock 5511999999999 ptt` (OGG/Opus 3 s via
  `WAMUX_PTT_FILE`) entregue ao primário sobre o socket.
- ✅ **Link preview**: novo `LinkPreview` no `SendTextRequest` (`matched_text`/`title`/`description`/
  `jpeg_thumbnail`/`preview_type`). A **borda** busca a URL e fornece os campos; o core relaya
  verbatim (sem HTTP de saída — mesma doutrina da mídia). Este waproto não tem `canonical_url`;
  `matched_text` é a URL. `preview_type=0` relaya como campo ausente (forma lib-natural).
- ✅ **Mensagens efêmeras**: `ephemeral_seconds` no `SendTextRequest` e no `SendMediaHeader` →
  `ContextInfo.expiration` (0 = não-efêmera; o valor — ex.: a configuração do chat — vem da borda).
  Texto: `build_text_message` puro (conversation simples só quando nada extra; senão
  ExtendedTextMessage). Mídia: contexto efêmero nos 5 tipos; mídia não-efêmera fica byte-idêntica.
- ✅ `SubscribeEvents`: inclusão dinâmica de novas contas na assinatura “todas” — o registry emite
  broadcast de contas criadas; o serviço assina **antes** do snapshot e dedupe por uuid (race
  comentada). O stream “all” fica aberto indefinidamente (contrato no proto); lag no canal de
  criações (>64) emite gap apontando re-subscribe/ListAccounts. Teste de integração
  `tests/event_subscription.rs` cobre o caminho dinâmico e o snapshot.
  Filtro por tipo **descartado** pelo litmus de pureza: a borda filtra com o que já recebe.
- Refactors de apoio: `build_media_message(header, upload)` (assinatura wire-shaped), builders
  por tipo `*_submessage`, `domain/wire_defaults.rs` (proto3 vazio→None compartilhado).
  +15 testes unit (70 no lib) + 1 integração nova. `send_types` ganha o kind `ptt` + `WAMUX_REF`.
  (History sync: **feito** — backfill por conexão + `FetchMessageHistory` sob demanda.)

**Sprint 6 — A borda (projeto separado)**
- Camada HTTP/auth/permissões consumindo o socket; webhooks; filtragem por usuário.

## Como rodar / validar
- Postgres: `docker run -d --name wamux-pg -e POSTGRES_USER=wamux -e POSTGRES_PASSWORD=wamux -e POSTGRES_DB=wamux -p 5433:5432 postgres:16`
- Testes: `DATABASE_URL=postgres://wamux:wamux@localhost:5433/wamux cargo test`
- Daemon: `WAMUX_DATABASE_URL=... WAMUX_SOCKET_PATH=/tmp/wamux.sock cargo run`
  (sem knob de roteamento: para forçar legacy, o cliente manda o JID `<numero>@c.us`)
- Pareamento QR (abre PNG): `cargo run --bin pair_socket /tmp/wamux.sock`
- E2E completo: `WAMUX_E2E_DESTRUCTIVE=1 cargo run --bin e2e_all /tmp/wamux.sock <numero>`
