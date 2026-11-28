// Client snippet generators for the wamux socket (#280). Pure functions over
// schema.json (generated from the .proto files by `proto_schema`), so the
// same code runs in the page and under node for checks. No DOM in here.
"use strict";

const WamuxSnippets = (() => {
  const CHUNK_SIZE = 64 * 1024;
  // A multiple of 4, so each line of `base64 -w` decodes on its own.
  const BASE64_LINE = 65536;
  const MEDIA_FILE = "photo.jpg";
  const PROTO_DIR = "wamux/crates/wamux-proto/proto";
  const GO_PACKAGE = "example.com/myapp/wamuxv1";
  const DEFAULT_ACCOUNT = "my-account";
  const INT64_KINDS = ["int64", "uint64", "sint64", "fixed64", "sfixed64"];
  const FLOAT_KINDS = ["double", "float"];
  // Presence-sensitive fields (a nested message, a proto3 `optional`) start
  // out of the skeleton: an empty one is not the same as none (an empty
  // `sticker_audio` is InvalidArgument). The addressing types are the
  // exception: a request that has one needs it, and an empty Jid where it is
  // optional reads as absent.
  const ADDRESSING = {
    "wamux.v1.AccountRef": () => ({ externalRef: DEFAULT_ACCOUNT }),
    "wamux.v1.Jid": () => ({ value: "" }),
  };

  const RECIPIENT = "<number>@s.whatsapp.net";
  const ACCOUNT = { externalRef: DEFAULT_ACCOUNT };
  // The first steps, in order: create an account, pair it, send, listen.
  const PRESETS = [
    { label: "Create account", service: "AccountService", method: "CreateAccount", request: ACCOUNT },
    { label: "Pair with QR", service: "AccountService", method: "PairWithQr", request: { account: ACCOUNT } },
    { label: "Send text", service: "MessagingService", method: "SendText",
      request: { account: ACCOUNT, to: { value: RECIPIENT }, text: "Hello from wamux" } },
    { label: "Events", service: "EventService", method: "SubscribeEvents", request: { account: ACCOUNT } },
  ];

  // ---- schema index ------------------------------------------------------

  function indexSchema(schema) {
    const messages = new Map(schema.messages.map((m) => [m.fullName, m]));
    const enums = new Map(schema.enums.map((e) => [e.fullName, e]));
    return { schema, messages, enums, services: schema.services };
  }

  function messageOf(index, fullName) {
    const message = index.messages.get(fullName);
    if (!message) throw new Error(`unknown message ${fullName}`);
    return message;
  }

  function packageOf(fullName) {
    const parts = fullName.split(".");
    return parts.filter((p) => /^[a-z]/.test(p)).join(".");
  }

  /** Names below the package: "wamux.v1.Outer.Inner" -> ["Outer", "Inner"]. */
  function localPath(fullName) {
    const pkg = packageOf(fullName);
    return fullName.slice(pkg.length + 1).split(".");
  }

  // ---- skeleton ----------------------------------------------------------

  function defaultOf(index, field) {
    if (field.label === "repeated") return [];
    if (field.label === "map") return {};
    if (field.kind === "message") return skeletonOf(index, field.typeName);
    if (field.kind === "enum") return index.enums.get(field.typeName).values[0].name;
    if (field.kind === "string" || field.kind === "bytes") return "";
    if (field.kind === "bool") return false;
    if (INT64_KINDS.includes(field.kind)) return "0";
    return 0;
  }

  /**
   * In the skeleton unless presence-sensitive, or a list, map or bytes (empty
   * is their default and only adds noise; the field list offers them). A
   * oneof contributes its first member.
   */
  function inSkeleton(field, firstOfOneof) {
    if (field.oneof) return firstOfOneof;
    if (field.label !== "singular" || field.kind === "bytes") return false;
    if (field.kind === "message") return field.typeName in ADDRESSING;
    return true;
  }

  function skeletonOf(index, typeName, path = []) {
    if (typeName in ADDRESSING) return ADDRESSING[typeName]();
    if (path.includes(typeName)) return {};
    const body = {};
    const seenOneofs = new Set();
    for (const field of messageOf(index, typeName).fields) {
      const first = field.oneof ? !seenOneofs.has(field.oneof) : false;
      if (field.oneof) seenOneofs.add(field.oneof);
      if (!inSkeleton(field, first)) continue;
      body[field.jsonName] = field.kind === "message" && field.label === "singular"
        ? skeletonOf(index, field.typeName, [...path, typeName])
        : defaultOf(index, field);
    }
    return body;
  }

  // ---- request checking --------------------------------------------------

  function fieldByKey(message, key) {
    const field = message.fields.find((f) => f.jsonName === key || f.name === key);
    if (!field) throw new Error(`${message.name} has no field "${key}"`);
    return field;
  }

  /** Throws with the offending path when `value` is not a `typeName` in proto3 JSON. */
  function checkMessage(index, typeName, value, where) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new Error(`${where}: expected an object for ${typeName}`);
    }
    const message = messageOf(index, typeName);
    const oneofs = new Map();
    for (const [key, item] of Object.entries(value)) {
      const field = fieldByKey(message, key);
      if (field.oneof && item !== null) {
        if (oneofs.has(field.oneof)) throw new Error(`${where}: oneof "${field.oneof}" has both "${oneofs.get(field.oneof)}" and "${key}"`);
        oneofs.set(field.oneof, key);
      }
      checkField(index, field, item, `${where}.${key}`);
    }
  }

  function checkField(index, field, value, where) {
    if (value === null) return;
    if (field.label === "repeated") {
      if (!Array.isArray(value)) throw new Error(`${where}: expected a list`);
      value.forEach((item, i) => checkSingle(index, field, item, `${where}[${i}]`));
      return;
    }
    if (field.label === "map") {
      if (typeof value !== "object" || Array.isArray(value)) throw new Error(`${where}: expected an object (map)`);
      Object.entries(value).forEach(([k, item]) => checkSingle(index, field, item, `${where}.${k}`));
      return;
    }
    checkSingle(index, field, value, where);
  }

  function checkSingle(index, field, value, where) {
    if (field.kind === "message") return checkMessage(index, field.typeName, value, where);
    if (field.kind === "enum") return enumValueOf(index, field.typeName, value, where);
    if (field.kind === "bool" && typeof value !== "boolean") throw new Error(`${where}: expected true or false`);
    if (field.kind === "string" && typeof value !== "string") throw new Error(`${where}: expected a string`);
    if (field.kind === "bytes") return bytesOf(value, where);
    if (!["bool", "string"].includes(field.kind)) numberText(field.kind, value, where);
  }

  function enumValueOf(index, typeName, value, where) {
    const values = index.enums.get(typeName).values;
    const found = values.find((v) => v.name === value || v.number === value);
    if (!found) throw new Error(`${where}: ${JSON.stringify(value)} is not a ${typeName} value`);
    return found;
  }

  function bytesOf(value, where) {
    if (typeof value !== "string") throw new Error(`${where}: bytes are a base64 string`);
    const standard = value.replace(/-/g, "+").replace(/_/g, "/");
    try {
      return Uint8Array.from(atob(standard), (c) => c.charCodeAt(0));
    } catch (_) {
      throw new Error(`${where}: not valid base64`);
    }
  }

  function numberText(kind, value, where) {
    const text = typeof value === "number" ? String(value) : value;
    const pattern = FLOAT_KINDS.includes(kind) ? /^-?\d+(\.\d+)?([eE][-+]?\d+)?$/ : /^-?\d+$/;
    if (typeof text !== "string" || !pattern.test(text)) throw new Error(`${where}: expected a ${kind}`);
    if (kind.startsWith("u") || kind.startsWith("fixed")) {
      if (text.startsWith("-")) throw new Error(`${where}: ${kind} cannot be negative`);
    }
    return text;
  }

  // ---- call shape ----------------------------------------------------------

  /** The header and bytes members of a client-streamed chunk's oneof, when it has that shape. */
  function streamPartsOf(index, typeName) {
    const message = messageOf(index, typeName);
    for (const oneof of message.oneofs) {
      const members = message.fields.filter((f) => f.oneof === oneof);
      const header = members.find((f) => f.kind === "message");
      const chunk = members.find((f) => f.kind === "bytes");
      if (header && chunk) return { message, oneof, header, chunk };
    }
    return null;
  }

  function callOf(index, service, method, request, socket) {
    checkMessage(index, method.inputType, request, method.inputType.split(".").pop());
    const input = messageOf(index, method.inputType);
    const shape = method.clientStreaming ? "client" : method.serverStreaming ? "server" : "unary";
    const parts = method.clientStreaming ? streamPartsOf(index, method.inputType) : null;
    return { index, service, method, input, request, socket, shape, parts };
  }

  // ---- text helpers --------------------------------------------------------

  function pretty(value, indent = "") {
    return JSON.stringify(value, null, 2).replace(/\n/g, `\n${indent}`);
  }

  function snake(name) {
    return name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2").toLowerCase();
  }

  function upperCamel(name) {
    return name.split(/_+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1).toLowerCase()).join("");
  }

  function goCamel(name) {
    return name.split("_").filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join("");
  }

  function shellQuote(text) {
    return `'${text.replace(/'/g, "'\\''")}'`;
  }

  function moduleOf(file) {
    return file.replace(/\.proto$/, "").replace(/[/.-]/g, "_");
  }

  function generate(lang, call) {
    const generator = GENERATORS[lang];
    if (!generator) throw new Error(`no generator for ${lang}`);
    return generator(call);
  }

  const api = {
    PRESETS, inSkeleton, CHUNK_SIZE, BASE64_LINE, MEDIA_FILE, PROTO_DIR, GO_PACKAGE, DEFAULT_ACCOUNT, INT64_KINDS,
    indexSchema, messageOf, localPath, skeletonOf, defaultOf, checkMessage, enumValueOf, bytesOf,
    numberText, streamPartsOf, callOf, pretty, snake, upperCamel, goCamel, shellQuote, moduleOf, generate,
  };
  const GENERATORS = {};
  api.GENERATORS = GENERATORS;
  return api;
})();

if (typeof module !== "undefined") module.exports = WamuxSnippets;
