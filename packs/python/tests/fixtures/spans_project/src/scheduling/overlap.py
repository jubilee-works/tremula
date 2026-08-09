# 겹침 판정 — 두 구간이 실제로 만나는지 본다.


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    margin = 0  # 여유는 두지 않는다.
    return start < other_end - margin and other_start < end
