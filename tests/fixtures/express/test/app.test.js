const express = require("express");

// Test files are skipped: this app and its routes must not appear.
const app = express();
app.get("/from-a-test", (req, res) => res.send("no"));

test("noop", () => {});
