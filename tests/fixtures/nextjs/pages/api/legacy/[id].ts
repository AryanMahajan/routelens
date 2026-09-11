import type { NextApiRequest, NextApiResponse } from "next";

export default async function handler(req: NextApiRequest, res: NextApiResponse) {
  switch (req.method) {
    case "GET":
      return res.json({ id: req.query.id, expand: req.query.expand });
    case "PATCH":
      return res.json({ updated: req.body });
    default:
      res.setHeader("Allow", "GET, PATCH");
      return res.status(405).end();
  }
}
