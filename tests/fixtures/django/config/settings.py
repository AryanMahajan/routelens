"""Deliberately minimal: no database, no admin, no sessions."""

import os

SECRET_KEY = "fixture-not-a-secret"
DEBUG = True
ALLOWED_HOSTS = ["*"]

INSTALLED_APPS = ["rest_framework", "api"]
MIDDLEWARE = ["django.middleware.common.CommonMiddleware"]

ROOT_URLCONF = "config.urls"

# Where the internal tools live is decided at deploy time.
INTERNAL_PREFIX = os.environ.get("INTERNAL_PREFIX", "internal/")

REST_FRAMEWORK = {
    "DEFAULT_RENDERER_CLASSES": ["rest_framework.renderers.JSONRenderer"],
    "DEFAULT_AUTHENTICATION_CLASSES": [],
    "UNAUTHENTICATED_USER": None,
}
