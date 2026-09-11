import { withAuth } from "../../lib/auth";

async function handler(req, res) {
  if (req.method === "GET") return res.json({ user: req.user });
  res.status(405).end();
}

// Wrapped in a higher-order function: the wrapper's name is the only auth hint there is.
export default withAuth(handler);
