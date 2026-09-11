import type { NextApiRequest, NextApiResponse } from "next";

// Pages Router. The method is whatever the handler checks for.
export default function handler(req: NextApiRequest, res: NextApiResponse) {
  if (req.method !== "POST") {
    res.setHeader("Allow", "POST");
    return res.status(405).end();
  }
  const { name } = req.body;
  res.status(200).json({ greeting: `Hello, ${name}` });
}
