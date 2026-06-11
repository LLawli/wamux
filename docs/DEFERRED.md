# Decisões adiadas (deliberadamente NÃO implementadas)

Registro do que foi avaliado em code-review e adiado de propósito, com o
motivo e o gatilho para revisitar. Não é backlog de bugs: nada aqui está
quebrado hoje.

## `created_tx`: capacidade 64 hardcoded (src/state/account_registry.rs)

**O que é.** O canal broadcast que notifica streams all-accounts sobre contas
recém-criadas tem capacidade fixa de 64, enquanto a capacidade irmã do
broadcast de eventos por conta é injetada via config (`RegistryTuning.
broadcast_capacity`, promovida de hardcode para config no Sprint 3).

**Por que foi adiado (code-review de 2026-06-11).**
- O hardcode é uma escolha documentada no próprio campo ("account creation is
  a rare, edge-driven action, never a hot path"), com contrato de recuperação
  definido (marker `subscription_gap` + reconcile via `ListAccounts`).
- O lag exige um burst de >64 creates ENQUANTO a task follower está starved;
  o trabalho por item do follower (lookup em HashSet + `tokio::spawn`) é
  ordens de magnitude mais rápido que o INSERT no Postgres que cada create
  faz, então import em massa sozinho não dispara o gap.
- A regra de DI do CLAUDE.md cobre "socket paths, modes, endpoints, feature
  flags", não capacidades de canal; outros canais do repo também têm
  capacidade fixa (mpsc 256 do stream de eventos, mpsc 16 do account_service,
  mpsc 8 do media_service). "Toda capacidade vira config" não é a norma do
  projeto; só o broadcast do hot path foi promovido.

**Quando revisitar.** Na próxima mexida real em `RegistryTuning` ou no módulo
`account_registry`: promover o 64 para um campo `created_channel_capacity`
(default 64) custa pouco no embalo e elimina a assimetria com
`broadcast_capacity`. Também revisitar antes se algum deployment fizer
provisionamento em massa com streams all-accounts abertos e os markers
`subscription_gap` aparecerem nos logs do edge.
