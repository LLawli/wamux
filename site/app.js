// The page (#280): picks an RPC from schema.json, keeps the request JSON in
// the editor, and renders the snippet of the chosen language. Everything that
// turns a request into code lives in snippets.js, languages.js and rust.js.
"use strict";

(() => {
  const S = WamuxSnippets;
  const LANGS = [["grpcurl", "grpcurl"], ["python", "Python"], ["go", "Go"], ["rust", "Rust"], ["js", "JS"]];
  const copyOf = (value) => JSON.parse(JSON.stringify(value));
  const STORE_KEY = "wamux-snippets-lang";
  const $ = (id) => document.getElementById(id);
  const state = { index: null, service: null, method: null, lang: "grpcurl", call: null };

  function remembered(key, fallback) {
    try {
      return localStorage.getItem(key) || fallback;
    } catch (_) {
      return fallback;
    }
  }

  function remember(key, value) {
    try {
      localStorage.setItem(key, value);
    } catch (_) {
      // Private windows refuse storage; the page works without it.
    }
  }

  function el(tag, props = {}, children = []) {
    const node = Object.assign(document.createElement(tag), props);
    for (const child of children) node.append(child);
    return node;
  }

  // ---- selection -------------------------------------------------------------

  function streamMark(method) {
    if (method.clientStreaming) return " ⇉";
    return method.serverStreaming ? " ⇇" : "";
  }

  function fillServices() {
    const options = state.index.services.map((s) => el("option", { value: s.name, textContent: s.name }));
    $("service").replaceChildren(...options);
  }

  function fillMethods() {
    const options = state.service.methods.map((m) => el("option", { value: m.name, textContent: m.name + streamMark(m) }));
    $("method").replaceChildren(...options);
    $("method").value = state.method.name;
  }

  function select(serviceName, methodName, request) {
    const service = state.index.services.find((s) => s.name === serviceName) || state.index.services[0];
    state.service = service;
    state.method = service.methods.find((m) => m.name === methodName) || service.methods[0];
    $("service").value = service.name;
    fillMethods();
    renderMethodDoc();
    const body = request || S.skeletonOf(state.index, state.method.inputType);
    $("request").value = JSON.stringify(body, null, 2);
    markPreset();
    refresh();
  }

  function markPreset() {
    for (const button of $("presets").children) {
      const on = button.dataset.service === state.service.name && button.dataset.method === state.method.name;
      button.classList.toggle("active", on);
    }
  }

  function renderMethodDoc() {
    const m = state.method;
    const short = (name) => name.split(".").pop();
    const input = `${m.clientStreaming ? "stream " : ""}${short(m.inputType)}`;
    const output = `${m.serverStreaming ? "stream " : ""}${short(m.outputType)}`;
    $("signature").textContent = `rpc ${m.name}(${input}) returns (${output})`;
    $("doc").textContent = m.comment || "";
    $("doc").hidden = !m.comment;
  }

  // ---- request and snippet ---------------------------------------------------

  function parsedRequest() {
    try {
      return { value: JSON.parse($("request").value) };
    } catch (error) {
      return { error: `JSON: ${error.message}` };
    }
  }

  function refresh() {
    const parsed = parsedRequest();
    let call = null;
    let problem = parsed.error;
    if (!problem) {
      try {
        call = S.callOf(state.index, state.service, state.method, parsed.value, socketPath());
      } catch (error) {
        problem = error.message;
      }
    }
    $("status").textContent = problem || "";
    $("request").classList.toggle("invalid", Boolean(problem));
    $("snippet-pane").classList.toggle("stale", Boolean(problem));
    if (call) state.call = call;
    renderFields(parsed.value);
    renderSnippet();
    writeHash();
  }

  function socketPath() {
    return $("socket").value.trim() || "/run/wamux.sock";
  }

  function renderSnippet() {
    if (!state.call) return;
    $("snippet").textContent = S.generate(state.lang, state.call);
    $("stubs").textContent = S.STUBS[state.lang](state.call);
    for (const tab of $("tabs").children) {
      const on = tab.dataset.lang === state.lang;
      tab.classList.toggle("active", on);
      tab.setAttribute("aria-selected", String(on));
    }
  }

  // ---- field list --------------------------------------------------------------

  /** The message the checkboxes edit: a streamed chunk's header, else the request. */
  function fieldScope() {
    const parts = state.method.clientStreaming ? S.streamPartsOf(state.index, state.method.inputType) : null;
    if (!parts) return { path: [], message: S.messageOf(state.index, state.method.inputType) };
    return { path: [parts.header.jsonName], message: S.messageOf(state.index, parts.header.typeName) };
  }

  function objectAt(root, path) {
    let node = root;
    for (const key of path) {
      if (!node || typeof node !== "object") return null;
      node = node[key];
    }
    return node && typeof node === "object" && !Array.isArray(node) ? node : null;
  }

  function typeLabel(field) {
    const short = field.typeName ? field.typeName.split(".").pop() : field.kind;
    if (field.label === "repeated") return `${short}[]`;
    if (field.label === "map") return `map<${field.mapKey}, ${short}>`;
    return field.label === "optional" ? `${short}?` : short;
  }

  function typeTitle(field) {
    if (field.kind !== "enum") return field.typeName || field.kind;
    return state.index.enums.get(field.typeName).values.map((v) => v.name).join("\n");
  }

  function fieldRow(field, target) {
    const box = el("input", { type: "checkbox", checked: Boolean(target && field.jsonName in target) });
    box.disabled = !target;
    box.title = target ? `Include ${field.jsonName}` : "Fix the JSON first";
    box.addEventListener("change", () => toggleField(field, box.checked));
    const name = el("code", { className: "fname", textContent: field.jsonName });
    const type = el("span", { className: "ftype", textContent: typeLabel(field), title: typeTitle(field) });
    const head = el("label", { className: "fhead" }, [box, name, type]);
    if (field.oneof) head.append(el("span", { className: "oneof", textContent: field.oneof, title: `oneof ${field.oneof}` }));
    const row = el("li", {}, [head]);
    if (field.comment) {
      const doc = el("p", { className: "fdoc", textContent: field.comment, title: "Expand" });
      doc.addEventListener("click", () => doc.classList.toggle("open"));
      row.append(doc);
    }
    return row;
  }

  function renderFields(root) {
    const scope = fieldScope();
    const target = root === undefined ? null : objectAt(root, scope.path);
    const prefix = scope.path.length ? `${scope.path.join(".")} · ` : "";
    $("fields-title").textContent = `${prefix}${scope.message.name}`;
    $("fields").replaceChildren(...scope.message.fields.map((f) => fieldRow(f, target)));
  }

  function valueFor(field) {
    if (field.kind === "message" && field.label === "singular") return S.skeletonOf(state.index, field.typeName);
    if (field.label === "optional") return S.defaultOf(state.index, { ...field, label: "singular" });
    return S.defaultOf(state.index, field);
  }

  function toggleField(field, on) {
    const parsed = parsedRequest();
    if (parsed.error) return;
    const scope = fieldScope();
    const target = objectAt(parsed.value, scope.path);
    if (!target) return;
    for (const other of scope.message.fields) {
      const sibling = on && field.oneof && other.oneof === field.oneof;
      if (sibling || other === field) {
        delete target[other.jsonName];
        delete target[other.name];
      }
    }
    if (on) target[field.jsonName] = valueFor(field);
    $("request").value = JSON.stringify(parsed.value, null, 2);
    refresh();
  }

  // ---- location, copy, wiring ------------------------------------------------

  function writeHash() {
    const hash = `#${state.service.name}/${state.method.name}/${state.lang}`;
    if (location.hash !== hash) history.replaceState(null, "", hash);
  }

  function readHash() {
    const [service, method, lang] = decodeURIComponent(location.hash.slice(1)).split("/");
    return { service, method, lang };
  }

  function copyFrom(button, source) {
    const done = () => {
      button.classList.add("done");
      setTimeout(() => button.classList.remove("done"), 1200);
    };
    navigator.clipboard.writeText($(source).textContent).then(done, () => {
      getSelection().selectAllChildren($(source));
    });
  }

  function buildTabs() {
    const tabs = LANGS.map(([lang, label]) => {
      const tab = el("button", { type: "button", textContent: label, role: "tab" });
      tab.dataset.lang = lang;
      tab.addEventListener("click", () => {
        state.lang = lang;
        remember(STORE_KEY, lang);
        renderSnippet();
        writeHash();
      });
      return tab;
    });
    $("tabs").replaceChildren(...tabs);
  }

  function buildPresets() {
    const exists = (p) => state.index.services.some((s) => s.name === p.service && s.methods.some((m) => m.name === p.method));
    const buttons = S.PRESETS.filter(exists).map(({ label, service, method, request }, i) => {
      const button = el("button", { type: "button", title: `${service}/${method}` }, [
        el("span", { className: "n", textContent: String(i + 1) }), label]);
      Object.assign(button.dataset, { service, method });
      button.addEventListener("click", () => select(service, method, copyOf(request)));
      return button;
    });
    $("presets").replaceChildren(...buttons);
  }

  function debounce(fn, ms) {
    let timer = 0;
    return () => {
      clearTimeout(timer);
      timer = setTimeout(fn, ms);
    };
  }

  function wire() {
    $("service").addEventListener("change", () => select($("service").value, null));
    $("method").addEventListener("change", () => select(state.service.name, $("method").value));
    $("request").addEventListener("input", debounce(refresh, 150));
    $("socket").addEventListener("input", debounce(refresh, 150));
    $("reset").addEventListener("click", () => select(state.service.name, state.method.name));
    $("copy").addEventListener("click", () => copyFrom($("copy"), "snippet"));
    $("copy-stubs").addEventListener("click", () => copyFrom($("copy-stubs"), "stubs"));
  }

  async function start() {
    try {
      const response = await fetch("schema.json");
      state.index = S.indexSchema(await response.json());
    } catch (error) {
      $("status").textContent = `schema.json did not load: ${error.message}`;
      return;
    }
    const wanted = readHash();
    const known = (lang) => LANGS.some(([l]) => l === lang);
    state.lang = [wanted.lang, remembered(STORE_KEY, "")].find(known) || "grpcurl";
    buildTabs();
    buildPresets();
    fillServices();
    wire();
    const first = S.PRESETS[0];
    const preset = S.PRESETS.find((p) => p.service === wanted.service && p.method === wanted.method);
    if (wanted.service) select(wanted.service, wanted.method, preset ? copyOf(preset.request) : null);
    else select(first.service, first.method, copyOf(first.request));
  }

  start();
})();
