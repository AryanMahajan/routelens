# Django fixture

A small but real Django + Django REST Framework project exercising every registration form
the Django adapter understands, plus the ones it deliberately does not. `expected-routes.json`
is the pinned result of scanning it; read `expected_misses` for the boundary.

No database and no models, so it runs without migrations:
`pip install -r requirements.txt && python manage.py runserver 8001`.
Guarded endpoints want `Authorization: Bearer letmein`.
