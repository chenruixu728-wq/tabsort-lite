import { collect, execute, restore } from "./layout.js";

const host = "com.tabsort_lite.connector";
const alarm = "tabsort-reconnect";
let port;
let running = false;

const identity = (async () => {
  const { profileId = crypto.randomUUID() } = await chrome.storage.local.get("profileId");
  const { sessionId = crypto.randomUUID() } = await chrome.storage.session.get("sessionId");
  await chrome.storage.local.set({ profileId });
  await chrome.storage.session.set({ sessionId });
  return { profileId, sessionId };
})();

async function connect() {
  if (port || running) return;
  const ids = await identity;
  if (port || running) return;
  const connection = chrome.runtime.connectNative(host);
  port = connection;
  connection.onDisconnect.addListener(() => {
    // Reading lastError prevents browser diagnostics from reporting expected retries.
    void chrome.runtime.lastError;
    if (port === connection) port = undefined;
    void chrome.action.setBadgeText({ text: "" });
  });
  connection.onMessage.addListener(async ({ id, type, data }) => {
    if (running || !Number.isSafeInteger(id)) return;
    running = true;
    try {
      const handlers = {
        ping: () => null,
        scan: () => collect(ids),
        execute: () => execute(ids, data),
        restore: () => restore(ids, data),
      };
      if (!Object.hasOwn(handlers, type)) throw new Error("未知指令");
      const result = await handlers[type]();
      if (port === connection) connection.postMessage({ id, ok: true, data: result });
      void chrome.action.setBadgeText({ text: "ON" });
    } catch (error) {
      if (port === connection) connection.postMessage({ id, ok: false, error: error.message });
    } finally {
      running = false;
    }
  });
  connection.postMessage({ protocol: 1, groupSplit: true, ...ids });
}

async function start() {
  if (!(await chrome.alarms.get(alarm))) {
    await chrome.alarms.create(alarm, { periodInMinutes: 0.5 });
  }
  await connect();
}

chrome.runtime.onInstalled.addListener(start);
chrome.runtime.onStartup.addListener(start);
chrome.action.onClicked.addListener(start);
chrome.alarms.onAlarm.addListener(({ name }) => {
  if (name === alarm) void connect();
});
void start();
