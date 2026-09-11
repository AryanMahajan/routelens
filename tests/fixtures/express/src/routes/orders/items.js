const express = require("express");

// mergeParams so :orderId from the parent is visible here.
const router = express.Router({ mergeParams: true });

router.get("/", (req, res) => res.json({ order: req.params.orderId, items: [] }));

router.post("/", (req, res) => {
  const { sku, quantity } = req.body;
  res.status(201).json({ sku, quantity });
});

module.exports = router;
