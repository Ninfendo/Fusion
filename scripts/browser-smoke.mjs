// Browser smoke test: the main tab runs the hub, a click on Pop out opens a
// window for the panel, a selection reaches the linked panel in that window,
// and closing it brings the panel back.
//
// It drives a headless Chrome over the DevTools protocol, with nothing to
// install: Node 22 or later has fetch and WebSocket built in.
//
//   chrome --headless=new --remote-debugging-port=9333 --user-data-dir=<tmp> &
//   node scripts/browser-smoke.mjs 9333 http://127.0.0.1:8710/
//
// It exits with 1 and says what failed.

const [port, app] = process.argv.slice(2);
const base = `http://127.0.0.1:${port}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function fail(text) {
  console.error(`FAIL: ${text}`);
  process.exit(1);
}

async function connect(target) {
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
  let next = 0;
  const pending = new Map();
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      pending.get(message.id)(message);
      pending.delete(message.id);
    }
  };
  const call = (method, params = {}) =>
    new Promise((resolve) => {
      const id = ++next;
      pending.set(id, resolve);
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) =>
    (await call("Runtime.evaluate", { expression, returnByValue: true })).result?.result?.value;
  await call("Page.enable");
  await call("Emulation.setDeviceMetricsOverride", {
    width: 1280,
    height: 800,
    deviceScaleFactor: 1,
    mobile: false,
  });
  return { call, evaluate };
}

async function open(url) {
  const response = await fetch(`${base}/json/new?${encodeURIComponent(url)}`, { method: "PUT" });
  return connect(await response.json());
}

const pages = async () =>
  (await (await fetch(`${base}/json/list`)).json()).filter((t) => t.type === "page");

/// Polls `check` until it returns something, for up to `ms`.
async function until(ms, check) {
  const start = Date.now();
  while (Date.now() - start < ms) {
    const value = await check();
    if (value) return value;
    await sleep(200);
  }
  return null;
}

const savedWindows = (tab) =>
  tab.evaluate(`(() => {
    const json = localStorage.getItem("fusion.hub.layout");
    return json ? JSON.parse(json).windows.length : 0;
  })()`);

// 1. The main tab starts and runs the hub.
const main = await open(app);
const drawn = await until(30000, () =>
  main.evaluate(`!document.getElementById("loading")`),
);
if (!drawn) fail("the main tab did not start");
await sleep(1500);

// 2. A click on Pop out in the L2 panel header (the right column, top)
//    opens a browser window for it.
await main.call("Page.bringToFront");
for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
  await main.call("Input.dispatchMouseEvent", {
    type,
    x: 974,
    y: 67,
    button: "left",
    clickCount: 1,
  });
  await sleep(60);
}
const popupTarget = await until(10000, async () =>
  (await pages()).find((t) => t.url.includes("?window=")),
);
if (!popupTarget) fail("Pop out did not open a window");
console.log(`popped out: ${popupTarget.url}`);
if ((await until(5000, async () => (await savedWindows(main)) === 2)) === null) {
  fail("the layout with the popped-out window was not saved");
}

// 3. A selection from the watchlist reaches the L2 in the new window, and
//    only there. A spy tab posts as a second copy of the main tab would.
const window = Number(new URL(popupTarget.url).searchParams.get("window"));
const popup = await connect(popupTarget);
await until(30000, () => popup.evaluate(`!document.getElementById("loading")`));
const spy = await open(`${app}spy`);
await spy.evaluate(`(() => {
  window.seen = [];
  window.ch = new BroadcastChannel("fusion");
  window.ch.onmessage = (e) => window.seen.push(JSON.parse(e.data));
})()`);
await spy.evaluate(`window.ch.postMessage(JSON.stringify({"to-hub": {"from": 1, "message":
  {"select": {"from": 1, "context": {"type": "fdc3.instrument", "id": {"ticker": "SOLUSDT"}},
  "gesture": null}}}}))`);
const delivered = await until(5000, () =>
  spy.evaluate(`(() => {
    const found = window.seen.filter(e => e["from-hub"]?.message?.deliver);
    return found.length ? JSON.stringify(found) : null;
  })()`),
);
const deliveries = JSON.parse(delivered ?? "[]");
if (deliveries.length !== 1) fail(`expected one delivery, got ${delivered}`);
if (deliveries[0]["from-hub"].to.window !== window) {
  fail(`the selection went to ${JSON.stringify(deliveries[0]["from-hub"].to)}, not window ${window}`);
}
console.log(`the selection reached window ${window}`);

// 4. Closing the window brings the panel back into the main window.
await fetch(`${base}/json/close/${popupTarget.id}`);
if ((await until(5000, async () => (await savedWindows(main)) === 1)) === null) {
  fail("closing the window did not bring its panel back");
}
console.log("closing the window brought its panel back");
console.log("PASS");
process.exit(0);
