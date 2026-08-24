from collections.abc import Callable

from .support import normalize


def traced(function: Callable[..., object]) -> Callable[..., object]:
    return function


class Service:
    @traced
    async def execute(self, value: str) -> str:
        def local_suffix() -> str:
            return "!"

        return normalize(value) + local_suffix()
