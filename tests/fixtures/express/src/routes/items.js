const { Router } = require("express");

const router = Router();

// Mounted at process.env.API_PREFIX, which is unknown until runtime.
router.get("/items", (req, res) => res.json([]));

module.exports = router;
