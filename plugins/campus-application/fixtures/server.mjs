import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("./public/", import.meta.url));
const port = Number(process.env.PORT ?? 32124);
const types = new Map([[".html","text/html; charset=utf-8"],[".css","text/css; charset=utf-8"],[".js","text/javascript; charset=utf-8"]]);

const server = createServer(async (req, res) => {
  const requestPath = new URL(req.url ?? "/", `http://127.0.0.1:${port}`).pathname;
  const relative = requestPath === "/" ? "index.html" : requestPath.replace(/^\/+/, "");
  if (!/^[A-Za-z0-9._-]+$/.test(relative)) {
    res.writeHead(400, {"content-type":"text/plain; charset=utf-8"}).end("bad request");
    return;
  }
  try {
    const bytes = await readFile(join(root, relative));
    res.writeHead(200, {"content-type": types.get(extname(relative)) ?? "application/octet-stream", "cache-control":"no-store"});
    res.end(bytes);
  } catch {
    res.writeHead(404, {"content-type":"text/plain; charset=utf-8"}).end("not found");
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`campus-application fixtures: http://127.0.0.1:${port}/`);
});
