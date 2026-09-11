"""A bearer token check, the way small DRF services tend to write one."""

from rest_framework.permissions import BasePermission

TOKEN = "letmein"


class HasToken(BasePermission):
    def has_permission(self, request, view):
        return request.headers.get("Authorization") == f"Bearer {TOKEN}"
