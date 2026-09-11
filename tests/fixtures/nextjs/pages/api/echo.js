// Never looks at req.method, so every method reaches it.
export default (req, res) => {
  res.json({ echo: req.query.message });
};
