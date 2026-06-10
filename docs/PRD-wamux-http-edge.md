# wamux-http-edge: PRD

> **Handoff:** este documento é autocontido e destinado a iniciar o projeto numa pasta
> nova, com outro agente, sem acesso ao repositório do core. Tudo que a borda precisa
> saber sobre o core (contratos, gotchas, responsabilidades herdadas) está aqui.
> Copie também o diretório `proto/` do core para a pasta nova (ele é a fonte da
> verdade do contrato gRPC; sincronize a cada mudança do core).

## 1. Contexto e posicionamento

O **wamux** é um core daemon de WhatsApp não-oficial (estilo Evolution API, mas sem
Baileys/JS): multiplexa muitas contas WhatsApp num único processo Rust e expõe tudo
via **gRPC sobre Unix domain socket**, sem nenhuma auth própria. O core é
deliberadamente um **relay puro**: toda política (auth, retries, filtragem, webhooks,
reconexão pós-terminal, histórico) foi empurrada para fora, para "a borda".

O **wamux-http-edge** é duas coisas ao mesmo tempo:

1. **A borda de referência.** O core vai ser publicado open source; ninguém adota um
   core que nem o autor usou. Este projeto é a prova executável de que o core é
   usável e o exemplo canônico de como consumi-lo. A camada que fala com o socket
   deve ser escrita para ser lida e copiada por futuros autores de borda.
2. **Um produto SaaS multi-tenant real.** API HTTP própria (não compatível com
   Evolution) para terceiros conectarem números de WhatsApp, enviarem/receberem
   mensagens e integrarem via webhooks/streams, com dashboard mínimo.

## 2. Personas

- **Tenant (cliente do SaaS):** desenvolvedor terceiro que se cadastra (signup
  aberto), cria instâncias, pareia o WhatsApp dele (QR), envia mensagens via REST e
  recebe eventos via webhook/SSE/WebSocket/polling.
- **Operador (você):** administra tenants, planos/quotas, observa o sistema, opera o
  deploy core+borda, integra com o sistema de cobrança externo.
- **Futuro autor de borda:** lê a camada de consumo do core como documentação viva.

## 3. Arquitetura em duas camadas (decisão estrutural)

```
wamux-http-edge
├── core-client/   # Camada 1: consumo do wamux (o "exemplo canônico")
│   ├── cliente tonic sobre UDS (reconexão ao socket)
│   ├── política de conexão de contas (always-on, reconnect pós-terminal c/ backoff)
│   ├── política de JID (@c.us, resolução via CheckOnWhatsApp + cache)
│   ├── pipeline de eventos (subscribe all dinâmico, recuperação de gap)
│   └── pipeline de envio (download de URL -> bytes inline, OGG/Opus p/ PTT)
└── product/       # Camada 2: o SaaS
    ├── auth (usuários, signup aberto, JWT, API keys)
    ├── API REST (axum) + SSE + WebSocket + polling
    ├── webhooks (fila, retry, assinatura HMAC) — eventos E billing
    ├── buffer de retenção (Postgres, N dias)
    ├── quotas + medição de uso (superfície de billing)
    └── dashboard (server-rendered, askama + htmx; PT + EN)
```

A Camada 1 não conhece tenants nem HTTP; expõe tipos Rust limpos para a Camada 2.
É o que um futuro autor de borda copiaria inteiro.

## 4. Decisões (locked)

| Tópico | Decisão |
|---|---|
| Consumidores | Terceiros, SaaS multi-tenant |
| API | Própria, limpa (sem compat Evolution; adaptador compat = fora de escopo) |
| Stack | Rust: axum (HTTP), tonic (cliente gRPC/UDS), sqlx/Postgres, tokio |
| Cadastro | **Signup aberto** (verificação de e-mail); anti-abuso via rate limit + quota de plano free |
| Billing | Sem cobrança embutida; **superfície de billing**: a borda mede uso e expõe admin API + emite/recebe webhooks de billing (§9) |
| Mercado | **BR primeiro, base neutra**: lógica de número/fuso em E.164 puro (sem hardcode regional), docs/ToS/marketing focados em BR no v1 |
| Histórico | Buffer curto de retenção (**default 7 dias, máx configurável 30**; não é histórico completo) |
| Auth | Usuários (signup/login, JWT para dashboard) + API keys m2m por tenant |
| Eventos p/ tenant | Os 4 transportes: webhooks, SSE, WebSocket, polling com cursor; o consumidor escolhe |
| Envio | **Síncrono** (POST espera o `SendResult`, 200 com message_id) **+ `Idempotency-Key`** para deduplicar reenvios |
| Echo (from_me) | **Sempre entregar, marcado**: `from_me=true` + origem (`sent_via_api`/`sent_via_phone`); filtro fica 100% com o tenant |
| Grupos | **Opt-in por instância** (default só DMs); eventos de grupo contam na quota |
| Número repetido | **Único por tenant**: a borda rejeita duas instâncias do mesmo número no mesmo tenant; tenants diferentes podem (são devices companion distintos) |
| Mídia recebida | On-demand: webhook carrega `media_id`; `GET /v1/media/{id}` decifra na hora via core (zero storage de mídia) |
| Escopo v1 | API + dashboard mínimo |
| Dashboard | Server-rendered no próprio axum (askama) + htmx; binário único, sem toolchain JS; **PT + EN** |
| TLS/ingress | Borda serve **HTTP puro**; TLS/domínio/cert num reverse proxy externo (Caddy/nginx/Traefik), documentado no deploy |
| Backup/DR | Seção dedicada (§13): o DB do core guarda as chaves Signal — perder = re-pareamento de todas as instâncias |
| Open source | Sim, core e borda; **forja a definir na hora de publicar** |
| Idioma docs internas | Português (este PRD); código/identificadores em inglês; dashboard + docs públicas da API em PT + EN |

## 5. Responsabilidades herdadas do core (não-negociáveis)

O core empurrou estas responsabilidades para a borda **por design**. Cada uma deve
existir na Camada 1:

1. **Auth total.** O socket não tem auth: quem abre vê todas as contas. A borda é a
   única coisa que pode abrir o socket em produção (mesmo host, permissão 0660 +
   grupo).
2. **Política de conexão.** O core boota com todas as contas DESCONECTADAS. A borda
   mantém a lista de instâncias que devem estar vivas e chama `ConnectAccount` no
   boot e após restart do core. Quedas transientes o core/lib resolve sozinho
   (backoff Fibonacci interno); a borda só reage a **exits terminais**
   (401/409/logout, estado `LoggedOut`/`Banned`/`Disconnected` final) com a política
   dela (backoff exponencial próprio, marcar instância como `disconnected`, notificar
   o tenant via webhook `connection.update`).
3. **Política de JID.** Envio 1:1: usar `<numero>@c.us` (legacy) para evitar o
   upgrade automático PN→LID da lib, que não entrega em algumas contas companion.
   Nunca adivinhar o 9º dígito BR: resolver número→JID canônico com
   `CheckOnWhatsApp` (usync) e cachear o resultado. O core relaya o JID verbatim.
4. **Retries/timeouts/entrega.** Compostos de `SendResult.message_id` + eventos
   `Receipt`. O core não tem nenhum "se X não aconteceu em N segundos".
5. **Recuperação de gap.** A entrega de eventos do core é lossy sob lag: o stream
   emite um `RawEvent{kind:"gap"}` com o estado atual na nota. Ao receber gap, a
   borda ressincroniza estado via `GetAccountStatus` e aceita a perda (os eventos
   perdidos não voltam; o ring de replay é curto e best-effort via
   `replay_from_ring`). No subscribe "all", um gap do canal de criações significa
   "contas novas podem estar faltando": reconciliar com `ListAccounts` ou
   re-assinar.
6. **Mídia.** O core NÃO faz HTTP de saída. Envio por URL: a borda baixa e streama
   os bytes inline (`SendMedia` é client-streaming: header + chunks). Recebimento:
   o evento traz um `MediaDescriptor` (direct_path, media_key, hashes, mime); a
   borda chama `DownloadMedia` (server-streaming) quando o tenant pedir.
7. **Link preview.** A borda busca a URL e fornece `matched_text` (a própria URL,
   o waproto desta versão não tem canonical_url), `title`, `description`,
   `jpeg_thumbnail`; o core relaya verbatim.
8. **PTT (nota de voz).** WhatsApp só renderiza nota de voz para **OGG/Opus**. A
   borda transcoda (ffmpeg/libopus) quando o tenant pedir voice note, e seta
   `ptt=true` + `seconds` (+ `waveform` opcional) no `SendMediaHeader`. Áudio comum:
   MP3 funciona como arquivo; OGG/Vorbis o WhatsApp não processa.
9. **Mensagens efêmeras.** `ephemeral_seconds` no send: o valor (configuração do
   chat) é responsabilidade da borda/tenant; 0 = não-efêmera.
10. **Histórico.** O core não persiste mensagem nenhuma. Backfill do pareamento só
    chega se `PairWithQr.backfill_history=true`; on-demand
    (`FetchMessageHistory`) exige o celular primário ONLINE e a conexão com
    `backfill_history=true` (senão a lib dropa a resposta); resposta chega
    assíncrona como `HistorySyncEvent` correlacionada por `session_id`.

## 6. Contrato do core (resumo para a pasta nova)

gRPC, pacote `wamux.v1`, sobre UDS (sem TCP/TLS). Reflection ligada em dev. 7
serviços:

- **AccountService:** CreateAccount (uuid + `external_ref` opcional e único),
  ListAccounts, GetAccountStatus, PairWithQr (server-stream de QR/resultado, aceita
  `backfill_history`), PairWithCode (QUEBRADO na whatsapp-rust 0.6.0, usar QR),
  ConnectAccount (aceita `backfill_history`), DisconnectAccount, Logout (exige
  conexão ativa, senão `FailedPrecondition`; faz unlink real no celular),
  DeleteAccount (apaga estado local, cascade).
- **EventService:** SubscribeEvents (server-stream). Seletor: uma conta | todas |
  nenhuma (= send-only). "Todas" é DINÂMICO: inclui contas criadas depois do
  subscribe e o stream fica aberto indefinidamente. `replay_from_ring` para replay
  curto. `EventEnvelope`: account_uuid, monotonic_seq (-1 = gap), ts_unix_ms, oneof
  {message, receipt, undecryptable, connection, pairing, presence, group,
  push_name, contact, history_sync, raw}. InboundMessage carrega `from_me` na
  `MessageKey` (base do echo marcado).
- **MessagingService:** SendText (text, mentions, quote, link_preview,
  ephemeral_seconds), SendMedia (client-stream: SendMediaHeader{mime_type, caption,
  media_type image|video|audio|document|sticker, filename, ptt, seconds, waveform,
  ephemeral_seconds} + chunks), SendReaction, EditMessage, DeleteMessage
  (revoke/for-me), FetchMessageHistory, SendPresence
  (typing/recording/paused/available), MarkRead. SendResult traz `message_id`.
- **MediaService:** DownloadMedia (server-stream: meta + chunks decifrados).
- **GroupService:** create, add/remove, promote/demote, subject/description,
  metadata, invite link get/revoke, join, ListParticipating, LeaveGroup.
- **ContactService:** CheckOnWhatsApp (recebe JIDs completos, não normaliza), foto
  de perfil get/set/remove (set exige conexão estável + JPEG quadrado), push name
  get/set, about, business profile, subscribe presence.
- **AdminService:** GetMetrics (texto Prometheus), Check (liveness/readiness do
  daemon).

Identidade de conta: UUID canônico + `external_ref` opcional. **Convenção da
borda:** `external_ref = "edge:{instance_id}"`, e o instance_id é o identificador
público que o tenant vê (ULID opaco).

## 7. Modelo de objetos do produto

```
User (email, senha argon2id, email_verified, role: admin|member)
 └── Tenant (1 user dono no v1; membros = fase 2; plano + quota)
      ├── ApiKey (hash, prefixo público we_live_..., last_used, revogável; escopo total no tenant)
      ├── Instance (id público ULID, nome, número E.164, status, groups_enabled,
      │     │        ↔ conta wamux via external_ref "edge:{instance_id}")
      │     ├── Webhook (url, secret HMAC, filtro de tipos de evento, estado, ativo)
      │     └── eventos no buffer de retenção (cursor por instância)
      ├── usage counters (msgs in/out, instâncias ativas, eventos — por período)
      └── quota (do plano: msgs/min, instâncias máx, eventos/dia, retenção)
```

- **Instance** é o objeto central: o tenant cria, pareia via QR, conecta/desconecta,
  envia em nome dela e recebe eventos dela.
- **Número único por tenant:** ao parear, a borda registra o número E.164 resolvido;
  rejeita uma segunda instância do mesmo número no mesmo tenant
  (`409 number_already_paired`).
- Estados da instância (derivados do core + política da borda): `created` (sem
  pareamento), `pairing` (QR emitido), `connected`, `disconnected` (terminal ou
  manual), `banned`, `logged_out`.

## 8. API REST (superfície v1)

Prefixo `/v1`, JSON, auth via `Authorization: Bearer <jwt>` (dashboard) ou
`X-Api-Key` (m2m). Erros: problem+json com código estável. Envio aceita
`Idempotency-Key` (header).

**Auth/conta**
- `POST /v1/auth/signup` (signup aberto + verificação de e-mail),
  `POST /v1/auth/verify-email`, `POST /v1/auth/login` (JWT),
  `POST /v1/auth/refresh`, `POST /v1/auth/logout`
- `GET/POST/DELETE /v1/api-keys`

**Instâncias**
- `POST /v1/instances` (cria; core CreateAccount)
- `GET /v1/instances`, `GET /v1/instances/{id}` (status sintetizado do core)
- `POST /v1/instances/{id}/pair` -> stream SSE com QR (string + PNG base64) até
  `paired`/timeout; aceita `backfill_history`
- `POST /v1/instances/{id}/connect`, `POST .../disconnect`, `POST .../logout`
- `PATCH /v1/instances/{id}` (nome, `groups_enabled`)
- `DELETE /v1/instances/{id}`

**Mensagens (por instância) — síncrono, com `Idempotency-Key`**
- `POST /v1/instances/{id}/messages/text` {to, text, quote?, mentions?,
  link_preview? (só url: a borda busca title/desc/thumb), ephemeral_seconds?}
  -> 200 {message_id}
- `POST /v1/instances/{id}/messages/media` (multipart ou {url}; media_type; caption;
  ptt? -> borda transcoda p/ OGG/Opus e seta seconds) -> 200 {message_id}
- `POST .../messages/reaction`, `PATCH .../messages/{msg_id}` (edit),
  `DELETE .../messages/{msg_id}` (?for_everyone)
- `POST .../presence` (typing/recording/paused), `POST .../read`
- `POST /v1/instances/{id}/check` {numbers[]} -> JIDs canônicos (CheckOnWhatsApp,
  com cache)
- `to` aceita número cru (a borda resolve e aplica a política @c.us) ou JID completo
  (relay verbatim)

**Eventos**
- `GET /v1/instances/{id}/events?cursor=...&types=...` (polling sobre o buffer)
- `GET /v1/instances/{id}/events/stream` (SSE; catch-up via `Last-Event-ID`),
  `GET .../events/ws` (WebSocket)
- `GET/PUT /v1/instances/{id}/webhooks` (até N webhooks por instância, cada um com
  url, secret, filtro de tipos)

**Mídia**
- `GET /v1/media/{media_id}` (on-demand: core DownloadMedia, streaming; media_id é
  um token opaco que referencia o descriptor guardado no buffer)

**Grupos/contatos (proxy fino, v1 mínimo)**
- `GET /v1/instances/{id}/groups`, `POST /v1/instances/{id}/groups`,
  `GET .../groups/{jid}`, operações de participantes/subject/invite conforme core
- `GET .../contacts/{jid}/profile` (foto, about, business)

**Admin (operador)**
- `POST /v1/admin/tenants`, `PATCH /v1/admin/tenants/{id}` (plano/quota/suspender),
  `GET /v1/admin/usage` (uso por tenant), `GET /v1/admin/health`, `GET /metrics`

## 9. Billing: superfície (sem cobrança embutida)

A cobrança real mora num sistema externo (ex.: Stripe). A borda é a **fonte de uso**
e o **executor de limites**:

- **Medição:** contadores por tenant e período (mensagens enviadas/recebidas,
  instâncias ativas, eventos entregues). Persistidos; expostos em
  `GET /v1/admin/usage` (pull) e emitidos como webhook `usage.recorded` para o seu
  sistema de billing (push, periódico).
- **Quota/plano:** cada tenant tem um plano com limites (msgs/min, instâncias máx,
  eventos/dia, dias de retenção). O admin seta plano/quota via
  `PATCH /v1/admin/tenants/{id}`; estourar limite -> `429` (rate) ou
  `403 quota_exceeded` (cota dura, ex.: instâncias máx).
- **Entrada do billing (lifecycle):** a borda ACEITA webhooks do provedor em
  `POST /v1/admin/billing/webhook` (autenticado) para aplicar plano/suspensão
  automaticamente (ex.: assinatura cancelada -> tenant suspenso, instâncias
  desconectadas). Mapeamento provedor->plano é config do operador.
- Suspensão de tenant: desconecta instâncias (mantém estado p/ re-conectar),
  bloqueia envio, mantém o dashboard acessível para regularizar.

## 10. Eventos: tipos e semântica

Tipos expostos ao tenant (mapeados do EventEnvelope do core, achatados em JSON):
`message.received` (inclui echo: `from_me` + `origin` = `sent_via_api`|`sent_via_phone`),
`message.receipt` (delivered/read/played), `message.undecryptable`,
`connection.update`, `pairing.update`, `presence.update`, `group.update`,
`contact.update`, `history.chunk` (só com backfill ligado), `gap` (transparência:
o tenant sabe que houve perda e pode reconciliar).

- **Echo sempre entregue e marcado** (transparência, filosofia do core): mensagens
  da própria conta — enviadas via API ou pelo celular do dono/outro device —
  chegam com `from_me=true` e `origin` explícito. O filtro é 100% do tenant.
- **Grupos opt-in por instância:** se `groups_enabled=false` (default), eventos de
  chats de grupo não são entregues nem contam na quota. Ligando, passam a entrar no
  fluxo e a contar.
- Todo evento ganha `event_id` (ULID, vira cursor do polling e `Last-Event-ID`),
  `instance_id`, `occurred_at`, e o payload bruto do core preservado em `raw` quando
  aplicável (nunca perder informação silenciosamente).
- **Buffer de retenção:** todos os eventos entregáveis ficam N dias (default 7, máx
  30) no Postgres da borda, indexados por (instance_id, event_id). É a fonte do
  polling, da reentrega de webhook e do catch-up de SSE/WS. History sync chunks
  grandes: metadados + blob em tabela separada, mesma retenção.

**Webhooks (eventos):**
- Entrega **at-least-once**; ordenação best-effort por instância (fila por
  instância, 1 worker por webhook). Idempotência pelo `event_id`.
- Assinatura `X-Wamux-Signature: sha256=HMAC(secret, body)` + `X-Wamux-Timestamp`
  (janela anti-replay de 5 min).
- Retry com backoff: 30s, 2m, 10m, 1h, 6h, depois 1x/6h até o evento sair da
  retenção; circuit breaker marca o webhook como `failing` no dashboard.
- Timeout de entrega 10s; 2xx = sucesso; 410 Gone desativa o webhook.
- SSRF: validar URL (bloquear redes privadas/localhost por default; allow list do
  operador).

## 11. Dashboard mínimo (v1)

Server-rendered (askama) + htmx, servido pelo próprio binário, **PT + EN**:
1. Signup/login + verificação de e-mail.
2. Lista de instâncias com status ao vivo (htmx polling/SSE).
3. Página da instância: parear (QR ao vivo via SSE), conectar/desconectar/logout,
   toggle de grupos, configurar webhooks (url/secret/filtros, estado failing), tail
   dos últimos eventos do buffer, enviar mensagem de teste.
4. API keys (criar/revogar).
5. Uso/quota do tenant (consumo vs limite do plano).
6. Admin (operador): tenants, planos/quotas, suspender, saúde do core
   (Check/GetMetrics).

## 12. Persistência e deploy

- **Postgres próprio da borda** (database separada do core; pode ser o mesmo
  servidor). Tabelas: users, tenants, plans, api_keys, instances, webhooks, events
  (retenção), webhook_deliveries, media_refs, usage_counters. Migrations sqlx. Job
  de expurgo da retenção (1x/h).
- **Colocação obrigatória com o core** (UDS): mesmo host/pod. Deploy de referência:
  docker-compose com `wamux` + `wamux-http-edge` + `postgres`, socket compartilhado
  por volume, borda no grupo dono do socket (0660). systemd como alternativa
  documentada.
- A borda é o único processo com acesso ao socket em produção. **Serve HTTP puro**;
  TLS/domínio/cert ficam num reverse proxy externo (Caddy/nginx/Traefik),
  documentado no deploy de referência.
- Config: TOML + env `WAMUX_EDGE_*` (espelha o padrão do core).

## 13. Backup e recuperação de desastre (seção dedicada)

Há **dois bancos críticos** com perfis diferentes:

- **DB do CORE (crítico, irrecuperável):** guarda o estado Signal/sessão/device de
  todas as contas. Perder = **todas as instâncias precisam re-parear** (cada tenant
  reescaneia QR). Não há como reconstruir. Requisitos: backup frequente
  (ex.: `pg_dump`/`pg_basebackup` + WAL archiving para PITR), restore **testado**
  periodicamente, backup cifrado em repouso (contém material de chave), retenção de
  backup documentada.
- **DB da BORDA (importante, parcialmente reconstruível):** users/tenants/instances/
  webhooks/quotas são críticos (perder = perder os clientes e a config). O buffer de
  eventos é efêmero por natureza (retenção curta) — perda tolerável. Backup diário +
  PITR recomendado para as tabelas de produto.

O PRD exige: estratégia de backup dos dois DBs no deploy de referência, runbook de
restore, e um teste de restore antes do go-live. O backup do DB do core é o item
número 1 de operação (a borda pode ser reconstruída do código; as chaves não).

## 14. Segurança

- Senhas: argon2id. API keys: aleatórias com prefixo público (`we_live_...`),
  armazenadas como hash, exibidas uma única vez.
- JWT curto (15 min) + refresh; revogação de refresh no logout.
- Signup aberto: verificação de e-mail obrigatória; rate limit de signup por IP;
  plano free com quota baixa como teto de abuso.
- Rate limit: token bucket por API key, default **60 msgs/min** e 600 req/min por
  tenant, ajustável por plano. 429 com Retry-After.
- Webhook SSRF: validar URL (bloquear redes privadas/localhost por default; allow
  list do operador).
- Billing webhook de entrada: autenticado (assinatura do provedor verificada).
- LGPD/privacidade: o buffer de retenção contém conteúdo de mensagens de terceiros;
  documentar a janela de retenção no ToS, expurgo automático, delete imediato no
  `DELETE /instances` e no delete de tenant. Backups do core são dados pessoais
  cifrados — incluir no inventário de tratamento.

## 15. Observabilidade

Espelha o core: tracing estruturado (1 linha por request HTTP: rota, tenant,
latência, status), `/metrics` Prometheus (requests, entregas de webhook por
status, lag de eventos, instâncias conectadas, uso por tenant), `/healthz` (inclui
Check do core via socket: a borda reporta unhealthy se o core caiu).

## 16. Licenciamento e publicação (proposta, vetável)

- Core (`wamux`): MIT OR Apache-2.0 (padrão Rust, máxima adoção).
- Borda (`wamux-http-edge`): AGPL-3.0 (protege o produto SaaS: forks fechados de
  SaaS precisam abrir o código) com opção de licença comercial sua.
- **Forja a definir na hora de publicar** (GitHub, Codeberg, GitLab, self-hosted —
  decisão adiada). READMEs com o posicionamento: "wamux é o core; wamux-http-edge é
  a borda de referência e um produto real".

## 17. Fases de implementação

**F1: fundação + caminho crítico** (o mínimo que prova o conjunto)
- Camada core-client completa: cliente UDS, política de conexão/reconexão,
  política de JID + cache de CheckOnWhatsApp, pipeline de eventos com gap recovery,
  testes contra um wamux real em socket temporário.
- Auth (signup aberto + verificação de e-mail, JWT, API keys), tenants/planos,
  instances, pareamento via SSE com QR, número-único-por-tenant.
- SendText síncrono + Idempotency-Key + buffer de eventos (com echo marcado) +
  webhooks (retry/assinatura) + polling.
- Deploy compose de referência (com reverse proxy de exemplo) + estratégia de
  backup dos dois DBs.

**F2: superfície completa de mensageria**
- SendMedia (multipart + URL), PTT com transcode, link preview fetcher,
  ephemeral, reaction/edit/delete/read/presence, mídia on-demand (`GET /media`).
- SSE + WebSocket de eventos; grupos opt-in por instância.
- Dashboard mínimo (PT + EN).

**F3: produto**
- Grupos/contatos (proxy), quotas/rate limits por plano, medição de uso +
  webhooks de billing + admin API de plano/suspensão, billing-webhook de entrada,
  hardening (SSRF, pen-test interno), docs públicas da API (OpenAPI, PT + EN),
  backfill de histórico exposto (`backfill_history` + history.chunk +
  FetchMessageHistory).

## 18. Fora de escopo (v1)

Compatibilidade Evolution API; cobrança embutida (só a superfície de billing);
multi-node/HA (1 core + 1 borda por host); chamadas de voz/vídeo;
communities/channels; mensagens interativas (botões/listas); app mobile; histórico
completo (além da retenção); equipe multi-usuário por tenant (fase 2); TLS nativo
na borda (fica no proxy).

## 19. Riscos e mitigação

| Risco | Mitigação |
|---|---|
| Ban de números (uso não-oficial) | Documentar honestamente; rate limits conservadores por default; warm-up de instância recém-pareada é responsabilidade do tenant (documentar boas práticas) |
| Perda do DB do core | Backup frequente + PITR + restore testado (§13); é o item nº1 de operação |
| Pair-code quebrado na lib 0.6.0 | v1 só QR (já decidido no core); reavaliar a cada release da whatsapp-rust |
| Mudança de contrato do core | proto/ copiado e versionado; CI da borda roda contra um wamux pinado por versão/commit |
| Webhook do tenant fora do ar | retenção 7d + retry schedule + polling como fallback universal |
| Buffer cresce demais (grupos grandes) | grupos opt-in por instância; quota de eventos/dia por plano; history.chunk em tabela separada; expurgo configurável |
| Abuso via signup aberto | verificação de e-mail, rate limit de signup, plano free com teto baixo, suspensão via admin |

## 20. Questões resolvidas (antes em aberto)

- Retenção: default **7 dias**, máx configurável **30**.
- Rate limit: default **60 msgs/min** por tenant, ajustável por plano.
- Licenças: **MIT/Apache-2.0** no core, **AGPL-3.0** na borda (+ comercial opcional).
- Webhook retry: 30s → 2m → 10m → 1h → 6h → 1x/6h até sair da retenção.
- Identificador público da instância: **ULID opaco** (`instance_id`), não slug.
