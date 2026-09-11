"""A URL conf nothing includes. Its routes exist in source and are served by nothing."""

from django.urls import path

from api import views

urlpatterns = [
    path("legacy/forgotten/", views.health),
]
