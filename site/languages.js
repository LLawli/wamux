// grpcurl, Python, Go and JavaScript snippets (#280). Each one sends the
// request JSON as written: proto3 JSON is what grpcurl, protobuf's
// json_format, protojson and proto-loader all read, so the editor's text is
// the request in every language. Rust has no JSON parser in prost; it gets a
// struct literal from rust.js.
"use strict";

(() => {
  const S = typeof module !== "undefined" ? require("./snippets.js") : WamuxSnippets;

  function socketArg(socket) {
    return /^[\w./-]+$/.test(socket) ? socket : S.shellQuote(socket);
  }

  // ---- grpcurl -------------------------------------------------------------

  function grpcurl(call) {
    const target = `${call.service.fullName}/${call.method.name}`;
    const tail = `${socketArg(call.socket)} ${target}`;
    if (call.shape !== "client") {
      return `grpcurl -unix -plaintext -d ${S.shellQuote(S.pretty(call.request))} \\\n  ${tail}\n`;
    }
    const lines = ["{", `  echo ${S.shellQuote(JSON.stringify(call.request))}`];
    if (call.parts) {
      const wrap = `s/.*/{"${call.parts.chunk.jsonName}":"&"}/`;
      lines.push(`  base64 -w ${S.BASE64_LINE} ${S.MEDIA_FILE} | sed '${wrap}'`);
    }
    lines.push(`} | grpcurl -unix -plaintext -d @ ${tail}`);
    return `${lines.join("\n")}\n`;
  }

  // ---- Python --------------------------------------------------------------

  function pyString(text) {
    if (!text.includes('"""') && !text.includes("\\")) return `"""\n${text}\n"""`;
    return JSON.stringify(text);
  }

  function pyClass(call, fullName) {
    const message = S.messageOf(call.index, fullName);
    return `${S.moduleOf(message.file)}_pb2.${S.localPath(fullName).join(".")}`;
  }

  function pyImports(call) {
    const modules = new Set([S.moduleOf(call.service.file), S.moduleOf(call.input.file)]);
    const lines = [...modules].sort().map((m) => `import ${m}_pb2`);
    lines.push(`import ${S.moduleOf(call.service.file)}_pb2_grpc`);
    return lines.sort().join("\n");
  }

  function pyCall(call) {
    const rpc = `stub.${call.method.name}`;
    const print = "print(json_format.MessageToJson(reply))";
    if (call.shape === "server") {
      return [`for reply in ${rpc}(request):`, `    ${print}`];
    }
    if (call.shape === "client") return [`reply = ${rpc}(frames())`, print];
    return [`reply = ${rpc}(request)`, print];
  }

  function pyFrames(call) {
    const cls = pyClass(call, call.method.inputType);
    const lines = ["", "", "def frames():", `    yield json_format.Parse(REQUEST, ${cls}())`];
    if (call.parts) {
      lines.push(`    with open(MEDIA_FILE, "rb") as media:`, "        while chunk := media.read(CHUNK_SIZE):",
        `            yield ${cls}(${call.parts.chunk.name}=chunk)`);
    }
    return lines;
  }

  function python(call) {
    const streamed = call.shape === "client";
    const head = ["import grpc", "from google.protobuf import json_format", "", pyImports(call), "",
      `SOCKET = ${JSON.stringify(`unix://${call.socket}`)}`,
      "# grpcio sends the socket path as :authority and the daemon resets the stream",
      "# (RST_STREAM, PROTOCOL_ERROR); any plain host name works.",
      'OPTIONS = [("grpc.default_authority", "localhost")]'];
    if (streamed && call.parts) head.push(`MEDIA_FILE = "${S.MEDIA_FILE}"`, `CHUNK_SIZE = ${S.CHUNK_SIZE}`);
    head.push(`REQUEST = ${pyString(S.pretty(call.request))}`);
    const body = ["", "", "def main() -> None:", "    with grpc.insecure_channel(SOCKET, options=OPTIONS) as channel:",
      `        stub = ${S.moduleOf(call.service.file)}_pb2_grpc.${call.service.name}Stub(channel)`];
    if (!streamed) body.push(`        request = json_format.Parse(REQUEST, ${pyClass(call, call.method.inputType)}())`);
    body.push(...pyCall(call).map((l) => `        ${l}`));
    const tail = ["", "", 'if __name__ == "__main__":', "    main()"];
    return `${[...head, ...(streamed ? pyFrames(call) : []), ...body, ...tail].join("\n")}\n`;
  }

  // ---- Go ------------------------------------------------------------------

  function goString(text) {
    return text.includes("`") ? JSON.stringify(text) : `\`${text}\``;
  }

  function goType(fullName) {
    return `wamuxv1.${S.localPath(fullName).join("_")}`;
  }

  function goImports(call) {
    const std = ["context", "fmt", "log"];
    if (call.shape !== "unary") std.push("errors", "io");
    if (call.shape === "client" && call.parts) std.push("os");
    const ext = ['"google.golang.org/grpc"', '"google.golang.org/grpc/credentials/insecure"',
      '"google.golang.org/protobuf/encoding/protojson"'];
    const own = `wamuxv1 "${S.GO_PACKAGE}"`;
    return ["import (", ...std.sort().map((p) => `\t"${p}"`), "", ...ext.map((p) => `\t${p}`), "", `\t${own}`, ")"];
  }

  function goCall(call) {
    const rpc = `client.${call.method.name}`;
    if (call.shape === "unary") {
      return [`reply, err := ${rpc}(ctx, req)`, "if err != nil {", "\tlog.Fatal(err)", "}",
        "fmt.Println(protojson.Format(reply))"];
    }
    if (call.shape === "server") {
      return [`stream, err := ${rpc}(ctx, req)`, "if err != nil {", "\tlog.Fatal(err)", "}", "for {",
        "\treply, err := stream.Recv()", "\tif errors.Is(err, io.EOF) {", "\t\tbreak", "\t}",
        "\tif err != nil {", "\t\tlog.Fatal(err)", "\t}", "\tfmt.Println(protojson.Format(reply))", "}"];
    }
    return goClientStream(call, rpc);
  }

  function goClientStream(call, rpc) {
    const send = call.parts ? "sendFrames(stream, req)" : "ignoreEOF(stream.Send(req))";
    return [`stream, err := ${rpc}(ctx)`, "if err != nil {", "\tlog.Fatal(err)", "}",
      `if err := ${send}; err != nil {`, "\tlog.Fatal(err)", "}", "reply, err := stream.CloseAndRecv()",
      "if err != nil {", "\tlog.Fatal(err)", "}", "fmt.Println(protojson.Format(reply))"];
  }

  // Helpers after main for a client stream. io.EOF from Send means the daemon
  // already answered (an error before the last chunk): CloseAndRecv has why.
  function goHelpers(call) {
    if (call.shape !== "client") return [];
    const eof = ["", "func ignoreEOF(err error) error {", "\tif errors.Is(err, io.EOF) {", "\t\treturn nil", "\t}",
      "\treturn err", "}"];
    if (!call.parts) return eof;
    const outer = goType(call.method.inputType);
    const member = S.goCamel(call.parts.chunk.name);
    const frame = `&${outer}{${S.goCamel(call.parts.oneof)}: &${outer}_${member}{${member}: buf[:n]}}`;
    const streamType = `wamuxv1.${call.service.name}_${call.method.name}Client`;
    return ["", `func sendFrames(stream ${streamType}, header *${outer}) error {`,
      "\tif err := stream.Send(header); err != nil {", "\t\treturn ignoreEOF(err)", "\t}",
      "\tmedia, err := os.Open(mediaFile)", "\tif err != nil {", "\t\treturn err", "\t}", "\tdefer media.Close()",
      `\tbuf := make([]byte, ${S.CHUNK_SIZE})`, "\tfor {", "\t\tn, err := media.Read(buf)", "\t\tif n > 0 {",
      `\t\t\tif err := stream.Send(${frame}); err != nil {`, "\t\t\t\treturn ignoreEOF(err)", "\t\t\t}", "\t\t}",
      "\t\tif errors.Is(err, io.EOF) {", "\t\t\treturn nil", "\t\t}", "\t\tif err != nil {", "\t\t\treturn err",
      "\t\t}", "\t}", "}", ...eof];
  }

  function go(call) {
    const consts = [`const socket = ${JSON.stringify(`unix://${call.socket}`)}`];
    if (call.shape === "client" && call.parts) consts.push(`const mediaFile = "${S.MEDIA_FILE}"`);
    consts.push(`const request = ${goString(S.pretty(call.request))}`);
    const body = ["conn, err := grpc.NewClient(socket, grpc.WithTransportCredentials(insecure.NewCredentials()))",
      "if err != nil {", "\tlog.Fatal(err)", "}", "defer conn.Close()",
      `client := wamuxv1.New${call.service.name}Client(conn)`, "ctx := context.Background()", "",
      `req := &${goType(call.method.inputType)}{}`, "if err := protojson.Unmarshal([]byte(request), req); err != nil {",
      "\tlog.Fatal(err)", "}", ...goCall(call)];
    const main = ["func main() {", ...body.map((l) => (l ? `\t${l}` : "")), "}"];
    const lines = ["package main", "", ...goImports(call), "", ...consts, "", ...main, ...goHelpers(call)];
    return `${lines.join("\n")}\n`;
  }

  // ---- JavaScript ----------------------------------------------------------

  function jsHead(call) {
    const lines = ['const grpc = require("@grpc/grpc-js");', 'const protoLoader = require("@grpc/proto-loader");'];
    if (call.shape === "client" && call.parts) {
      lines.unshift('const fs = require("node:fs");');
    }
    lines.push("", `const PROTO_DIR = "${S.PROTO_DIR}";`, `const SOCKET = ${JSON.stringify(`unix://${call.socket}`)};`);
    if (call.shape === "client" && call.parts) lines.push(`const MEDIA_FILE = "${S.MEDIA_FILE}";`);
    lines.push(`const REQUEST = ${S.pretty(call.request)};`, "",
      `const definition = protoLoader.loadSync("${call.service.file}", {`,
      "  includeDirs: [PROTO_DIR],", "  longs: String,", "  enums: String,", "  bytes: String,", "  oneofs: true,", "});",
      `const wamux = grpc.loadPackageDefinition(definition).${call.service.fullName.split(".").slice(0, -1).join(".")};`,
      `const client = new wamux.${call.service.name}(SOCKET, grpc.credentials.createInsecure());`, "");
    return lines;
  }

  function jsCall(call) {
    const rpc = `client.${call.method.name}`;
    if (call.shape === "unary") {
      return [`${rpc}(REQUEST, (error, reply) => {`, "  client.close();", "  if (error) {",
        "    console.error(error.message);", "    process.exitCode = 1;", "    return;", "  }",
        "  console.log(JSON.stringify(reply, null, 2));", "});"];
    }
    if (call.shape === "server") {
      return [`const call = ${rpc}(REQUEST);`, 'call.on("data", (reply) => console.log(JSON.stringify(reply)));',
        'call.on("error", (error) => {', "  console.error(error.message);", "  process.exitCode = 1;", "});",
        'call.on("end", () => client.close());'];
    }
    return jsClientStream(call, rpc);
  }

  function jsClientStream(call, rpc) {
    const lines = ["function sendFrames() {", "  return new Promise((resolve, reject) => {"];
    if (!call.parts) {
      lines.push(`    const call = ${rpc}((error, reply) => (error ? reject(error) : resolve(reply)));`,
        "    call.write(REQUEST);", "    call.end();");
    } else {
      lines.push(`    const media = fs.createReadStream(MEDIA_FILE, { highWaterMark: ${S.CHUNK_SIZE} });`,
        `    const call = ${rpc}((error, reply) => {`, "      media.destroy();",
        "      if (error) reject(error);", "      else resolve(reply);", "    });", "    call.write(REQUEST);",
        '    media.on("data", (chunk) => {', `      if (!call.write({ ${call.parts.chunk.jsonName}: chunk })) {`,
        "        media.pause();", '        call.once("drain", () => media.resume());', "      }", "    });",
        '    media.on("end", () => call.end());', '    media.on("error", (error) => {', "      call.cancel();",
        "      reject(error);", "    });");
    }
    lines.push("  });", "}", "", "sendFrames()", "  .then((reply) => console.log(JSON.stringify(reply, null, 2)))",
      "  .catch((error) => {", "    console.error(error.message);", "    process.exitCode = 1;", "  })",
      "  .finally(() => client.close());");
    return lines;
  }

  function javascript(call) {
    return `${[...jsHead(call), ...jsCall(call)].join("\n")}\n`;
  }

  // ---- getting the stubs -----------------------------------------------------

  const CLONE = "git clone https://github.com/LLawli/wamux";

  function grpcurlStubs(call) {
    return [`# Reflection is on by default (enable_reflection). With it off, load the protos:`,
      `${CLONE}`, `grpcurl -unix -plaintext -import-path ${S.PROTO_DIR} -proto ${call.service.file} \\`,
      `  -d '{}' ${socketArg(call.socket)} ${call.service.fullName}/${call.method.name}`].join("\n");
  }

  function pythonStubs() {
    return [CLONE, "pip install grpcio grpcio-tools",
      `python -m grpc_tools.protoc -I ${S.PROTO_DIR} --python_out=. --grpc_python_out=. ${S.PROTO_DIR}/*.proto`]
      .join("\n");
  }

  // The protos carry no go_package, so each file is mapped to one package.
  function goStubs() {
    return [CLONE, `go mod init ${S.GO_PACKAGE.split("/").slice(0, -1).join("/")}`,
      "go install google.golang.org/protobuf/cmd/protoc-gen-go@latest",
      "go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest",
      "go get google.golang.org/grpc google.golang.org/protobuf", `PKG=${S.GO_PACKAGE}`, "MAP=",
      `for f in ${S.PROTO_DIR}/*.proto; do`, '  b=$(basename "$f")',
      '  MAP="$MAP --go_opt=M$b=$PKG --go-grpc_opt=M$b=$PKG"', "done", "mkdir -p wamuxv1",
      `protoc -I ${S.PROTO_DIR} --go_out=wamuxv1 --go_opt=paths=source_relative \\`,
      `  --go-grpc_out=wamuxv1 --go-grpc_opt=paths=source_relative $MAP ${S.PROTO_DIR}/*.proto`].join("\n");
  }

  function jsStubs() {
    return [CLONE, "npm install @grpc/grpc-js @grpc/proto-loader",
      "# No generated code: proto-loader reads the .proto files at run time (PROTO_DIR)."].join("\n");
  }

  S.STUBS = { grpcurl: grpcurlStubs, python: pythonStubs, go: goStubs, js: jsStubs };
  S.GENERATORS.grpcurl = grpcurl;
  S.GENERATORS.python = python;
  S.GENERATORS.go = go;
  S.GENERATORS.js = javascript;
})();
