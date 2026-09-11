const { Router } = require("express");

// Built by a factory: the routes exist, but reaching them means running this function.
function createV2Router() {
  const router = Router();
  router.get("/users", (req, res) => res.json([]));
  return router;
}

module.exports = { createV2Router };
