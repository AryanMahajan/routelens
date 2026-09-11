const express = require("express");
const cors = require("cors");
const helmet = require("helmet");
const morgan = require("morgan");

const usersRouter = require("./routes/users");
const { router: adminRouter } = require("./routes/admin");
const { createV2Router } = require("./routes/v2");
const { requireAuth } = require("./middleware/auth");

const API = "/api";
const V1 = `${API}/v1`;

const app = express();

app.use(helmet());
app.use(cors());
app.use(morgan("dev"));
app.use(express.json());

app.get("/health", (req, res) => res.json({ ok: true }));

// A router mounted under a constant prefix, folded from two declarations above.
app.use(`${V1}/users`, usersRouter);

// Inline require, and a router file that mounts a second router inside it.
app.use(V1, require("./routes/orders"));

// The same router mounted twice: both paths are real.
app.use("/admin", requireAuth, adminRouter);
app.use("/internal/admin", adminRouter);

// A prefix that only exists at runtime.
app.use(process.env.API_PREFIX, require("./routes/items"));

// A router built by a call, which static analysis cannot follow.
app.use("/api/v2", createV2Router());

app.all("/ping", (req, res) => res.send("pong"));

module.exports = app;
