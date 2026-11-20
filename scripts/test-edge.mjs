import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const suffix = randomUUID().replaceAll("-", "");
const scratch = resolve(".tools", `edge-test-${suffix}`);
const profile = join(scratch, "profile");
const extension = join(scratch, "extension");
const hostName = `com.tabsort_lite.test_${suffix}`;
const hostKey = `HKCU\\Software\\Microsoft\\Edge\\NativeMessagingHosts\\${hostName}`;
const edgeExe = join(process.env["ProgramFiles(x86)"], "Microsoft", "Edge", "Application", "msedge.exe");
const release = resolve("target", "release");
const hostExe = join(release, "tabsort-lite.exe");
const testExe = process.argv[2] ?? execFileSync("cargo.exe", ["test", "--release", "--locked", "--no-run", "--message-format=json"], { encoding: "utf8", windowsHide: true })
  .split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line)).findLast((event) => event.profile?.test && event.executable)?.executable;
assert(testExe, "Could not find the Rust integration test executable");

let aiRequests = 0;
const web = createServer(async (request, response) => {
  if (request.url === "/v1/chat/completions" && request.method === "POST") {
    try {
      const body = [];
      for await (const chunk of request) body.push(chunk);
      const payload = JSON.parse(Buffer.concat(body).toString("utf8"));
      assert.equal(request.headers.authorization, "Bearer fixture-key");
      assert.equal(payload.model, "fixture-model");
      assert.match(payload.messages.find(({ role }) => role === "system").content, /自行推荐/);
      const raw = payload.messages.find(({ role }) => role === "user").content;
      assert(!raw.includes("http://") && !raw.includes("https://"));
      const inputs = JSON.parse(raw);
      assert(!inputs.some(({ title }) => title === "pinned"));
      assert(!inputs.some(({ title }) => title === "手动研究组"));
      assert(inputs.some(({ title }) => title === "dashboard"));
      const kept = inputs.find(({ keep_group }) => keep_group === 2);
      assert.equal(kept?.title, "保留的开发组");
      const paper = inputs.find(({ title }) => title === "radar paper").id;
      const code = inputs.find(({ title }) => title === "radar code").id;
      const omitted = inputs.find(({ title }) => title === "interview guide").id;
      const conflicting = inputs.find(({ title }) => title === "firmware testing").id;
      const uncertain = inputs.find(({ title }) => title === "internship").id;
      const other = inputs.filter(({ id }) => ![paper, code, kept.id, omitted, conflicting, uncertain].includes(id)).map(({ id }) => id);
      const plan = { windows: [
        { title: "研究与分析", groups: [{ title: "雷达 · 文献阅读", items: [paper] }] },
        { title: "项目开发", groups: [{ title: "雷达 · 代码实现", items: [code, conflicting] }] },
        { title: "求职准备", groups: [{ title: "辅助工作", items: [...other, conflicting] }] },
      ], uncertain_items: [uncertain] };
      aiRequests++;
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify({ choices: [{ finish_reason: "stop", message: { content: JSON.stringify(plan) } }] }));
    } catch (error) {
      console.error(error);
      response.writeHead(500).end();
    }
    return;
  }
  const title = request.url.slice(1).replaceAll("-", " ");
  response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  response.end(`<!doctype html><title>${title}</title><body style="height:3000px"><input id="draft"><script>window.loaded = Math.random().toString()</script>`);
});
await new Promise((done) => web.listen(0, "127.0.0.1", done));
const base = `http://127.0.0.1:${web.address().port}/`;
const modelDirectory = process.env.TABSORT_TEST_MODELS ?? resolve("dist", "TabSortLite-Edge-1.0.0", "models");
const env = { ...process.env, TABSORT_TEST_PIPE: suffix, TABSORT_TEST_DATA: scratch, TABSORT_TEST_URL: base, TABSORT_TEST_MODELS: modelDirectory };

async function eventually(get) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      const value = await get();
      if (value) return value;
    } catch {}
    await sleep(200);
  }
  throw new Error("Timed out waiting for the isolated Edge profile");
}

async function devtools(url) {
  const socket = new WebSocket(url);
  await new Promise((done, fail) => {
    socket.addEventListener("open", done, { once: true });
    socket.addEventListener("error", fail, { once: true });
  });
  socket.send(JSON.stringify({ id: -1, method: "Runtime.runIfWaitingForDebugger" }));
  let id = 0;
  return {
    close: () => socket.close(),
    evaluate(expression) {
      const requestId = ++id;
      return new Promise((done, fail) => {
        const cleanup = () => {
          clearTimeout(timer);
          socket.removeEventListener("message", reply);
          socket.removeEventListener("close", closed);
        };
        const closed = () => { cleanup(); fail(new Error("Isolated Edge debugging connection closed")); };
        const timer = setTimeout(() => { cleanup(); fail(new Error(`Isolated Edge operation timed out while evaluating ${expression.slice(0, 70)}`)); }, 30_000);
        const reply = ({ data }) => {
          const response = JSON.parse(data);
          if (response.id !== requestId) return;
          cleanup();
          if (response.error || response.result.exceptionDetails) {
            fail(new Error(JSON.stringify(response.error ?? response.result.exceptionDetails)));
          } else done(response.result.result.value);
        };
        socket.addEventListener("close", closed, { once: true });
        socket.addEventListener("message", reply);
        socket.send(JSON.stringify({ id: requestId, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true } }));
      });
    },
  };
}

let edge;
let worker;
let page;
let port;
try {
  await mkdir(profile, { recursive: true });
  await cp("extension", extension, { recursive: true });
  const workerFile = join(extension, "background.js");
  const source = await readFile(workerFile, "utf8");
  await writeFile(workerFile, source.replace("com.tabsort_lite.connector", hostName));
  const id = (await readFile(join(extension, "id.txt"), "utf8")).trim();
  const nativeManifest = join(scratch, "host.json");
  await writeFile(nativeManifest, JSON.stringify({ name: hostName, description: "Isolated TabSort Lite test", type: "stdio", path: hostExe, allowed_origins: [`chrome-extension://${id}/`] }));
  execFileSync("reg.exe", ["add", hostKey, "/ve", "/t", "REG_SZ", "/d", nativeManifest, "/f"], { windowsHide: true });

  edge = spawn(edgeExe, ["--headless=new", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--no-first-run", "--disable-sync", "--disable-background-networking", "--disable-features=DisableLoadExtensionCommandLineSwitch,msImplicitSignin", `--load-extension=${extension}`, `${base}form`], { env, windowsHide: true, stdio: "ignore" });
  port = await eventually(async () => (await readFile(join(profile, "DevToolsActivePort"), "utf8")).split(/\r?\n/)[0]);
  const targets = () => fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json());
  await eventually(async () => (await targets()).find(({ url }) => url === `chrome-extension://${id}/background.js`));
  const debugUrl = `chrome-extension://${id}/anchor.html`;
  const background = await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(debugUrl)}`, { method: "PUT" }).then((response) => response.json());
  worker = await devtools(background.webSocketDebuggerUrl);
  await eventually(() => worker.evaluate("typeof chrome !== 'undefined' && typeof chrome.tabs?.query === 'function'"));
  await worker.evaluate(`(async () => {
    const base = ${JSON.stringify(base)};
    const form = (await chrome.tabs.query({ url: base + "form" }))[0];
    const startup = (await chrome.tabs.query({})).filter((tab) => !tab.url?.startsWith(base) && tab.url !== ${JSON.stringify(debugUrl)});
    if (startup.length) await chrome.tabs.remove(startup.map((tab) => tab.id));
    await chrome.tabs.create({ windowId: form.windowId, url: base + "pinned", pinned: true, active: false });
    const manual = [];
    for (const path of ["radar-paper", "radar-code"]) manual.push((await chrome.tabs.create({ windowId: form.windowId, url: base + path, active: false })).id);
    const group = await chrome.tabs.group({ tabIds: manual, createProperties: { windowId: form.windowId } });
    await chrome.tabGroups.update(group, { title: "手动研究组", color: "purple", collapsed: true });
    const kept = [];
    for (const path of ["shader-compilation", "rust-ownership"]) kept.push((await chrome.tabs.create({ windowId: form.windowId, url: base + path, active: false })).id);
    const keptGroup = await chrome.tabs.group({ tabIds: kept, createProperties: { windowId: form.windowId } });
    await chrome.tabGroups.update(keptGroup, { title: "保留的开发组", color: "green" });
    await chrome.tabs.create({ windowId: form.windowId, url: base + "firmware-testing", active: false });
    const other = await chrome.windows.create({ url: base + "campus-recruiting" });
    for (const path of ["interview-guide", "dashboard", "rust-ownership", "internship"]) await chrome.tabs.create({ windowId: other.id, url: base + path, active: false });
    const deadline = Date.now() + 20_000;
    while ((await chrome.tabs.query({ url: base + "*" })).some((tab) => tab.status !== "complete")) {
      if (Date.now() > deadline) throw new Error("Local test pages did not finish loading");
      await new Promise((resolve) => setTimeout(resolve, 40));
    }
    return true;
  })()`);
  worker.close();
  worker = undefined;
  await fetch(`http://127.0.0.1:${port}/json/close/${background.id}`);
  const formTarget = await eventually(async () => (await targets()).find(({ url }) => url === `${base}form`));
  page = await devtools(formTarget.webSocketDebuggerUrl);
  const expected = await page.evaluate(`document.querySelector("#draft").value = "未提交的测试内容"; scrollTo(0, 620); [window.loaded, document.querySelector("#draft").value, scrollY]`);
  assert.equal(expected[2], 620);

  const test = spawn(resolve(testExe), ["--ignored", "--nocapture", "isolated_edge_round_trip"], { env, windowsHide: true, stdio: "inherit" });
  const exit = await new Promise((done, fail) => { test.on("error", fail); test.on("exit", done); });
  assert.equal(exit, 0);
  assert.equal(aiRequests, 1);
  const formAfter = await eventually(async () => (await targets()).find(({ id }) => id === formTarget.id));
  assert.equal(formAfter.url, formTarget.url);
  const actual = await page.evaluate(`[window.loaded, document.querySelector("#draft").value, scrollY]`);
  assert.deepEqual(actual, expected, "Move and restore changed the form or scroll state");
  const result = JSON.parse(await readFile(join(scratch, "edge-result.json"), "utf8"));
  console.log(JSON.stringify({ ...result, formAndScrollRetained: true }, null, 2));
} finally {
  page?.close();
  worker?.close();
  if (port) {
    try {
      const version = await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1500) }).then((response) => response.json());
      const socket = new WebSocket(version.webSocketDebuggerUrl);
      await Promise.race([new Promise((done) => {
        socket.addEventListener("open", () => socket.send(JSON.stringify({ id: 1, method: "Browser.close" })), { once: true });
        socket.addEventListener("close", done, { once: true });
        socket.addEventListener("error", done, { once: true });
      }), sleep(1500)]);
      socket.close();
    } catch {}
  }
  if (edge?.pid && edge.exitCode === null) {
    try { execFileSync("taskkill.exe", ["/pid", String(edge.pid), "/t", "/f"], { windowsHide: true, stdio: "ignore" }); } catch {}
  }
  web.close();
  try { execFileSync("reg.exe", ["delete", hostKey, "/f"], { windowsHide: true, stdio: "ignore" }); } catch {}
}
