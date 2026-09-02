# Breaking change para a borda — 2026-09-02 (issue #20)

Uma mudança só, e ela troca o que um RPC existente **faz**, não a forma dele.
A borda compila igual e o comportamento muda: é o pior tipo de mudança para
passar despercebida, então está aqui sozinha.

Arquivo alterado: `proto/messaging.proto`.

| # | Mudança | Impacto na borda |
|---|---------|------------------|
| 1 | `MarkRead` passa a mandar recibo de leitura em vez de sincronizar estado | **BREAKING** (mesma assinatura, outro efeito) |
| 2 | `MarkChatRead` é novo e faz o que o `MarkRead` fazia | Aditivo |

---

## 1. `MarkRead` agora manda o recibo (BREAKING)

**Antes:** `MarkRead` chamava `chat_actions().mark_chat_as_read()`, que é uma
mutação de app-state — irmã de arquivar e fixar. Ela marca a conversa como lida
**nos aparelhos da própria conta** e não põe recibo nenhum na rede. Os campos
`message_ids` e `sender` da requisição eram lidos e descartados.

O resultado: os tiques da outra pessoa nunca ficavam azuis, e nada dizia isso.
O RPC devolve `Empty` nos dois casos.

**Agora:** `MarkRead` chama `client.mark_as_read(chat, sender, message_ids)`, que
emite o stanza de recibo. Os três campos passam a ser usados.

```proto
message MarkReadRequest {
  AccountRef account = 1;
  Jid chat = 2;
  repeated string message_ids = 3;  // agora lido
  Jid sender = 4;                   // agora lido
}
```

**O que a borda precisa fazer:**

- Se você chamava `MarkRead` **para mandar recibo**: nada. Passou a funcionar.
  Confira que está preenchendo `message_ids` e, em grupo, `sender`.
- Se você chamava `MarkRead` **pelo efeito nos próprios aparelhos**: troque para
  `MarkChatRead`, que preserva exatamente o comportamento antigo.
- Se você quer os dois, como o app oficial faz: chame os dois. O core não
  compõe as duas operações por você, de propósito — juntá-las tiraria a
  possibilidade de pedir uma sem a outra.

**`message_ids` vazio não é erro.** A biblioteca retorna antes de montar o
stanza, então uma chamada sem nada a reconhecer não custa nada e não falha.

**`sender` vazio é ausência, não erro.** Em DM o autor é a própria conversa e o
campo não se aplica. Mandar `Jid { value: "" }` ou omitir o campo dão no mesmo.

## 2. `MarkChatRead` (aditivo)

```proto
rpc MarkChatRead(MarkReadRequest) returns (Empty);
```

Marca a conversa lida nos aparelhos da própria conta. Lê só `account` e `chat`
— o estado de leitura de uma conversa é por conversa, não por mensagem. É o
`MarkRead` de antes desta mudança, com o nome que descreve o que ele faz, do
lado do `MarkUnread` que já existia para a direção oposta.

## Duas coisas que o core NÃO consegue te dizer

Ambas são do upstream expor, e valem porque as duas se parecem com o bug que
esta mudança corrige:

1. **Qual recibo saiu.** Quando a conta tem "confirmações de leitura" desligada
   nas configurações de privacidade, o WhatsApp manda `read-self` em vez de
   `read` — e **só em DM privada**; leitura em grupo sai como `read` de qualquer
   forma. A biblioteca lê essa flag de um snapshot de device que ela não expõe
   (`persistence_manager` é `pub(crate)` e não há getter), então o core não tem
   como reportar qual dos dois mandou.

2. **Se o recibo chegou.** Não há confirmação de volta. Silêncio aqui não é
   prova de nada, nos dois sentidos.

O que serve de verificação é um segundo aparelho: abra a conversa, chame
`MarkRead` com os ids das mensagens que a outra pessoa mandou, e pergunte a ela
se os tiques ficaram azuis. **Marca d'água no lado de quem chama não é prova** —
foi exatamente isso que escondeu o bug: a coluna `remote_read_ms` do
`wamux-omarchy` avançava a cada chamada, porque é escrita quando o relay aceita,
o que diz que a chamada aconteceu e não que um recibo saiu.
