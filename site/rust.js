// The Rust snippet (#280): tonic over the Unix socket, the way the repo's own
// tools connect (crates/wamux-tools/src/socket_client.rs). prost types have
// no JSON parser, so the request JSON becomes a struct literal, named the way
// prost-build names things (snake_case modules, UpperCamel enum variants with
// the enum's own prefix stripped, `r#` on keywords).
"use strict";

(() => {
  const S = typeof module !== "undefined" ? require("./snippets.js") : WamuxSnippets;
  const KEYWORDS = new Set(["as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "trait", "true", "try", "type", "unsafe", "use", "where", "while", "abstract",
    "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield"]);
  // These cannot be raw identifiers, so prost appends an underscore instead.
  const SUFFIXED = new Set(["self", "super", "crate", "Self"]);
  const STEP = "    ";

  function ident(name) {
    const snake = S.snake(name);
    if (SUFFIXED.has(snake)) return `${snake}_`;
    return KEYWORDS.has(snake) ? `r#${snake}` : snake;
  }

  function typeIdent(name) {
    return S.upperCamel(S.snake(name));
  }

  /** `wamux.v1.Outer.Inner` -> `pb::outer::Inner`. */
  function rustPath(fullName) {
    const parts = S.localPath(fullName);
    const modules = parts.slice(0, -1).map((p) => ident(p));
    return ["pb", ...modules, typeIdent(parts[parts.length - 1])].join("::");
  }

  /** The module prost puts a message's oneof enums in: `pb::account_ref`. */
  function oneofModule(fullName) {
    return ["pb", ...S.localPath(fullName).map((p) => ident(p))].join("::");
  }

  function enumVariant(enumFullName, valueName) {
    const prefix = typeIdent(S.localPath(enumFullName).pop());
    const camel = S.upperCamel(valueName);
    const stripped = camel.startsWith(prefix) ? camel.slice(prefix.length) : camel;
    return /^[A-Z]/.test(stripped) ? stripped : camel;
  }

  function rustString(text) {
    let out = "";
    for (const ch of text) {
      const code = ch.codePointAt(0);
      const named = { "\\": "\\\\", '"': '\\"', "\n": "\\n", "\r": "\\r", "\t": "\\t" }[ch];
      if (named) out += named;
      else if (code < 0x20 || code === 0x7f) out += `\\u{${code.toString(16)}}`;
      else out += ch;
    }
    return `"${out}"`;
  }

  function floatText(text) {
    return /[.eE]/.test(text) ? text : `${text}.0`;
  }

  /** One value of a non-message field; `owned` spells a String without relying on inference. */
  function scalar(index, field, value, where, owned) {
    if (field.kind === "string") return `${rustString(value)}.${owned ? "to_string()" : "into()"}`;
    if (field.kind === "bool") return String(value);
    if (field.kind === "bytes") {
      const bytes = S.bytesOf(value, where);
      return bytes.length ? `vec![${[...bytes].join(", ")}]` : "Vec::new()";
    }
    if (field.kind === "enum") {
      const found = S.enumValueOf(index, field.typeName, value, where);
      return `${rustPath(field.typeName)}::${enumVariant(field.typeName, found.name)} as i32`;
    }
    const text = S.numberText(field.kind, value, where);
    return ["double", "float"].includes(field.kind) ? floatText(text) : text;
  }

  function single(index, field, value, indent, where, owned) {
    if (field.kind === "message") return messageLiteral(index, field.typeName, value, indent, where);
    return scalar(index, field, value, where, owned);
  }

  function listLiteral(index, field, items, indent, where) {
    if (items.length === 0) return "Vec::new()";
    const inner = indent + STEP;
    const values = items.map((item, i) => single(index, field, item, inner, `${where}[${i}]`, true));
    if (field.kind !== "message") return `vec![${values.join(", ")}]`;
    return `vec![\n${values.map((v) => `${inner}${v},`).join("\n")}\n${indent}]`;
  }

  function mapKey(kind, key) {
    if (kind === "string") return `${rustString(key)}.to_string()`;
    return key;
  }

  function mapLiteral(index, field, entries, indent, where) {
    const pairs = Object.entries(entries);
    if (pairs.length === 0) return "Default::default()";
    const inner = indent + STEP;
    const lines = pairs.map(([k, v]) => {
      const value = single(index, field, v, inner, `${where}.${k}`, true);
      return `${inner}(${mapKey(field.mapKey, k)}, ${value}),`;
    });
    return `[\n${lines.join("\n")}\n${indent}]\n${indent}.into_iter()\n${indent}.collect()`;
  }

  function fieldValue(index, field, value, indent, where) {
    if (field.label === "repeated") return listLiteral(index, field, value, indent, where);
    if (field.label === "map") return mapLiteral(index, field, value, indent, where);
    const inner = single(index, field, value, indent, where, false);
    return field.label === "optional" || field.kind === "message" ? `Some(${inner})` : inner;
  }

  function oneofValue(index, message, field, value, indent, where) {
    const variant = `${oneofModule(message.fullName)}::${S.upperCamel(field.oneof)}::${S.upperCamel(field.name)}`;
    return `Some(${variant}(${single(index, field, value, indent, where, false)}))`;
  }

  /** proto3 leaves a default scalar off the wire, so the literal leaves it to `..Default::default()`. */
  function isDefault(index, field, value) {
    if (field.oneof || field.label === "optional" || value === null) return value === null;
    if (field.label === "repeated") return value.length === 0;
    if (field.label === "map") return Object.keys(value).length === 0;
    if (field.kind === "message") return false;
    if (field.kind === "enum") return S.enumValueOf(index, field.typeName, value, "").number === 0;
    if (field.kind === "bytes" || field.kind === "string") return value === "";
    if (field.kind === "bool") return value === false;
    return Number(value) === 0;
  }

  function fieldLines(index, message, value, inner, where) {
    const lines = [];
    for (const field of message.fields) {
      const key = [field.jsonName, field.name].find((k) => k in value);
      if (key === undefined || isDefault(index, field, value[key])) continue;
      const at = `${where}.${key}`;
      const text = field.oneof
        ? oneofValue(index, message, field, value[key], inner, at)
        : fieldValue(index, field, value[key], inner, at);
      lines.push({ name: ident(field.oneof || field.name), text });
    }
    return lines;
  }

  function messageLiteral(index, typeName, value, indent, where) {
    S.checkMessage(index, typeName, value, where);
    const message = S.messageOf(index, typeName);
    const inner = indent + STEP;
    const lines = fieldLines(index, message, value, inner, where);
    const total = message.fields.filter((f) => !f.oneof).length + message.oneofs.length;
    const body = lines.map((l) => `${inner}${l.name}: ${l.text},`);
    if (lines.length < total) body.push(`${inner}..Default::default()`);
    if (body.length === 0) return `${rustPath(typeName)} {}`;
    if (lines.length === 0) return `${rustPath(typeName)}::default()`;
    return `${rustPath(typeName)} {\n${body.join("\n")}\n${indent}}`;
  }

  function rustCall(call) {
    const rpc = `client.${S.snake(call.method.name)}`;
    if (call.shape === "unary") {
      return [`let reply = ${rpc}(request).await?.into_inner();`, 'println!("{reply:#?}");'];
    }
    if (call.shape === "server") {
      return [`let mut stream = ${rpc}(request).await?.into_inner();`,
        "while let Some(reply) = stream.message().await? {", '    println!("{reply:#?}");', "}"];
    }
    return [...rustFrames(call), `let reply = ${rpc}(tokio_stream::iter(frames)).await?.into_inner();`,
      'println!("{reply:#?}");'];
  }

  function rustFrames(call) {
    const outer = rustPath(call.method.inputType);
    if (!call.parts) return [`let frames: Vec<${outer}> = vec![request];`];
    const { message, oneof, chunk } = call.parts;
    const variant = `${oneofModule(message.fullName)}::${S.upperCamel(oneof)}::${S.upperCamel(chunk.name)}`;
    const others = message.fields.filter((f) => !f.oneof).length + message.oneofs.length > 1;
    const rest = others ? [`${STEP}${STEP}..Default::default()`] : [];
    return ["let media: Vec<u8> = std::fs::read(MEDIA_FILE)?;", `let mut frames: Vec<${outer}> = vec![request];`,
      `frames.extend(media.chunks(CHUNK_SIZE).map(|chunk| ${outer} {`,
      `${STEP}${ident(oneof)}: Some(${variant}(chunk.to_vec())),`, ...rest, "}));"];
  }

  function rust(call) {
    const client = `${typeIdent(call.service.name)}Client`;
    const consts = [`const SOCKET: &str = ${rustString(call.socket)};`];
    if (call.shape === "client" && call.parts) {
      consts.push(`const MEDIA_FILE: &str = "${S.MEDIA_FILE}";`, `const CHUNK_SIZE: usize = ${S.CHUNK_SIZE};`);
    }
    const request = messageLiteral(call.index, call.method.inputType, call.request, STEP, "request");
    const body = ["// tonic wants a URI; the connector ignores it and dials the socket.",
      'let channel = Endpoint::try_from("http://[::1]:50051")?',
      "    .connect_with_connector(service_fn(|_: Uri| async {",
      "        Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(SOCKET).await?))", "    }))",
      "    .await?;", `let mut client = ${client}::new(channel);`, "", `let request = ${request.trimStart()};`,
      ...rustCall(call), "Ok(())"];
    const lines = ["use hyper_util::rt::TokioIo;", "use tokio::net::UnixStream;",
      "use tonic::transport::{Endpoint, Uri};", "use tower::service_fn;", "", "pub mod pb {",
      '    tonic::include_proto!("wamux.v1");', "}", "",
      `use pb::${ident(call.service.name)}_client::${client};`, "", ...consts, "", "#[tokio::main]",
      "async fn main() -> Result<(), Box<dyn std::error::Error>> {",
      ...body.map((l) => (l ? `${STEP}${l}` : "")), "}"];
    return `${lines.join("\n")}\n`;
  }

  function rustStubs() {
    return ["git clone https://github.com/LLawli/wamux", "# protoc on PATH (or PROTOC=/path/to/protoc), then:",
      "cargo add tonic@0.14 tonic-prost@0.14 prost@0.14 tower@0.5 tokio-stream@0.1",
      "cargo add tokio@1 --features full", "cargo add hyper-util@0.1 --features tokio",
      "cargo add --build tonic-prost-build@0.14", "", "// build.rs", "use std::path::PathBuf;", "",
      "fn main() -> Result<(), Box<dyn std::error::Error>> {", `    let dir = PathBuf::from("${S.PROTO_DIR}");`,
      "    let mut protos = Vec::new();", "    for entry in std::fs::read_dir(&dir)? {", "        let path = entry?.path();",
      '        if path.extension().is_some_and(|e| e == "proto") {', "            protos.push(path);", "        }", "    }",
      "    tonic_prost_build::configure()", "        .build_server(false)", "        .compile_protos(&protos, &[dir])?;",
      "    Ok(())", "}"].join("\n");
  }

  S.STUBS.rust = rustStubs;
  S.GENERATORS.rust = rust;
  S.rustLiteral = (index, typeName, value) => messageLiteral(index, typeName, value, "", "request");
})();
