function requireAuth(req, res, next) {
  if (!req.headers.authorization) return res.sendStatus(401);
  next();
}

module.exports = { requireAuth };
