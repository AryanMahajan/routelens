const { Router } = require("express");
const itemsRouter = require("./items");

const router = Router();

router.get("/orders", (req, res) => {
  const status = req.query.status;
  res.json({ status });
});

router.get("/orders/:orderId", (req, res) => res.json({ id: req.params.orderId }));

// Nested mount: the items router's paths sit under /orders/:orderId.
router.use("/orders/:orderId/items", itemsRouter);

module.exports = router;
