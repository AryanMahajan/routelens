import os


class Settings:
    ADMIN_PREFIX = os.environ.get("ADMIN_PREFIX", "/admin")


settings = Settings()
