const { Router } = require("express");

// Declared, exported, and never mounted anywhere.
const router = Router();

router.get("/legacy/report", (req, res) => res.json({}));

module.exports = router;
