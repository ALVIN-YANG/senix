import http from 'node:http';

// Fixed bytes, no disk, logging or artificial delay. This is a single Node worker.
const payload = Buffer.alloc(1024, 'x');
http.createServer((_request, response) => {
  response.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': payload.length });
  response.end(payload);
}).listen(4101, '0.0.0.0');
