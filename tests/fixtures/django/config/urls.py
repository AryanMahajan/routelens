"""The root URL conf — everything hangs off `ROOT_URLCONF`."""

from django.conf import settings
from django.urls import include, path, re_path
from rest_framework.routers import DefaultRouter

from api.views import TagViewSet, UserViewSet, health, legacy_ping

router = DefaultRouter()
router.register(r"users", UserViewSet, basename="user")
router.register("tags", TagViewSet, basename="tag")

urlpatterns = [
    path("health/", health, name="health"),
    path("api/", include("api.urls")),
    path("api/v1/", include(router.urls)),
    # An inline list — Django allows it, and people use it for small groups.
    path("legacy/", include([path("ping/", legacy_ping)])),
    # The old regex form, still common in long-lived projects.
    re_path(r"^reports/(?P<year>\d+)/$", health, name="report-year"),
    # A prefix nobody can know without running the config.
    path(settings.INTERNAL_PREFIX, include("api.internal_urls")),
]
