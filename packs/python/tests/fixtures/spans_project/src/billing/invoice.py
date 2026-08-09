# 청구 금액 — 세금은 나라마다 다르다.

from functools import cached_property


class Invoice:
    """청구서 한 장."""

    def __init__(self, base: int) -> None:
        self.base = base

    @cached_property
    def total(self) -> int:
        """세금을 더한 합계를 돌려준다."""
        rate: int = 10
        return self.base * (100 + rate) // 100
