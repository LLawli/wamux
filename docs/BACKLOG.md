# wamux — Backlog de bugs / dívidas

Status: **todos os itens abertos foram corrigidos** (2026-06-09). Os `[FIXED]` ficam por
rastreabilidade. Cada item tem: onde, sintoma, causa, conserto e (quando aplicável) como
foi verificado.

## Abertos

_(nenhum no momento)_

## Corrigidos

### B7 — `[FIXED]` Assertion `saw_gap` do load test era flaky (timing)
- **Onde:** `tests/load_multi_account.rs::no_head_of_line_blocking_and_gap_under_load`.
- **Sintoma:** `assert!(saw_gap, ...)` falhava ~50% das vezes (passava no re-run). Descoberto ao
  verificar o B5.
- **Causa:** a `flood` (20k eventos) rodava como task concorrente e o gap-check às vezes começava
  **antes** do broadcast transbordar de verdade → o forwarder do slow não tinha perdido eventos
  ainda → nenhum marcador de gap na janela.
- **Fix:** `flood.await` **antes** do gap-check. `broadcast::send` nunca bloqueia, então a flood
  termina em ~ms e o lag fica determinístico (o forwarder do slow, travado no onward channel não
  drenado, já perdeu eventos). Removido o `flood.abort()` final (já completou).
- **Verificado:** 4/4 runs verdes (antes ~50%).

### B5 — `[FIXED]` Cleanup do load test deixava contas órfãs
- **Onde:** `tests/load_multi_account.rs`.
- **Sintoma:** runs interrompidos/panic deixavam contas `load-...` no Postgres (chegou a **200**
  acumuladas).
- **Causa:** o cleanup só rodava no caminho feliz (loop de 200 deletes no fim); um panic pulava.
- **Fix:** helper `sweep_load_orphans` (1 `DELETE ... LIKE 'load-%'`) chamado **no setup**
  (self-heal de runs abortados) **e no fim** (tidy). O setup-sweep limita a acumulação a no máximo
  um run abortado, que o próximo run limpa.
- **Verificado:** setup-sweep apagou as 200 órfãs existentes; após um run bem-sucedido, contagem
  de `load-%` = **0**.

### B4 — `[FIXED]` Teste M2a só checava contador de frames, não que um nó foi parseado
- **Onde:** `tests/stress_handshake.rs::registered_client_logs_in_and_talks_over_transport` +
  `src/stress/mock_wa_server.rs`.
- **Sintoma:** o teste passava verde **mesmo com o B1 ativo** (mock decifrava mas nunca parseava).
- **Causa:** `post_login_frames` incrementa **antes** do `unmarshal_ref`; o teste só assertava
  `post_login_frames() >= 1`.
- **Fix:** novo contador `parsed_nodes` no mock (incrementa só quando `unmarshal_ref` dá certo) +
  accessor `parsed_nodes()`; o M2a agora asserta `parsed_nodes() >= 1`.
- **Verificado:** revertendo o fix do B1 (alimentar `unmarshal_ref` com o buffer ainda empacotado),
  o M2a **FALHA** ("the server must actually parse a post-login node"); com o fix, passa.

### B6 — `[FECHADO]` "Falha no 1º envio recém-pareado" era o JID errado (B3)
- **Onde:** fluxo live (`src/bin/stress_live.rs` M4).
- **Resolução:** o re-run com o JID correto (B3) entregou todos os probes (0 timeouts); as falhas
  "No pre-key bundle"/"479" do 1º run eram o número incompleto, não falta de sync. Resíduo: um
  envio no **instante exato** de um pareamento fresco (com JID correto) ainda não foi medido — se
  algum dia o 1º probe pós-QR falhar, reabrir para investigar warm-up de sessão.

### B1 — `[FIXED]` Mock não desempacotava o flag byte do frame (recv)
- **Onde:** `src/stress/mock_wa_server.rs` (read loop pós-login).
- **Sintoma:** keepalive nunca pingava → dead-socket reconnect; M3 não seguraria as conexões.
- **Causa:** payload decifrado é `[flag_byte][nó]` (flag&2 ⇒ zlib), o mesmo envelope que o
  `wacore_binary::Encoder` escreve no send; o mock chamava `unmarshal_ref` direto → parse falhava
  em silêncio → IQs pós-login (usync) sem resposta → waiter eterno → ping pulado (`has_pending`).
- **Fix:** `wacore_binary::util::unpack` antes do `unmarshal_ref`. (Agora coberto pelo B4.)

### B2 — `[FIXED]` `stress_live` esperava `Connected` (socket) em vez do login
- **Onde:** `src/bin/stress_live.rs::connect_real`.
- **Sintoma:** num run de QR fresco, o bin seguia pros probes antes de parear/logar.
- **Causa:** a lib emite `Event::Connected` quando o **socket** sobe, **antes** do pareamento.
- **Fix:** esperar `client.is_logged_in()` (não o estado `Connected`).

### B3 — `[FIXED]` Destino M4 sem código de país
- **Onde:** `src/bin/stress_live.rs` (`ALLOWED_DEST`) + regra ai-memory `_rules/live-whatsapp-sends.md`.
- **Sintoma:** envio rejeitado — "479 SmaxInvalid (wrong JID format)" e "No pre-key bundle".
- **Causa:** JID `11999999999@c.us` sem o `55` (Brasil) — número não-E.164 válido.
- **Fix:** `5511999999999@c.us` (55 BR + DDD + número). Único destino permitido nos sends live.
