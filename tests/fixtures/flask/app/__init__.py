"""Application factory — the layout most Flask projects grow into."""

from flask import Flask

from .api import admin, items, orphan, users  # noqa: F401  (orphan is deliberately unregistered)
from .api.views import NoteAPI
from .config import settings

API_V1 = "/api/v1"


def create_app() -> Flask:
    app = Flask(__name__)

    # A prefix given here *replaces* the blueprint's own `/users`, so this is where the
    # whole `/api/v1/users` comes from.
    app.register_blueprint(users.bp, url_prefix=API_V1 + "/users")
    # The same blueprint served under two versions, as a migration often leaves things.
    app.register_blueprint(items.bp, url_prefix="/v1", name="items_v1")
    app.register_blueprint(items.bp, url_prefix="/v2", name="items_v2")
    # A prefix nobody can know without running the config.
    app.register_blueprint(admin.bp, url_prefix=settings.ADMIN_PREFIX)

    note_view = NoteAPI.as_view("note")
    app.add_url_rule("/notes/<int:note_id>", view_func=note_view)
    app.add_url_rule("/notes", view_func=note_view, defaults={"note_id": None}, methods=["GET"])

    from .api import dynamic

    dynamic.register(app)

    @app.get("/health")
    def health():
        """Liveness probe."""
        return {"ok": True}

    return app
