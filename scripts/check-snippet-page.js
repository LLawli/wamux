#!/usr/bin/env node
// #280: the snippet page (site/) renders every RPC of site/schema.json in
// every language. A contract change the generators cannot render (a new
// client-stream shape, a type they do not know) must fail here, not on the
// published page. Pure node, no packages.
//
// Usage: scripts/check-snippet-page.js
"use strict";

const path = require("node:path");
const assert = require("node:assert/strict");

const SITE = path.join(__dirname, "..", "site");
const S = require(path.join(SITE, "snippets.js"));
require(path.join(SITE, "languages.js"));
require(path.join(SITE, "rust.js"));

const LANGS = ["grpcurl", "python", "go", "rust", "js"];
// An absolute floor, not a count derived from the schema: an empty or
// truncated schema.json must not pass as "nothing to render".
const MIN_RPCS = 80;
const SOCKET = "/run/wamux.sock";

// The Send text preset as grpcurl, pinned byte for byte: the first send a
// newcomer copies.
const SEND_TEXT_GRPCURL = `grpcurl -unix -plaintext -d '{
  "account": {
    "externalRef": "my-account"
  },
  "to": {
    "value": "<number>@s.whatsapp.net"
  },
  "text": "Hello from wamux"
}' \\
  /run/wamux.sock wamux.v1.MessagingService/SendText
`;

function methodOf(index, serviceName, methodName) {
  const service = index.services.find((s) => s.name === serviceName);
  assert.ok(service, `service ${serviceName} is missing`);
  const method = service.methods.find((m) => m.name === methodName);
  assert.ok(method, `${serviceName}/${methodName} is missing`);
  return { service, method };
}

function renderAll(index, service, method, request) {
  const call = S.callOf(index, service, method, request, SOCKET);
  if (method.clientStreaming) assert.ok(call.parts, `${method.name}: no header/bytes oneof to stream`);
  for (const lang of LANGS) {
    const where = `${service.name}/${method.name} (${lang})`;
    const rpc = lang === "rust" ? `.${S.snake(method.name)}(` : method.name;
    assert.ok(S.generate(lang, call).includes(rpc), `${where}: snippet does not call ${rpc}`);
    assert.ok(S.STUBS[lang](call).length > 0, `${where}: no stubs note`);
  }
}

function checkEveryRpc(index) {
  let rpcs = 0;
  for (const service of index.services) {
    for (const method of service.methods) {
      renderAll(index, service, method, S.skeletonOf(index, method.inputType));
      rpcs += 1;
    }
  }
  assert.ok(rpcs >= MIN_RPCS, `only ${rpcs} RPCs rendered, expected at least ${MIN_RPCS}`);
  return rpcs;
}

function checkPresets(index) {
  for (const preset of S.PRESETS) {
    const { service, method } = methodOf(index, preset.service, preset.method);
    renderAll(index, service, method, preset.request);
  }
  const { service, method } = methodOf(index, "MessagingService", "SendText");
  const sendText = S.PRESETS.find((p) => p.method === "SendText");
  const call = S.callOf(index, service, method, sendText.request, SOCKET);
  assert.equal(S.generate("grpcurl", call), SEND_TEXT_GRPCURL);
}

function main() {
  const index = S.indexSchema(require(path.join(SITE, "schema.json")));
  const rpcs = checkEveryRpc(index);
  checkPresets(index);
  console.log(`snippet page renders ${rpcs} RPCs in ${LANGS.length} languages, ${S.PRESETS.length} presets`);
}

try {
  main();
} catch (error) {
  console.error(`ERROR: ${error.message}`);
  process.exit(1);
}
