from enum import Enum


class PageType(str, Enum):
    PAGE = "page"
    POST = "post"

    def __str__(self) -> str:
        return str(self.value)
