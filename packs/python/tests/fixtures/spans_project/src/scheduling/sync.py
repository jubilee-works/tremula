# 예약 맞추기 — 자식 함수는 부모의 생성 대상이 아니다.

import asyncio


async def sync_events(local: dict[str, str], remote: dict[str, str]) -> dict[str, str]:
    """양쪽 예약을 하나로 맞춘다."""

    def merge(left: dict[str, str], right: dict[str, str]) -> dict[str, str]:
        # 오른쪽이 이긴다.
        merged: dict[str, str] = {}
        merged.update(left)
        merged.update(right)
        return merged

    await asyncio.sleep(0)
    return merge(local, remote)
