const http = require("node:http");

const server = http.createServer((_request, response) => {
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
