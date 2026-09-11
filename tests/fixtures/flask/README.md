# Flask fixture

A small but real Flask application exercising every registration form the Flask adapter
understands, plus the ones it deliberately does not. `expected-routes.json` is the pinned
result of scanning it; read `expected_misses` for the boundary.

Run it: `pip install flask && flask --app app run --port 5050` from this directory.
The `secret` token is `letmein`, sent as `Authorization: Bearer letmein`.
