exports.list = (req, res) => {
  const { limit = 20, offset = 0 } = req.query;
  res.json({ limit, offset });
};

exports.remove = (req, res) => res.sendStatus(204);
