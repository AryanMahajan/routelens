const { Router } = require("express");
const { requireAuth } = require("../middleware/auth");
const users = require("../controllers/users");

const router = Router();

router.get("/", users.list);

router.post("/", (req, res) => {
  const { email, name } = req.body;
  res.status(201).json({ email, name });
});

router.get("/:id", requireAuth, (req, res) => {
  const trace = req.get("X-Trace-Id");
  res.json({ id: req.params.id, trace });
});

router.delete("/:id", requireAuth, users.remove);

router
  .route("/:id/avatar")
  .put(requireAuth, (req, res) => res.sendStatus(204))
  .delete(requireAuth, (req, res) => res.sendStatus(204));

module.exports = router;
