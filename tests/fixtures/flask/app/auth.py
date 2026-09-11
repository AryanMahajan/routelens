"""A bearer token check, the way small Flask apps tend to write one."""

from functools import wraps

from flask import abort, request

TOKEN = "letmein"


def token_required(view):
    @wraps(view)
    def wrapped(*args, **kwargs):
        header = request.headers.get("Authorization", "")
        if header != f"Bearer {TOKEN}":
            abort(401)
        return view(*args, **kwargs)

    return wrapped
