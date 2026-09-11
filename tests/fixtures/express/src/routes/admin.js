const express = require("express");

const router = express.Router();

router.get("/stats", (req, res) => res.json({ users: 0 }));

router.post("/reindex", (req, res) => res.sendStatus(202));

// A named export, destructured at the mount site.
module.exports = { router };
