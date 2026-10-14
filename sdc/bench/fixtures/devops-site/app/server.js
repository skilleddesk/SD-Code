const http = require("http");

const port = Number(process.env.PORT || 8080);

http
  .createServer((req, res) => {
    if (req.url === "/health") {
      res.writeHead(200, { "content-type": "text/plain" });
      return res.end("ok");
    }
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ shop: "open", path: req.url }));
  })
  .listen(port, () => console.log(`shop-api listening on ${port}`));
