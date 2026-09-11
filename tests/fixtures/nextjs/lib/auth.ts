export function withAuth(handler: (req: any, res: any) => unknown) {
  return (req: any, res: any) => {
    if (!req.headers.authorization) return res.status(401).end();
    return handler(req, res);
  };
}
