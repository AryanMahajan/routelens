"""Settings loaded at import time — deliberately not statically resolvable."""

import os


class Settings:
    API_PREFIX = os.environ.get("API_PREFIX", "/admin")


settings = Settings()
