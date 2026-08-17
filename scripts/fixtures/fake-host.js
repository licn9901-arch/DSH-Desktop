const http = require("node:http");

let settingsRevision = 1;
const scenario = process.env.DSH_DESKTOP_FAKE_HOST_SCENARIO ?? "legacy";
const pluginDelay = Number(process.env.DSH_DESKTOP_FAKE_PLUGIN_DELAY_MS ?? "500");

const server = http.createServer((request, response) => {
  if (request.method === "POST" && request.url === "/sidebar/api/settings.get") {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ ok: true, value: { value: {}, revision: settingsRevision } }));
    return;
  }
  if (request.method === "POST" && request.url === "/sidebar/api/settings.update") {
    let body = "";
    request.setEncoding("utf8");
    request.on("data", (chunk) => { body += chunk; });
    request.on("end", () => {
      const payload = JSON.parse(body);
      if (payload.expectedRevision !== settingsRevision) {
        response.writeHead(409, { "content-type": "application/json" });
        response.end(JSON.stringify({ ok: false, error: { message: "revision conflict" } }));
        return;
      }
      settingsRevision += 1;
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify({ ok: true, value: { value: payload.patch, revision: settingsRevision } }));
    });
    return;
  }
  response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  response.end("<!doctype html><title>DSH fake host</title><h1>ready</h1>");
});

server.listen(0, "127.0.0.1", () => {
  const address = server.address();
  const url = `http://127.0.0.1:${address.port}`;
  if (scenario === "legacy") {
    process.stdout.write(`dsh web: ${url}\n`);
    return;
  }
  process.stdout.write(`dsh desktop-core: ${url}\n`);
  if (scenario === "core-crash") {
    setTimeout(() => process.exit(21), Math.max(50, pluginDelay)).unref();
    return;
  }
  if (scenario === "plugins-never") return;
  setTimeout(() => process.stdout.write(`dsh web: ${url}\n`), Math.max(0, pluginDelay)).unref();
});

function shutdown() {
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(1), 4000).unref();
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
