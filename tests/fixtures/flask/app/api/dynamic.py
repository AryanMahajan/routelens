"""Routes registered in a loop — invisible to static analysis, visible to runtime enrich."""

RESOURCES = ["widgets", "gadgets"]


def register(app):
    for name in RESOURCES:
        app.add_url_rule(
            f"/dyn/{name}",
            endpoint=f"dyn_{name}",
            view_func=lambda name=name: {"resource": name},
        )
