const http = require("node:http");

let settingsRevision = 1;

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
  process.stdout.write(`dsh web: http://127.0.0.1:${address.port}\n`);
});

function shutdown() {
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(1), 4000).unref();
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
